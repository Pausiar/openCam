mod client;
mod decoder;
#[cfg(target_os = "linux")]
mod sink_linux;

use std::env;
use std::io;
use std::time::Duration;

use opencam_core::protocol::MsgType;

const QUEUE_CAPACITY: usize = 4;
const DEFAULT_CONFIG: &str =
    r#"{"codec":"h264","width":1920,"height":1080,"fps":30,"bitrate_kbps":8000,"lens":"wide"}"#;

fn main() -> io::Result<()> {
    // Uso: opencam-desktop <host:puerto> <dispositivo_v4l2>
    // Por USB: adb forward tcp:4747 tcp:4747, y host 127.0.0.1:4747.
    let args: Vec<String> = env::args().collect();
    let addr = args.get(1).map(String::as_str).unwrap_or("127.0.0.1:4747");
    let device = args.get(2).map(String::as_str).unwrap_or("/dev/video0");

    let mut client = client::Client::connect(addr, Duration::from_secs(5))?;
    let caps = client::handshake(&mut client, DEFAULT_CONFIG)?;
    println!("Conectado a {addr}. CAPS: {caps}");

    let mut decoder = decoder::VideoDecoder::new(QUEUE_CAPACITY)
        .map_err(|e| io::Error::other(e.to_string()))?;

    // PING cada segundo desde un hilo aparte, con el socket clonado: la lectura no se bloquea.
    // Usa el mismo origen de tiempo que `Client::now_us`, para que la latencia sea coherente.
    let mut pinger = client.try_clone_stream()?;
    let epoch = client.epoch();
    std::thread::spawn(move || {
        let mut seq: u32 = 0;
        loop {
            let header = opencam_core::Header {
                msg_type: MsgType::Ping.code(),
                flags: 0,
                seq,
                timestamp_us: epoch.elapsed().as_micros() as u64,
                payload_len: 0,
            };
            seq = seq.wrapping_add(1);
            if io::Write::write_all(&mut pinger, &header.encode()).is_err() {
                break;
            }
            std::thread::sleep(Duration::from_secs(1));
        }
    });

    #[cfg(target_os = "linux")]
    let mut camera = sink_linux::VirtualCamera::open(device, 1920, 1080)?;
    #[cfg(not(target_os = "linux"))]
    {
        let _ = device;
        return Err(io::Error::other("la cámara virtual de este sistema aún no está implementada"));
    }

    loop {
        let msg = client.recv()?;
        match MsgType::from_code(msg.msg_type) {
            Some(MsgType::Video) => {
                let keyframe = msg.flags & opencam_core::FLAG_KEYFRAME != 0;
                if decoder.push_access_unit(msg.payload, msg.timestamp_us, keyframe) {
                    client.send(MsgType::KeyframeRequest, 0, &[])?;
                }
                if let Some(frame) = decoder.next_rgb().map_err(|e| io::Error::other(e.to_string()))? {
                    let yuyv = rgb_to_yuyv(&frame.rgb, frame.width, frame.height);
                    #[cfg(target_os = "linux")]
                    camera.write_yuyv(&yuyv)?;
                    #[cfg(not(target_os = "linux"))]
                    let _ = yuyv;
                }
            }
            Some(MsgType::Pong) => {
                println!(
                    "latencia: {} ms, frames descartados: {}",
                    client::pong_latency_us(&client, &msg) / 1000,
                    decoder.dropped()
                );
            }
            _ => {}
        }
    }
}

/// Convierte RGB8 a YUYV 4:2:2 (dos píxeles comparten croma).
fn rgb_to_yuyv(rgb: &[u8], width: usize, height: usize) -> Vec<u8> {
    let mut out = vec![0u8; width * height * 2];
    for i in 0..(width * height / 2) {
        let (r0, g0, b0) = px(rgb, i * 2);
        let (r1, g1, b1) = px(rgb, i * 2 + 1);
        let y0 = luma(r0, g0, b0);
        let y1 = luma(r1, g1, b1);
        let (u, v) = chroma(r0 as i32 + r1 as i32, g0 as i32 + g1 as i32, b0 as i32 + b1 as i32);
        out[i * 4] = y0;
        out[i * 4 + 1] = u;
        out[i * 4 + 2] = y1;
        out[i * 4 + 3] = v;
    }
    out
}

fn px(rgb: &[u8], idx: usize) -> (u8, u8, u8) {
    (rgb[idx * 3], rgb[idx * 3 + 1], rgb[idx * 3 + 2])
}

fn luma(r: u8, g: u8, b: u8) -> u8 {
    (((66 * r as i32 + 129 * g as i32 + 25 * b as i32 + 128) >> 8) + 16).clamp(0, 255) as u8
}

/// Croma promediada sobre dos píxeles (sumas de R, G y B).
fn chroma(r: i32, g: i32, b: i32) -> (u8, u8) {
    let (r, g, b) = (r / 2, g / 2, b / 2);
    let u = ((-38 * r - 74 * g + 112 * b + 128) >> 8) + 128;
    let v = ((112 * r - 94 * g - 18 * b + 128) >> 8) + 128;
    (u.clamp(0, 255) as u8, v.clamp(0, 255) as u8)
}

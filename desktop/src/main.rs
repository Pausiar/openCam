mod cli;
mod client;
mod decoder;
mod session;
#[cfg(target_os = "linux")]
mod sink_linux;
#[cfg(windows)]
mod sink_windows;
#[cfg(unix)]
mod usbmuxd;

use std::env;
use std::io;

use cli::{Cli, CliError};
use opencam_core::protocol::MsgType;
#[cfg(any(target_os = "linux", windows))]
use session::End;

#[cfg(any(target_os = "linux", windows))]
const QUEUE_CAPACITY: usize = 4;

fn main() -> io::Result<()> {
    // Android por USB: adb forward tcp:4747 tcp:4747, y host 127.0.0.1:4747.
    // iPhone por USB (macOS/Linux con usbmuxd): usar el argumento `iphone`.
    let cli = match Cli::parse(env::args().skip(1)) {
        Ok(cli) => cli,
        Err(CliError::Help) => {
            print!("{}", cli::HELP);
            return Ok(());
        }
        Err(CliError::Invalid(msg)) => {
            eprintln!("{msg}\n\n{}", cli::HELP);
            std::process::exit(2);
        }
    };

    #[cfg(target_os = "linux")]
    let mut camera = sink_linux::VirtualCamera::open(&cli.device, cli.width, cli.height)?;
    #[cfg(windows)]
    let mut camera = sink_windows::VirtualCamera::open(cli.width, cli.height)?;
    #[cfg(not(any(target_os = "linux", windows)))]
    return Err(io::Error::other("la cámara virtual de este sistema aún no está implementada"));

    #[cfg(any(target_os = "linux", windows))]
    session::run(&cli, |client| serve(client, &mut camera))
}

/// Recibe vídeo de una conexión ya negociada hasta que se pierda o falle la cámara.
#[cfg(any(target_os = "linux", windows))]
fn serve(
    client: &mut client::Client,
    #[cfg(target_os = "linux")] camera: &mut sink_linux::VirtualCamera,
    #[cfg(windows)] camera: &mut sink_windows::VirtualCamera,
) -> End {
    // Decodificador nuevo por conexión: espera el primer keyframe de la sesión.
    let mut decoder = match decoder::VideoDecoder::new(QUEUE_CAPACITY) {
        Ok(d) => d,
        Err(e) => return End::Fatal(io::Error::other(e.to_string())),
    };
    loop {
        let msg = match client.recv() {
            Ok(m) => m,
            Err(e) => return End::Lost(e),
        };
        match MsgType::from_code(msg.msg_type) {
            Some(MsgType::Video) => {
                let keyframe = msg.flags & opencam_core::FLAG_KEYFRAME != 0;
                let mut need_keyframe = decoder.push_access_unit(msg.payload, msg.timestamp_us, keyframe);
                match decoder.next_rgb() {
                    Ok(Some(frame)) => {
                        let yuyv = rgb_to_yuyv(&frame.rgb, frame.width, frame.height);
                        if let Err(e) = camera.write_yuyv(&yuyv) {
                            return End::Fatal(e);
                        }
                    }
                    Ok(None) => {}
                    Err(e) => {
                        // Un frame corrupto no es fatal: se pide un keyframe y se sigue.
                        eprintln!("error de decodificación: {e}");
                        need_keyframe = true;
                    }
                }
                if need_keyframe {
                    if let Err(e) = client.send(MsgType::KeyframeRequest, 0, &[]) {
                        return End::Lost(e);
                    }
                }
            }
            Some(MsgType::Pong) => {
                println!(
                    "RTT: {} ms, frames descartados: {}",
                    client::pong_latency_us(client, &msg) / 1000,
                    decoder.dropped()
                );
            }
            Some(MsgType::Error) => {
                eprintln!("ERROR del móvil: {}", String::from_utf8_lossy(&msg.payload));
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

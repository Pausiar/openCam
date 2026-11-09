//! Bucle de sesión: conecta, negocia y sirve; si se pierde la conexión, reconecta con backoff.
//! Los mensajes de estado van a stderr: stdout puede ser el flujo Y4M (`--output -`).

use std::io;
use std::sync::mpsc::Receiver;
use std::sync::Mutex;
use std::thread::sleep;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use opencam_core::protocol::MsgType;
use opencam_core::{Backoff, BitrateConfig, BitrateController, Metrics};

use crate::cli::Cli;
use crate::client::{self, Client};
use crate::convert::mirror_yuyv;
use crate::decoder::VideoDecoder;
use crate::settings::{Command, LiveSettings};
use crate::sink::FrameSink;
use crate::transport;

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);
const QUEUE_CAPACITY: usize = 4;

/// Cómo termina una conexión servida.
pub enum End {
    /// Fin normal: no se reconecta (solo lo usan los tests; en producción la sesión no termina).
    #[cfg_attr(not(test), allow(dead_code))]
    Done,
    /// Conexión perdida: se reconecta salvo `--no-reconnect`.
    Lost(io::Error),
    /// Error local (p. ej. la cámara virtual): reconectar no lo arregla.
    Fatal(io::Error),
}

/// Repite conectar + handshake + `serve` hasta un fin normal o fatal (o un único intento con
/// `--no-reconnect`). El backoff se reinicia tras cada handshake correcto.
/// Cada handshake pide los ajustes en vivo actuales (lo cambiado en la consola se conserva).
pub fn run(
    cli: &Cli,
    live: &Mutex<LiveSettings>,
    mut serve: impl FnMut(&mut Client) -> End,
) -> io::Result<()> {
    let seed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(1, |d| d.as_nanos() as u64);
    let mut backoff = Backoff::new(500, 5000, 20, seed);
    loop {
        let config = cli.config_json(&live.lock().unwrap());
        let err = match transport::open(cli, &config, HANDSHAKE_TIMEOUT) {
            Ok(transport::Connected {
                mut client,
                caps,
                via,
            }) => {
                backoff.reset();
                eprintln!("Conectado por {via}. CAPS: {caps}");
                if caps.contains("h265") {
                    eprintln!("Aviso: el móvil anuncia h265; este escritorio solo decodifica H.264 y lo pide en CONFIG.");
                }
                client.spawn_pinger();
                let end = serve(&mut client);
                client.close();
                match end {
                    End::Done => return Ok(()),
                    End::Fatal(e) => return Err(e),
                    End::Lost(e) => e,
                }
            }
            Err(e) => e,
        };
        if !cli.reconnect {
            return Err(err);
        }
        let delay = backoff.next_delay_ms();
        eprintln!("Conexión perdida ({err}); reintentando en {delay} ms");
        sleep(Duration::from_millis(delay));
    }
}

/// NAL inicial de H.265 (VPS/SPS/PPS = tipos 32..=34): un móvil que ignora CONFIG y sigue en h265.
/// shortcut: solo mira el primer NAL tras el start code; basta porque el primer frame lleva parámetros.
fn looks_h265(au: &[u8]) -> bool {
    let i = au
        .windows(3)
        .position(|w| w == [0, 0, 1])
        .map_or(usize::MAX, |p| p + 3);
    au.get(i)
        .is_some_and(|b| (32..=34).contains(&((b >> 1) & 0x3f)) && b & 0x80 == 0 && b & 0x1f == 0)
}

/// Recibe vídeo de una conexión ya negociada hasta que se pierda o falle el sink.
/// Escala al tamaño del sink (letterbox) aunque el móvil mande otra resolución.
/// Las órdenes de `commands` (consola) actualizan `live` y se reenvían al móvil como CONFIG.
pub fn serve(
    client: &mut Client,
    sink: &mut dyn FrameSink,
    cli: &Cli,
    live: &Mutex<LiveSettings>,
    commands: Option<&Receiver<Command>>,
) -> End {
    // Decodificador nuevo por conexión: espera el primer keyframe de la sesión.
    let mut decoder = match VideoDecoder::new(QUEUE_CAPACITY) {
        Ok(d) => d,
        Err(e) => return End::Fatal(io::Error::other(e.to_string())),
    };
    let (w, h) = sink
        .fixed_size()
        .unwrap_or((cli.width as usize, cli.height as usize));
    let t0 = Instant::now();
    let mut metrics = Metrics::new(1000, 0);
    let mut current = live.lock().unwrap().clone();
    let mut bitrate = BitrateController::new(BitrateConfig::default(), current.bitrate_kbps);
    let (mut last_dropped, mut warned_h265) = (0u64, false);
    loop {
        // shortcut: las órdenes se atienden al llegar mensajes; con vídeo eso es cada frame.
        for cmd in commands.into_iter().flat_map(|rx| rx.try_iter()) {
            let send = current.apply(cmd);
            *live.lock().unwrap() = current.clone();
            if let Command::Bitrate(k) = cmd {
                bitrate = BitrateController::new(BitrateConfig::default(), k);
            }
            eprintln!("Ajuste aplicado: {cmd:?}");
            if send {
                let json = cli.config_json(&current);
                if let Err(e) = client.send(MsgType::Config, 0, json.as_bytes()) {
                    return End::Lost(e);
                }
            }
        }
        let msg = match client.recv() {
            Ok(m) => m,
            Err(e) => return End::Lost(e),
        };
        match MsgType::from_code(msg.msg_type) {
            Some(MsgType::Video) if looks_h265(&msg.payload) => {
                if !warned_h265 {
                    warned_h265 = true;
                    eprintln!("Aviso: el móvil envía H.265 pese a CONFIG h264; no se puede decodificar. Frames ignorados.");
                }
            }
            Some(MsgType::Video) => {
                let keyframe = msg.flags & opencam_core::FLAG_KEYFRAME != 0;
                let mut need_keyframe =
                    decoder.push_access_unit(msg.payload, msg.timestamp_us, keyframe);
                match decoder.next_yuyv(w, h) {
                    Ok(Some(mut yuyv)) => {
                        metrics.on_decoded();
                        if current.mirror {
                            mirror_yuyv(&mut yuyv, w);
                        }
                        if let Err(e) = sink.write_yuyv(&yuyv) {
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
                metrics.on_rtt((client::pong_latency_us(client, &msg) / 1000) as u32)
            }
            Some(MsgType::Error) => {
                eprintln!("ERROR del móvil: {}", String::from_utf8_lossy(&msg.payload))
            }
            _ => {}
        }
        // shortcut: STATS se evalúa al llegar mensajes; un móvil mudo ya cae por la conexión.
        let dropped = decoder.dropped();
        metrics.on_dropped((dropped - last_dropped) as u32);
        last_dropped = dropped;
        if let Some(snap) = metrics.poll(t0.elapsed().as_millis() as u64) {
            let target = bitrate.update(snap.rtt_ms, snap.dropped);
            if let Err(e) = client.send(MsgType::Stats, 0, snap.to_json(target).as_bytes()) {
                return End::Lost(e);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use opencam_core::protocol::{Header, MsgType, HEADER_LEN};
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};

    fn read_type(s: &mut TcpStream) -> u8 {
        let mut raw = [0u8; HEADER_LEN];
        s.read_exact(&mut raw).unwrap();
        let h = Header::decode(&raw).unwrap();
        s.read_exact(&mut vec![0u8; h.payload_len as usize])
            .unwrap();
        h.msg_type
    }

    fn write_type(s: &mut TcpStream, t: MsgType, p: &[u8]) {
        let h = Header {
            msg_type: t.code(),
            flags: 0,
            seq: 0,
            timestamp_us: 0,
            payload_len: p.len() as u32,
        };
        s.write_all(&h.encode()).unwrap();
        s.write_all(p).unwrap();
    }

    /// Móvil falso: dos conexiones con handshake completo; la primera se cierra a mitad de sesión.
    #[test]
    fn reconnects_and_repeats_handshake() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let cli = Cli {
            addr: listener.local_addr().unwrap().to_string(),
            ..Cli::default()
        };
        let phone = std::thread::spawn(move || {
            for _ in 0..2 {
                let (mut s, _) = listener.accept().unwrap();
                assert_eq!(read_type(&mut s), MsgType::Hello.code());
                write_type(&mut s, MsgType::Hello, b"{}");
                write_type(&mut s, MsgType::Caps, b"{}");
                assert_eq!(read_type(&mut s), MsgType::Config.code());
                // al terminar la iteración se cierra el socket
            }
        });
        let mut served = 0;
        run(&cli, &Mutex::new(cli.live()), |c| {
            served += 1;
            loop {
                match c.recv() {
                    Err(e) if served == 1 => return End::Lost(e),
                    Err(_) => return End::Done,
                    Ok(_) => {}
                }
            }
        })
        .unwrap();
        assert_eq!(served, 2);
        phone.join().unwrap();
    }

    #[test]
    fn no_reconnect_fails_once() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let cli = Cli {
            addr: listener.local_addr().unwrap().to_string(),
            reconnect: false,
            ..Cli::default()
        };
        drop(listener); // nadie escucha
        assert!(run(&cli, &Mutex::new(cli.live()), |_| End::Done).is_err());
    }

    #[derive(Clone, Default)]
    struct Shared(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

    impl Write for Shared {
        fn write(&mut self, b: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn looks_h265_detects_vps_only() {
        assert!(looks_h265(&[0, 0, 0, 1, 0x40, 1, 0xc]));
        assert!(!looks_h265(&[0, 0, 0, 1, 0x67, 0x42]));
        assert!(!looks_h265(&[1, 2, 3]));
    }

    /// Móvil falso: codifica 1280x720 con openh264; el cliente escala a 1920x1080 (Y4M) y envía STATS.
    #[test]
    fn end_to_end_scales_to_sink_and_sends_stats() {
        use openh264::encoder::Encoder;
        use openh264::formats::YUVBuffer;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let cli = Cli {
            addr: listener.local_addr().unwrap().to_string(),
            reconnect: false,
            sink: crate::cli::SinkKind::Y4m,
            ..Cli::default()
        };
        let phone = std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            assert_eq!(read_type(&mut s), MsgType::Hello.code());
            write_type(&mut s, MsgType::Hello, b"{}");
            write_type(&mut s, MsgType::Caps, br#"{"codecs":["h264","h265"]}"#);
            assert_eq!(read_type(&mut s), MsgType::Config.code());

            let mut rd = s.try_clone().unwrap();
            let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            let stop2 = stop.clone();
            let reader = std::thread::spawn(move || {
                // Lee hasta ver STATS y devuelve su JSON.
                loop {
                    let mut raw = [0u8; HEADER_LEN];
                    rd.read_exact(&mut raw).unwrap();
                    let h = Header::decode(&raw).unwrap();
                    let mut p = vec![0u8; h.payload_len as usize];
                    rd.read_exact(&mut p).unwrap();
                    if h.msg_type == MsgType::Stats.code() {
                        stop2.store(true, std::sync::atomic::Ordering::SeqCst);
                        return String::from_utf8(p).unwrap();
                    }
                }
            });

            let (w, h) = (1280usize, 720usize);
            let mut yuv = vec![120u8; w * h];
            yuv.extend(vec![90u8; w * h / 4]);
            yuv.extend(vec![200u8; w * h / 4]);
            let mut enc = Encoder::new().unwrap();
            let mut first = true;
            while !stop.load(std::sync::atomic::Ordering::SeqCst) {
                let au = enc
                    .encode(&YUVBuffer::from_vec(yuv.clone(), w, h))
                    .unwrap()
                    .to_vec();
                let flags = if first {
                    opencam_core::FLAG_KEYFRAME
                } else {
                    0
                };
                first = false;
                let hdr = Header {
                    msg_type: MsgType::Video.code(),
                    flags,
                    seq: 0,
                    timestamp_us: 0,
                    payload_len: au.len() as u32,
                };
                if s.write_all(&hdr.encode())
                    .and_then(|_| s.write_all(&au))
                    .is_err()
                {
                    break;
                }
                std::thread::sleep(Duration::from_millis(80));
            }
            let stats = reader.join().unwrap();
            let _ = s.shutdown(std::net::Shutdown::Both);
            stats
        });

        let out = Shared::default();
        let mut sink = crate::sink::Y4mSink::new(out.clone(), 1920, 1080, 30).unwrap();
        let live = Mutex::new(cli.live());
        let (tx, rx) = std::sync::mpsc::channel();
        tx.send(Command::Zoom(2.0)).unwrap();
        tx.send(Command::Mirror(true)).unwrap();
        assert!(run(&cli, &live, |c| serve(c, &mut sink, &cli, &live, Some(&rx))).is_err());
        assert_eq!(live.lock().unwrap().zoom, Some(2.0)); // el móvil cierra: Lost sin reconexión
        let stats = phone.join().unwrap();
        assert!(
            stats.contains(r#""target_bitrate_kbps":8000"#) && stats.contains("decoded_fps"),
            "{stats}"
        );

        let data = out.0.lock().unwrap();
        let hdr = b"YUV4MPEG2 W1920 H1080 F30:1 Ip A1:1 C422\n";
        assert!(data.starts_with(hdr));
        let frame = 6 + 1920 * 1080 * 2;
        let body = &data[hdr.len()..];
        assert!(
            body.len() >= 3 * frame && body.len() % frame == 0,
            "{} bytes",
            body.len()
        );
        // Plano Y: 16:9 sobre 16:9 llena el frame, incluida la esquina (sin barras).
        let y = &body[6..];
        assert!(y[540 * 1920 + 960].abs_diff(120) <= 4);
        assert!(y[0].abs_diff(120) <= 4);
    }
}

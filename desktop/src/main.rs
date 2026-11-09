mod adb;
mod cli;
mod client;
mod convert;
mod decoder;
mod discovery;
mod session;
mod sink;
#[cfg(target_os = "linux")]
mod sink_linux;
#[cfg(windows)]
mod sink_windows;
mod transport;
#[cfg(unix)]
mod usbmuxd;

use std::env;
use std::io;
use std::time::Duration;

use cli::{Cli, CliError, SinkKind};
use sink::FrameSink;

fn main() -> io::Result<()> {
    // Por defecto (`auto`) busca el móvil solo: USB (adb / usbmuxd) y luego Wi-Fi (mDNS).
    let mut cli = match Cli::parse(env::args().skip(1)) {
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

    if cli.list {
        return list();
    }
    if cli.discover {
        cli.addr = "wifi".into();
    }

    let mut sink = open_sink(&cli)?;
    session::run(&cli, |client| session::serve(client, &mut *sink, &cli))
}

fn list() -> io::Result<()> {
    match adb::find(None) {
        Some(path) => match adb::devices(&path) {
            Ok(devs) => devs
                .iter()
                .for_each(|d| println!("usb\tandroid\t{} ({})", d.serial, d.state)),
            Err(e) => eprintln!("adb: {e}"),
        },
        None => eprintln!("adb: no encontrado (Android por USB no disponible)"),
    }
    for f in discovery::browse(Duration::from_secs(3))? {
        println!("{}\t{}\t{}", f.addr, f.platform, f.name);
    }
    #[cfg(unix)]
    match usbmuxd::devices() {
        Ok(devs) => devs
            .iter()
            .for_each(|d| println!("iphone\tios\t{}", d.udid)),
        Err(e) => eprintln!("usbmuxd: {e}"),
    }
    Ok(())
}

fn open_sink(cli: &Cli) -> io::Result<Box<dyn FrameSink>> {
    match cli.sink {
        SinkKind::Y4m => Ok(Box::new(sink::Y4mSink::open(
            &cli.output,
            cli.width as usize,
            cli.height as usize,
            cli.fps,
        )?)),
        #[cfg(target_os = "linux")]
        SinkKind::V4l2 => Ok(Box::new(sink_linux::VirtualCamera::open(
            &cli.device,
            cli.width,
            cli.height,
        )?)),
        #[cfg(windows)]
        SinkKind::Windows => Ok(Box::new(sink_windows::VirtualCamera::open(
            cli.width, cli.height,
        )?)),
        #[allow(unreachable_patterns)]
        _ => Err(io::Error::other(
            "ese sink no está disponible en este sistema operativo",
        )),
    }
}

// En Windows la app es de ventana (sin consola negra); en modo CLI se engancha a la consola padre.
#![cfg_attr(windows, windows_subsystem = "windows")]

mod adb;
mod cli;
mod client;
mod convert;
mod decoder;
mod discovery;
mod gui;
mod session;
mod settings;
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

/// La app de Windows no tiene consola propia; en modo CLI usa la de quien la lanzó.
#[cfg(windows)]
fn attach_console() {
    use windows::Win32::System::Console::{AttachConsole, ATTACH_PARENT_PROCESS};
    // SAFETY: llamada Win32 sin punteros; falla sin efecto si no hay consola padre.
    let _ = unsafe { AttachConsole(ATTACH_PARENT_PROCESS) };
}

fn main() -> io::Result<()> {
    // Sin argumentos se abre la ventana. Con argumentos, modo línea de órdenes:
    // Por defecto (`auto`) busca el móvil solo: USB (adb / usbmuxd) y luego Wi-Fi (mDNS).
    let args: Vec<String> = env::args().skip(1).collect();
    if args.is_empty() {
        return gui::run();
    }
    #[cfg(windows)]
    attach_console();
    let mut cli = match Cli::parse(args) {
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

    if cli.gui {
        return gui::run();
    }
    if cli.list {
        return list();
    }
    if cli.discover {
        cli.addr = "wifi".into();
    }

    let mut sink = open_sink(&cli)?;
    let obs = session::Stderr;
    session::run(&cli, &obs, |client| {
        session::serve(client, &mut *sink, &cli, &obs)
    })
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

pub fn open_sink(cli: &Cli) -> io::Result<Box<dyn FrameSink>> {
    match cli.sink {
        SinkKind::None => Ok(Box::new(sink::NullSink)),
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

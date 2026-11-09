mod cli;
mod client;
mod convert;
mod decoder;
mod discovery;
mod session;
mod settings;
mod sink;
#[cfg(target_os = "linux")]
mod sink_linux;
#[cfg(windows)]
mod sink_windows;
#[cfg(unix)]
mod usbmuxd;

use std::env;
use std::io::{self, BufRead};
use std::sync::mpsc::{self, Receiver};
use std::sync::Mutex;
use std::time::Duration;

use cli::{Cli, CliError, SinkKind};
use settings::Command;
use sink::FrameSink;

fn main() -> io::Result<()> {
    // Android por USB: adb forward tcp:4747 tcp:4747, y host 127.0.0.1:4747.
    // iPhone por USB (macOS/Linux con usbmuxd): usar el argumento `iphone`.
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
        let found = discovery::browse(Duration::from_secs(3))?;
        let first = found.first().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "no se halló ningún teléfono por mDNS",
            )
        })?;
        eprintln!(
            "Descubierto: {} ({}) en {}",
            first.name, first.platform, first.addr
        );
        cli.addr = first.addr.clone();
    }

    let mut sink = open_sink(&cli)?;
    let live = Mutex::new(cli.live());
    let commands = cli.controls.then(spawn_console);
    session::run(&cli, &live, |client| {
        session::serve(client, &mut *sink, &cli, &live, commands.as_ref())
    })
}

/// Lee órdenes en vivo (zoom, enfoque...) de stdin. stdout puede ser el Y4M; stdin queda libre.
fn spawn_console() -> Receiver<Command> {
    let (tx, rx) = mpsc::channel();
    eprintln!("Escribe `ayuda` y pulsa Intro para ver los ajustes en vivo.");
    std::thread::spawn(move || {
        for line in io::stdin().lock().lines() {
            let Ok(line) = line else { break };
            match Command::parse(&line) {
                Ok(Some(cmd)) => {
                    if tx.send(cmd).is_err() {
                        break;
                    }
                }
                Ok(None) => {}
                Err(msg) => eprintln!("{msg}"),
            }
        }
    });
    rx
}

fn list() -> io::Result<()> {
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

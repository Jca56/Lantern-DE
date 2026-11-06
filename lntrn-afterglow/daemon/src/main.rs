//! lntrn-afterglowd: Afterglow's root helper, the one part that changes
//! the hardware. A boot service; it sets nothing at start, and puts
//! everything back to stock when it stops.
//!
//! ```text
//! lntrn-afterglowd              serve on /run/lntrn-afterglowd.sock (as root)
//! lntrn-afterglowd --probe      say what can be tuned here, and stop
//! lntrn-afterglowd --dry-run    serve, saying what would be set but setting nothing
//! lntrn-afterglowd --socket P   listen at P instead: a test as a user
//! ```

mod serve;

use std::os::unix::fs::{FileTypeExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use lntrn_afterglowd::tune::{Fan, Limits, Tune};
use lntrn_afterglowd::{GROUP, SOCKET};

use serve::log::say;
use serve::socket::{self, Door};
use serve::state::Daemon;
use serve::ffi;

/// Where the stock tune is kept through restarts within one boot.
const STATE_DIR: &str = "/run/lntrn-afterglowd";

struct Args {
    probe: bool,
    dry_run: bool,
    socket: PathBuf,
}

fn args() -> Result<Args, String> {
    let mut a = Args { probe: false, dry_run: false, socket: PathBuf::from(SOCKET) };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--probe" => a.probe = true,
            "--dry-run" => a.dry_run = true,
            "--socket" => a.socket = it.next().map(PathBuf::from).ok_or("--socket needs a path")?,
            other => return Err(format!("unknown argument {other}")),
        }
    }
    Ok(a)
}

fn main() {
    let args = args().unwrap_or_else(|e| {
        eprintln!("lntrn-afterglowd: {e}");
        std::process::exit(2);
    });
    // Before any thread starts, so only the signal thread sees signals.
    let signals = ffi::block_signals();
    let root = ffi::euid() == 0;
    if args.probe {
        return probe(&Daemon::start(None, true));
    }
    if !root && args.socket == Path::new(SOCKET) {
        eprintln!("lntrn-afterglowd: run it as root (it's a service), or give --socket for a test");
        std::process::exit(1);
    }
    let daemon = Daemon::start(root.then_some(Path::new(STATE_DIR)), args.dry_run);
    let gid = socket::group(GROUP).map(|(gid, _)| gid);
    let listener = bind(&args.socket, root, gid).unwrap_or_else(|e| {
        say(&e);
        std::process::exit(1);
    });
    say(&format!("listening on {}{}{}", args.socket.display(), if root { "" } else { " as a user: nothing will take" }, if args.dry_run { ", dry run" } else { "" }));
    let daemon = Arc::new(Mutex::new(daemon));

    let ticking = daemon.clone();
    std::thread::Builder::new()
        .name("tick".into())
        .spawn(move || {
            loop {
                std::thread::sleep(Duration::from_secs(1));
                let mut d = ticking.lock().unwrap_or_else(|e| e.into_inner());
                if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| d.tick())).is_err() {
                    say("a tick went wrong: the fans go back to the card");
                    d.fans_to_card();
                }
            }
        })
        .expect("the ticking thread");

    let stopping = daemon.clone();
    let path = args.socket.clone();
    std::thread::Builder::new()
        .name("signals".into())
        .spawn(move || {
            let signal = ffi::wait(&signals);
            say(&format!("signal {signal}: stopping"));
            stopping.lock().unwrap_or_else(|e| e.into_inner()).shut_down();
            let _ = std::fs::remove_file(&path);
            std::process::exit(0);
        })
        .expect("the signal thread");

    let door = Door { own: ffi::euid(), group: gid.map(|_| GROUP.to_owned()) };
    socket::serve(listener, daemon, door);
}

/// Listen at `path`: born `0600` so no one slips in first, then handed to
/// the group when running as root.
fn bind(path: &Path, root: bool, gid: Option<u32>) -> Result<UnixListener, String> {
    if UnixStream::connect(path).is_ok() {
        return Err(format!("another helper already answers at {}", path.display()));
    }
    // A socket left by a helper that died; never anything else.
    if std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_socket()) {
        let _ = std::fs::remove_file(path);
    }
    ffi::set_umask(0o177);
    let listener = UnixListener::bind(path).map_err(|e| format!("can't listen at {}: {e}", path.display()));
    ffi::set_umask(0o022);
    let listener = listener?;
    if root {
        match gid {
            Some(gid) => {
                std::os::unix::fs::chown(path, Some(0), Some(gid)).map_err(|e| format!("can't give the socket to {GROUP}: {e}"))?;
                std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o660)).map_err(|e| e.to_string())?;
            }
            None => say(&format!("there is no {GROUP} group: only root can use the helper")),
        }
    }
    Ok(listener)
}

fn probe(d: &Daemon) {
    let Limits { gpu, cpu, stock } = &d.limits;
    println!("Afterglow can tune:");
    match gpu {
        Some(g) => {
            println!("  {} (driver {})", g.name, g.driver);
            println!("    core offset {:+} to {:+} MHz, memory offset {:+} to {:+} MHz", g.core_offset.0, g.core_offset.1, g.memory_offset.0, g.memory_offset.1);
            println!("    power {} to {} W (stock {} W)", g.power.0, g.power.1, g.power_default);
            match g.lock {
                Some((lo, hi)) => println!("    core clock lock {lo} to {hi} MHz"),
                None => println!("    no core clock lock"),
            }
            match g.fan_range {
                Some((lo, hi)) => println!("    {} fans, {lo} to {hi}%; slows itself at {} °C", g.fans, g.slowdown),
                None => println!("    fans can't be set"),
            }
        }
        None => println!("  no NVIDIA card"),
    }
    println!("  {}", cpu.model);
    println!("    governors {:?}, energy preferences {:?}, turbo {}", cpu.governors, cpu.epps, if cpu.turbo { "switchable" } else { "fixed" });
    println!("    fastest P-core {} MHz, E-core {} MHz", cpu.pcore_mhz, cpu.ecore_mhz);
    match cpu.power {
        Some((lo, hi)) => println!("    package power limits {lo} to {hi} W"),
        None => println!("    no package power limits"),
    }
    println!("Stock: {}", describe(stock));
}

fn describe(t: &Tune) -> String {
    let c = &t.cpu;
    let cpu = format!("{} / {} / turbo {} / top {}% / PL1 {} W PL2 {} W", c.governor, if c.epp.is_empty() { "-" } else { &c.epp }, c.turbo, c.max_percent, c.pl1, c.pl2);
    match &t.gpu {
        Some(g) => {
            let fan = match &g.fan {
                Fan::Auto => "auto".to_owned(),
                Fan::Fixed(p) => format!("{p}%"),
                Fan::Curve(p) => format!("curve of {}", p.len()),
            };
            format!("core {:+} mem {:+} MHz, {} W, lock {:?}, fans {fan}; {cpu}", g.core_offset, g.memory_offset, g.power_limit, g.lock)
        }
        None => cpu,
    }
}

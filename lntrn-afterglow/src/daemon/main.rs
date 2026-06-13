//! lntrn-afterglowd — root helper daemon for Afterglow.
//!
//! Owns every privileged write (NVML setters, RAPL limits, cpufreq knobs)
//! behind a peer-cred-checked Unix socket so the GUI never needs root.
//!
//!   lntrn-afterglowd --probe              hardware discovery report, no socket
//!   lntrn-afterglowd --config <path>      serve; re-apply profile iff stable=1

mod server;

use std::io::{BufRead, BufReader, Write};
use std::os::fd::AsRawFd;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;

use lntrn_afterglow::{profile, SOCKET_PATH};

fn main() {
    let mut probe = false;
    let mut config: Option<PathBuf> = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--probe" => probe = true,
            "--config" => config = args.next().map(PathBuf::from),
            other => {
                eprintln!("afterglowd: unknown arg {other}");
                std::process::exit(2);
            }
        }
    }

    let mut hw = server::Hw::init();

    if probe {
        print_probe(&mut hw);
        return;
    }

    if unsafe { libc::geteuid() } != 0 {
        eprintln!("afterglowd: not running as root — setters will fail, serving read-only");
    }

    if let Some(cfg) = &config {
        match profile::load(cfg) {
            Some(p) if p.stable => {
                eprintln!("afterglowd: applying stable profile from {}", cfg.display());
                for e in hw.apply_profile(&p) {
                    eprintln!("afterglowd: profile apply: {e}");
                }
            }
            Some(_) => eprintln!("afterglowd: profile present but not marked stable — booting stock"),
            None => eprintln!("afterglowd: no profile yet"),
        }
    }

    // Stale-socket cleanup: if something answers, another daemon owns it.
    if UnixStream::connect(SOCKET_PATH).is_ok() {
        eprintln!("afterglowd: already running");
        return;
    }
    let _ = std::fs::remove_file(SOCKET_PATH);

    let listener = match UnixListener::bind(SOCKET_PATH) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("afterglowd: bind {SOCKET_PATH}: {e}");
            std::process::exit(1);
        }
    };

    // Hand the socket to the session user (sudo caller); keep it 0600+owner.
    let owner_uid = std::env::var("SUDO_UID")
        .ok()
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or_else(|| unsafe { libc::getuid() });
    let owner_gid = std::env::var("SUDO_GID")
        .ok()
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(owner_uid);
    if let Ok(cpath) = std::ffi::CString::new(SOCKET_PATH) {
        unsafe {
            libc::chown(cpath.as_ptr(), owner_uid, owner_gid);
            libc::chmod(cpath.as_ptr(), 0o600);
        }
    }

    eprintln!("afterglowd: listening on {SOCKET_PATH} (owner uid {owner_uid})");
    for stream in listener.incoming() {
        let Ok(stream) = stream else { continue };
        match peer_uid(&stream) {
            Some(uid) if uid == 0 || uid == owner_uid => {}
            other => {
                eprintln!("afterglowd: rejected connection from uid {other:?}");
                continue;
            }
        }
        serve_client(stream, &mut hw);
    }
}

fn serve_client(stream: UnixStream, hw: &mut server::Hw) {
    let Ok(mut writer) = stream.try_clone() else {
        return;
    };
    let reader = BufReader::new(stream);
    for line in reader.lines() {
        let Ok(line) = line else { break };
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let response = hw.handle(line);
        if writeln!(writer, "{response}").is_err() {
            break;
        }
    }
}

fn peer_uid(stream: &UnixStream) -> Option<u32> {
    let mut cred: libc::ucred = unsafe { std::mem::zeroed() };
    let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    let ret = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            (&mut cred as *mut libc::ucred).cast(),
            &mut len,
        )
    };
    (ret == 0).then_some(cred.uid)
}

fn print_probe(hw: &mut server::Hw) {
    println!("== lntrn-afterglow hardware probe ==");
    println!("euid: {}", unsafe { libc::geteuid() });
    match &hw.nvml {
        Some(n) => match n.info() {
            Ok(i) => {
                println!("GPU: {} (driver {})", i.name, i.driver);
                println!(
                    "  power limit: {}W (range {}-{}W, default {}W)",
                    i.power_default_w, i.power_min_w, i.power_max_w, i.power_default_w
                );
                println!(
                    "  core offset: {} MHz (range {}..{})",
                    i.core_offset.current, i.core_offset.min, i.core_offset.max
                );
                println!(
                    "  mem offset:  {} MHz (range {}..{})",
                    i.mem_offset.current, i.mem_offset.min, i.mem_offset.max
                );
                println!("  fans: {}", i.fan_count);
                match n.snapshot() {
                    Ok(s) => println!(
                        "  now: {} MHz core / {} MHz mem, {}°C, {:.1}W, {}% util, fans {:?}%",
                        s.core_mhz, s.mem_mhz, s.temp_c, s.power_w, s.util_pct, s.fans_pct
                    ),
                    Err(e) => println!("  snapshot failed: {e}"),
                }
            }
            Err(e) => println!("GPU: info failed: {e}"),
        },
        None => println!("GPU: no NVIDIA driver on this machine"),
    }
    match &hw.rapl {
        Some(r) => {
            let l = r.limits();
            println!(
                "RAPL: PL1={:?}W PL2={:?}W (max {:?}/{:?}W)",
                l.pl1_w, l.pl2_w, l.pl1_max_w, l.pl2_max_w
            );
        }
        None => println!("RAPL: not available"),
    }
    println!("CPU: {}", hw.cpu.model);
    let t = hw.cpu.tuning();
    println!(
        "  governor: {} (available: {:?})",
        t.governor, t.governors
    );
    println!("  EPP: {:?} (available: {:?})", t.epp, t.epps);
    println!(
        "  turbo: {:?}, max_freq: {}%",
        t.no_turbo.map(|nt| !nt),
        t.max_freq_pct
    );
    let snap = hw.cpu.snapshot();
    let pcores = snap.cores.iter().filter(|c| c.is_pcore).count();
    println!(
        "  {} logical cpus ({} P-thread, {} E-core)",
        snap.cores.len(),
        pcores,
        snap.cores.len() - pcores
    );
}

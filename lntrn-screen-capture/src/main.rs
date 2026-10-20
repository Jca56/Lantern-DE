//! Lantern's screen recorder. Super+Print brings up the picker on every
//! monitor; the area chosen there is recorded, H.264 on the GPU's own
//! video engine (`lntrn-video`) into an MP4 in `~/Videos`, under a badge
//! with Stop. Super+Print again stops it too. Once it's saved, a
//! notification offers to open it. On Lantern UI 2.
//!
//! ## Layout
//!
//! - `picker/` — choosing what to record: an area dragged out, a window,
//!   or a whole screen, on any monitor.
//! - `capture/` — the screen, through wlr-screencopy.
//! - `record.rs` — the recording's thread: frames to the encoder, the file.
//! - `badge.rs` — the badge while it records.
//! - `ipc.rs` — the socket that lets one hotkey start and stop.
//! - `control.rs` — stop, and waking the UI, across threads.
//! - `notify.rs` — the notification at the end; `files.rs` — names, sizes.
//! - `look.rs` — the desktop's look, and how the surfaces are made.

mod badge;
mod capture;
mod control;
mod files;
mod ipc;
mod look;
mod notify;
mod picker;
mod record;

use std::path::PathBuf;

use lntrn_core::log_info;
use lntrn_kit::desktop::Desktop;

use crate::record::{Job, Status};

const USAGE: &str = "Usage: lntrn-screen-capture [OPTIONS]

Pick an area of the screen and record it. Run again while it records to stop.

  -o, --output <PATH>     Where the video goes (default: ~/Videos/recording_<time>.mp4)
  -r, --framerate <FPS>   Frames a second at most (default: 60)
  -h, --help              Show this";

struct Args {
    path: Option<PathBuf>,
    fps: u32,
}

fn parse_args() -> Result<Args, String> {
    let mut args = Args { path: None, fps: 60 };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "-o" | "--output" => args.path = Some(it.next().ok_or("--output needs a path")?.into()),
            "-r" | "--framerate" => args.fps = it.next().and_then(|v| v.parse().ok()).filter(|f| (1..=240).contains(f)).ok_or("--framerate needs a number from 1 to 240")?,
            "-h" | "--help" => {
                println!("{USAGE}");
                std::process::exit(0);
            }
            other => return Err(format!("unknown option {other}")),
        }
    }
    Ok(args)
}

fn main() {
    lntrn_kit::startup::log_panics(look::APP_ID);
    let args = parse_args().unwrap_or_else(|e| {
        eprintln!("lntrn-screen-capture: {e}\n\n{USAGE}");
        std::process::exit(2);
    });
    if ipc::tell_running() {
        log_info!("screen-capture: told the running one to stop");
        return;
    }
    control::catch_signals();
    let listening = ipc::listen();
    let monitors = match capture::monitors() {
        Ok(m) if !m.is_empty() => m,
        Ok(_) => return notify::failed("There is no monitor to record."),
        Err(e) => return notify::failed(&e),
    };
    let desktop = Desktop::read();
    let Some(choice) = picker::pick(&monitors, &desktop) else {
        log_info!("screen-capture: cancelled");
        return;
    };
    let status = Status::new();
    let job = Job { output: choice.output.clone(), area: choice.area, path: args.path.unwrap_or_else(files::default_path), fps: args.fps };
    let recording = record::start(job, status.clone());
    // Until the file is saved. Without a badge (the compositor took it),
    // the recording goes on until Super+Print stops it.
    badge::show(&choice.output, &desktop, status.clone());
    let outcome = recording.join().unwrap_or_else(|_| Err("the recording crashed".into()));
    // Free for the next recording before the notification waits.
    drop(listening);
    match outcome {
        Ok(saved) => notify::saved(&saved),
        // Stopped before the first frame: nothing to tell.
        Err(e) if control::stopped() && !status.began() => log_info!("screen-capture: {e}"),
        Err(e) => notify::failed(&e),
    }
}

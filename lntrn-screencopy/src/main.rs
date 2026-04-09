mod capture;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

pub static STOP: AtomicBool = AtomicBool::new(false);

fn main() {
    install_signal_handler();

    let (output_path, framerate) = parse_args();

    // Ensure output directory exists
    if let Some(dir) = output_path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }

    eprintln!("Recording to: {}", output_path.display());
    eprintln!("Framerate: {framerate} fps");
    eprintln!("Press Ctrl+C to stop recording.\n");

    let start = std::time::Instant::now();

    match capture::record_screen(&output_path, framerate, &STOP) {
        Ok(frame_count) => {
            let duration = start.elapsed();
            let secs = duration.as_secs_f64();
            eprintln!("\nRecording finished:");
            eprintln!("  Frames: {frame_count}");
            eprintln!("  Duration: {secs:.1}s");
            if secs > 0.0 {
                eprintln!("  Avg FPS: {:.1}", frame_count as f64 / secs);
            }
            if let Ok(meta) = std::fs::metadata(&output_path) {
                let mb = meta.len() as f64 / (1024.0 * 1024.0);
                eprintln!("  File size: {mb:.1} MB");
            }
            eprintln!("  Output: {}", output_path.display());
        }
        Err(e) => {
            eprintln!("Error: {e}");
            std::process::exit(1);
        }
    }
}

fn parse_args() -> (PathBuf, u32) {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut output: Option<PathBuf> = None;
    let mut framerate: u32 = 60;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-o" | "--output" => {
                if i + 1 < args.len() {
                    output = Some(PathBuf::from(&args[i + 1]));
                    i += 2;
                } else {
                    eprintln!("Error: --output requires a path");
                    std::process::exit(1);
                }
            }
            "-r" | "--framerate" => {
                if i + 1 < args.len() {
                    framerate = args[i + 1].parse().unwrap_or_else(|_| {
                        eprintln!("Error: --framerate requires a number");
                        std::process::exit(1);
                    });
                    i += 2;
                } else {
                    eprintln!("Error: --framerate requires a number");
                    std::process::exit(1);
                }
            }
            "-h" | "--help" => {
                eprintln!("Usage: lntrn-screencopy [OPTIONS]");
                eprintln!();
                eprintln!("Options:");
                eprintln!("  -o, --output <PATH>    Output file (default: ~/Videos/recording_*.mp4)");
                eprintln!("  -r, --framerate <FPS>  Target framerate (default: 60)");
                eprintln!("  -h, --help             Show this help");
                std::process::exit(0);
            }
            _ => {
                eprintln!("Unknown argument: {}", args[i]);
                std::process::exit(1);
            }
        }
    }

    let output = output.unwrap_or_else(default_output_path);
    (output, framerate)
}

fn default_output_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    let dir = PathBuf::from(home).join("Videos");
    let ts = timestamp();
    dir.join(format!("recording_{ts}.mp4"))
}

fn timestamp() -> String {
    unsafe {
        let mut t: libc::time_t = 0;
        libc::time(&mut t);
        let tm = libc::localtime(&t);
        if tm.is_null() {
            return format!("{t}");
        }
        let tm = &*tm;
        format!(
            "{:04}-{:02}-{:02}_{:02}-{:02}-{:02}",
            tm.tm_year + 1900,
            tm.tm_mon + 1,
            tm.tm_mday,
            tm.tm_hour,
            tm.tm_min,
            tm.tm_sec,
        )
    }
}

fn install_signal_handler() {
    unsafe {
        let mut sa: libc::sigaction = std::mem::zeroed();
        sa.sa_sigaction = handle_signal as *const () as usize;
        sa.sa_flags = 0;
        libc::sigaction(libc::SIGINT, &sa, std::ptr::null_mut());
        libc::sigaction(libc::SIGTERM, &sa, std::ptr::null_mut());
    }
}

extern "C" fn handle_signal(_sig: libc::c_int) {
    STOP.store(true, Ordering::SeqCst);
}

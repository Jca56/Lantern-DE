//! Desktop sound comes from a helper: this same program, run again with
//! one flag and one environment variable. PipeWire's ALSA plugin gives a
//! capture stream the default sink's monitor (what the speakers play)
//! only when the stream says `stream.capture.sink`, and the one way to
//! say it is `PIPEWIRE_ALSA` in the environment, read as the device
//! opens: process-wide, so the microphone would get it too, and setting
//! it while other threads run is unsound. A child has its own.
//!
//! The helper writes each block it captures to its stdout: the moment it
//! came (nanoseconds since the epoch, u64), its frames (u32), then the
//! samples (f32), all little-endian. It stops when its stdin closes.

use std::io::{Read, Write};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use lntrn_audio::{Input, Spec};
use lntrn_core::log_error;

use super::feed::{CHANNELS, Feed, RATE};

/// The flag that makes this program the helper.
pub const FLAG: &str = "--desktop-audio-helper";
const PROPS: &str = "{ stream.capture.sink = true }";
/// Frames in one block, at most: a sanity check on what comes in.
const BIGGEST: usize = RATE as usize;

/// The helper's side: capture the default sink's monitor to stdout
/// until stdin closes.
pub fn serve() -> ! {
    let mut bytes = Vec::new();
    let send = move |block: &[f32]| {
        let at = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap_or_default();
        bytes.clear();
        bytes.extend_from_slice(&(at.as_nanos() as u64).to_le_bytes());
        bytes.extend_from_slice(&((block.len() / CHANNELS) as u32).to_le_bytes());
        for s in block {
            bytes.extend_from_slice(&s.to_le_bytes());
        }
        let mut out = std::io::stdout().lock();
        // The recorder gone: nothing to send to.
        if out.write_all(&bytes).and_then(|()| out.flush()).is_err() {
            std::process::exit(0);
        }
    };
    let spec = Spec::new(RATE, CHANNELS as u16);
    let input = match Input::open_device("pipewire", spec, send) {
        Ok(input) => input,
        Err(e) => {
            log_error!("screen-capture: no desktop sound: {e}");
            std::process::exit(1);
        }
    };
    let mut buf = [0u8; 64];
    while matches!(std::io::stdin().read(&mut buf), Ok(n) if n > 0) {}
    drop(input);
    std::process::exit(0);
}

/// The helper, running: dropping it stops it.
pub struct Helper {
    child: Child,
}

impl Drop for Helper {
    fn drop(&mut self) {
        drop(self.child.stdin.take());
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Start the helper, its blocks going into `feed`.
pub fn start(feed: Arc<Feed>) -> Result<Helper, String> {
    let exe = std::env::current_exe().map_err(|e| format!("can't find this program to run it again: {e}"))?;
    let mut child = Command::new(exe)
        .arg(FLAG)
        .env("PIPEWIRE_ALSA", PROPS)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|e| format!("can't start the desktop audio helper: {e}"))?;
    let stdout = child.stdout.take().ok_or("the desktop audio helper has no output")?;
    std::thread::Builder::new()
        .name("capture-desktop-audio".into())
        .spawn(move || read(stdout, &feed))
        .map_err(|e| format!("can't read the desktop audio helper: {e}"))?;
    Ok(Helper { child })
}

/// Blocks from the helper into `feed`, until it stops.
fn read(mut from: impl Read, feed: &Feed) {
    let mut head = [0u8; 12];
    let mut bytes = Vec::new();
    let mut block = Vec::new();
    loop {
        if from.read_exact(&mut head).is_err() {
            break;
        }
        let at = Duration::from_nanos(u64::from_le_bytes(head[..8].try_into().unwrap()));
        let frames = u32::from_le_bytes(head[8..].try_into().unwrap()) as usize;
        if frames > BIGGEST {
            break;
        }
        bytes.resize(frames * CHANNELS * 4, 0);
        if from.read_exact(&mut bytes).is_err() {
            break;
        }
        block.clear();
        block.extend(bytes.as_chunks::<4>().0.iter().map(|b| f32::from_le_bytes(*b)));
        feed.push(&block, at);
    }
    feed.end("the desktop audio helper stopped");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_from_the_helper_reach_the_feed() {
        let mut wire = Vec::new();
        for (at, value) in [(1_000_000_000u64, 0.25f32), (1_020_000_000, 0.5)] {
            wire.extend_from_slice(&at.to_le_bytes());
            wire.extend_from_slice(&960u32.to_le_bytes());
            for _ in 0..960 * CHANNELS {
                wire.extend_from_slice(&value.to_le_bytes());
            }
        }
        let feed = Feed::new("desktop");
        read(&wire[..], &feed);
        assert_eq!(feed.report(), (1920, Some("the desktop audio helper stopped".into())));
        feed.start(Duration::from_secs(1) - Duration::from_millis(20));
        let mut out = vec![0.0; 1920 * CHANNELS];
        feed.mix_into(0, 1920, &mut out);
        assert!(out[..960 * CHANNELS].iter().all(|&s| s == 0.25) && out[960 * CHANNELS..].iter().all(|&s| s == 0.5));
    }
}

//! FFmpeg video decoder running on a dedicated thread.
//!
//! Opens a video file via `video-rs`, decodes frames to RGBA, and sends them
//! through a bounded channel to the main thread. Responds to play/pause/seek
//! commands via a separate command channel.

use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};
use crossbeam_channel::{bounded, Receiver, Sender, TryRecvError};
use video_rs::decode::Decoder;

/// A decoded video frame ready for GPU upload.
pub struct DecodedFrame {
    pub rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub pts: f64, // presentation timestamp in seconds
}

/// Commands sent from main thread to decode thread.
pub enum DecodeCmd {
    Play,
    Pause,
    Seek(f64), // seek to time in seconds
    Stop,
}

/// Metadata about the opened video file.
pub struct VideoMeta {
    pub duration: f64,
    pub fps: f32,
    pub width: u32,
    pub height: u32,
}

/// Handle to the decoder thread, held by the main thread.
pub struct DecoderHandle {
    pub frame_rx: Receiver<DecodedFrame>,
    pub cmd_tx: Sender<DecodeCmd>,
    pub meta: VideoMeta,
    thread: Option<thread::JoinHandle<()>>,
}

impl DecoderHandle {
    /// Open a video file, read metadata, and spawn the decode thread.
    pub fn open(path: &Path) -> Result<Self> {
        // Open decoder on main thread to read metadata, then move to thread
        let decoder = Decoder::new(path)
            .map_err(|e| anyhow!("failed to open video: {e}"))?;

        let (w, h) = decoder.size();
        let fps = decoder.frame_rate();
        let duration = decoder.duration()
            .map_err(|e| anyhow!("failed to get duration: {e}"))?
            .as_secs_f64();

        let meta = VideoMeta { duration, fps, width: w, height: h };

        let (frame_tx, frame_rx) = bounded::<DecodedFrame>(3);
        let (cmd_tx, cmd_rx) = bounded::<DecodeCmd>(16);

        let thread = thread::spawn(move || {
            decode_loop(decoder, frame_tx, cmd_rx);
        });

        Ok(Self {
            frame_rx,
            cmd_tx,
            meta,
            thread: Some(thread),
        })
    }

    pub fn send(&self, cmd: DecodeCmd) {
        let _ = self.cmd_tx.send(cmd);
    }
}

impl Drop for DecoderHandle {
    fn drop(&mut self) {
        let _ = self.cmd_tx.send(DecodeCmd::Stop);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// Convert RGB24 data to RGBA (add alpha=255 to each pixel).
fn rgb_to_rgba(rgb: &[u8], w: u32, h: u32) -> Vec<u8> {
    let expected = (w * h * 3) as usize;
    let mut rgba = Vec::with_capacity((w * h * 4) as usize);
    let data = if rgb.len() >= expected { &rgb[..expected] } else { rgb };
    for pixel in data.chunks_exact(3) {
        rgba.push(pixel[0]);
        rgba.push(pixel[1]);
        rgba.push(pixel[2]);
        rgba.push(255);
    }
    rgba
}

/// Main decode loop running on the decode thread.
fn decode_loop(
    mut decoder: Decoder,
    frame_tx: Sender<DecodedFrame>,
    cmd_rx: Receiver<DecodeCmd>,
) {
    let (w, h) = decoder.size();
    let fps = decoder.frame_rate();
    let frame_dur = if fps > 0.0 { Duration::from_secs_f64(1.0 / fps as f64) } else { Duration::from_millis(33) };

    let mut playing = false;
    let mut play_start = Instant::now();
    let mut play_start_pts = 0.0f64;

    loop {
        // Check commands (non-blocking when playing, blocking when paused)
        let cmd = if playing {
            match cmd_rx.try_recv() {
                Ok(cmd) => Some(cmd),
                Err(TryRecvError::Empty) => None,
                Err(TryRecvError::Disconnected) => return,
            }
        } else {
            // Block until we get a command
            match cmd_rx.recv() {
                Ok(cmd) => Some(cmd),
                Err(_) => return,
            }
        };

        if let Some(cmd) = cmd {
            match cmd {
                DecodeCmd::Play => {
                    playing = true;
                    play_start = Instant::now();
                    play_start_pts = last_pts_or(&decoder, 0.0);
                }
                DecodeCmd::Pause => {
                    playing = false;
                    continue;
                }
                DecodeCmd::Seek(secs) => {
                    let ms = (secs * 1000.0) as i64;
                    let _ = decoder.seek(ms.max(0));
                    // Decode one frame at the seek position and send it
                    if let Ok((time, frame)) = decoder.decode() {
                        let pts = time.as_secs_f64();
                        if let Some(rgb) = frame.as_slice() {
                            let rgba = rgb_to_rgba(rgb, w, h);
                            let _ = frame_tx.try_send(DecodedFrame { rgba, width: w, height: h, pts });
                        }
                    }
                    if playing {
                        play_start = Instant::now();
                        play_start_pts = secs;
                    }
                    continue;
                }
                DecodeCmd::Stop => return,
            }
        }

        if !playing { continue; }

        // Decode next frame
        match decoder.decode() {
            Ok((time, frame)) => {
                let pts = time.as_secs_f64();
                let rgb = match frame.as_slice() {
                    Some(s) => s,
                    None => continue,
                };
                let rgba = rgb_to_rgba(rgb, w, h);

                // Frame pacing: wait until wall-clock matches PTS
                let elapsed = play_start.elapsed().as_secs_f64();
                let target = pts - play_start_pts;
                if target > elapsed {
                    let wait = Duration::from_secs_f64((target - elapsed).min(frame_dur.as_secs_f64()));
                    thread::sleep(wait);
                }

                // Send frame (blocks if channel full — natural backpressure)
                if frame_tx.send(DecodedFrame { rgba, width: w, height: h, pts }).is_err() {
                    return; // main thread dropped the receiver
                }
            }
            Err(_) => {
                // End of stream — pause
                playing = false;
            }
        }
    }
}

fn last_pts_or(_decoder: &Decoder, default: f64) -> f64 {
    default
}

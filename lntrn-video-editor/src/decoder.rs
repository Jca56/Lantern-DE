//! FFmpeg video decoder running on a dedicated thread.
//!
//! Opens a video file via `video-rs`, decodes frames to RGBA, and sends them
//! through a channel to the main thread. Responds to play/pause/seek
//! commands via a separate command channel.

use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};
use crossbeam_channel::{unbounded, Receiver, Sender, TryRecvError};
use video_rs::decode::Decoder;

const SEEK_PREVIEW_SCAN_FRAMES: usize = 90;

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
        // Open decoder on main thread to read metadata, then move to thread.
        let decoder = Decoder::new(path).map_err(|e| anyhow!("failed to open video: {e}"))?;

        let (w, h) = decoder.size();
        let fps = decoder.frame_rate();
        let duration = decoder
            .duration()
            .map_err(|e| anyhow!("failed to get duration: {e}"))?
            .as_secs_f64();

        let meta = VideoMeta {
            duration,
            fps,
            width: w,
            height: h,
        };

        let (frame_tx, frame_rx) = unbounded::<DecodedFrame>();
        let (cmd_tx, cmd_rx) = unbounded::<DecodeCmd>();

        let path = path.to_path_buf();
        let thread = thread::spawn(move || {
            decode_loop(path, decoder, frame_tx, cmd_rx);
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
    let data = if rgb.len() >= expected {
        &rgb[..expected]
    } else {
        rgb
    };
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
    path: PathBuf,
    mut decoder: Decoder,
    frame_tx: Sender<DecodedFrame>,
    cmd_rx: Receiver<DecodeCmd>,
) {
    let (w, h) = decoder.size();
    let fps = decoder.frame_rate();
    let frame_dur = if fps > 0.0 {
        Duration::from_secs_f64(1.0 / fps as f64)
    } else {
        Duration::from_millis(33)
    };

    let mut playing = false;
    let mut play_start = Instant::now();
    let mut play_start_pts = 0.0f64;
    let mut current_pts = 0.0f64;

    loop {
        // Check commands (non-blocking when playing, blocking when paused)
        let first_cmd = if playing {
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

        if let Some(first_cmd) = first_cmd {
            let mut pending_seek = None;
            let mut should_continue = false;

            for cmd in std::iter::once(first_cmd).chain(cmd_rx.try_iter()) {
                match cmd {
                    DecodeCmd::Play => {
                        if let Some(secs) = pending_seek.take() {
                            current_pts =
                                seek_and_send_frame(&path, &mut decoder, &frame_tx, w, h, secs)
                                    .unwrap_or(secs);
                        }
                        playing = true;
                        play_start = Instant::now();
                        play_start_pts = current_pts;
                    }
                    DecodeCmd::Pause => {
                        if let Some(secs) = pending_seek.take() {
                            current_pts =
                                seek_and_send_frame(&path, &mut decoder, &frame_tx, w, h, secs)
                                    .unwrap_or(secs);
                        }
                        playing = false;
                        should_continue = true;
                    }
                    DecodeCmd::Seek(secs) => {
                        pending_seek = Some(secs);
                    }
                    DecodeCmd::Stop => return,
                }
            }

            if let Some(secs) = pending_seek.take() {
                current_pts =
                    seek_and_send_frame(&path, &mut decoder, &frame_tx, w, h, secs).unwrap_or(secs);
                if playing {
                    play_start = Instant::now();
                    play_start_pts = current_pts;
                }
                should_continue = true;
            }

            if should_continue {
                continue;
            }
        }

        if !playing {
            continue;
        }

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
                    let wait =
                        Duration::from_secs_f64((target - elapsed).min(frame_dur.as_secs_f64()));
                    thread::sleep(wait);
                }

                // Send the frame to the UI thread; playback pacing keeps this
                // from outrunning the renderer under normal conditions.
                current_pts = pts;
                if frame_tx
                    .send(DecodedFrame {
                        rgba,
                        width: w,
                        height: h,
                        pts,
                    })
                    .is_err()
                {
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

fn seek_and_send_frame(
    path: &Path,
    decoder: &mut Decoder,
    frame_tx: &Sender<DecodedFrame>,
    w: u32,
    h: u32,
    secs: f64,
) -> Option<f64> {
    let target = secs.max(0.0);
    let frame = match decode_seek_frame(decoder, target) {
        Some(frame) => frame,
        None => {
            let mut rebuilt = Decoder::new(path).ok()?;
            let frame = decode_seek_frame(&mut rebuilt, target)?;
            *decoder = rebuilt;
            frame
        }
    };

    send_decoded_frame(frame_tx, frame, w, h)
}

fn decode_seek_frame(
    decoder: &mut Decoder,
    target_secs: f64,
) -> Option<(video_rs::Time, video_rs::Frame)> {
    let ms = (target_secs * 1000.0) as i64;
    if decoder.seek(ms).is_ok() {
        if let Some(frame) = decode_until(decoder, target_secs, SEEK_PREVIEW_SCAN_FRAMES) {
            return Some(frame);
        }
    }

    // `video-rs`' millisecond seek uses a narrow one-second range. When that
    // fails, use FFmpeg's timestamp seek directly through seek_to_frame and
    // return the nearest decoded frame immediately. Despite the name,
    // stream_index=-1 makes this timestamp use AV_TIME_BASE units.
    let timestamp_us = (target_secs * 1_000_000.0) as i64;
    if decoder.seek_to_frame(timestamp_us).is_ok() {
        return decoder.decode().ok();
    }

    decoder.seek_to_start().ok()?;
    decode_until(decoder, target_secs, usize::MAX)
}

fn decode_until(
    decoder: &mut Decoder,
    target_secs: f64,
    max_frames: usize,
) -> Option<(video_rs::Time, video_rs::Frame)> {
    let mut last = None;
    for _ in 0..max_frames {
        let (time, frame) = decoder.decode().ok()?;
        let pts = time.as_secs_f64();
        last = Some((time, frame));
        if pts >= target_secs {
            return last;
        }
    }
    last
}

fn send_decoded_frame(
    frame_tx: &Sender<DecodedFrame>,
    frame: (video_rs::Time, video_rs::Frame),
    w: u32,
    h: u32,
) -> Option<f64> {
    let (time, frame) = frame;
    let pts = time.as_secs_f64();
    let rgb = frame.as_slice()?;
    let rgba = rgb_to_rgba(rgb, w, h);
    frame_tx
        .send(DecodedFrame {
            rgba,
            width: w,
            height: h,
            pts,
        })
        .ok()?;
    Some(pts)
}

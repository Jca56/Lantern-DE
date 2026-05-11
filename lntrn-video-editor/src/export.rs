//! Export pipeline — builds an ffmpeg filter graph from the project and
//! runs it in a background thread, surfacing progress through a global
//! status slot the render layer can read.
//!
//! MVP scope: V1 + A1 only. Multi-track compositing on export is a future
//! enhancement; today the model has V2/A2 tracks for layout but `ffmpeg`
//! only consumes V1 + A1 clips. Track-level mute is respected.

use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, Result};

use crate::project::{Project, TimelineClip, TrackKind};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ExportFormat {
    Mp4,
    Gif,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ExportQuality {
    Low,
    Medium,
    High,
}

#[derive(Clone, Debug)]
pub struct ExportRequest {
    pub format: ExportFormat,
    pub quality: ExportQuality,
    pub output_path: PathBuf,
    /// 0 = derive from source.
    pub width: u32,
    pub height: u32,
    /// 0 = derive from source / sensible default per format.
    pub fps: u32,
}

impl ExportRequest {
    pub fn defaults_for(format: ExportFormat) -> ExportRequest {
        match format {
            ExportFormat::Mp4 => ExportRequest {
                format,
                quality: ExportQuality::Medium,
                output_path: default_output_path(format),
                width: 1280,
                height: 720,
                fps: 30,
            },
            ExportFormat::Gif => ExportRequest {
                format,
                quality: ExportQuality::Medium,
                output_path: default_output_path(format),
                width: 480,
                height: 0,
                fps: 15,
            },
        }
    }
}

fn default_output_path(format: ExportFormat) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let (dir, ext) = match format {
        ExportFormat::Mp4 => ("Videos", "mp4"),
        ExportFormat::Gif => ("Pictures", "gif"),
    };
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    let mut out = home;
    out.push(dir);
    if !out.exists() {
        out = std::env::temp_dir();
    }
    out.push(format!("lantern-edit-{stamp}.{ext}"));
    out
}

struct ExportState {
    message: String,
    finished: bool,
}

fn slot() -> &'static Mutex<Option<ExportState>> {
    static SLOT: OnceLock<Mutex<Option<ExportState>>> = OnceLock::new();
    SLOT.get_or_init(|| Mutex::new(None))
}

pub fn status_message() -> Option<String> {
    slot().lock().ok()?.as_ref().map(|s| s.message.clone())
}

pub fn is_running() -> bool {
    slot()
        .lock()
        .ok()
        .and_then(|g| g.as_ref().map(|s| !s.finished))
        .unwrap_or(false)
}

#[allow(dead_code)]
pub fn clear_status() {
    if let Ok(mut g) = slot().lock() {
        *g = None;
    }
}

fn set_status(message: String, finished: bool) {
    if let Ok(mut g) = slot().lock() {
        *g = Some(ExportState { message, finished });
    }
}

pub fn start(req: ExportRequest, project: &Project) -> Result<()> {
    if is_running() {
        return Err(anyhow!("another export is in progress"));
    }
    if project.timeline_clips.is_empty() {
        return Err(anyhow!("timeline is empty — nothing to export"));
    }
    let args = build_args(&req, project)?;
    eprintln!("[export] ffmpeg {}", args.join(" "));
    let total_dur = project.timeline_duration();
    set_status("Exporting…  0%".into(), false);

    thread::spawn(move || {
        let mut cmd = Command::new("ffmpeg");
        cmd.args(&args);
        cmd.stdin(Stdio::null());
        cmd.stdout(Stdio::null());
        cmd.stderr(Stdio::piped());
        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                set_status(format!("Export failed to start: {e}"), true);
                return;
            }
        };
        if let Some(stderr) = child.stderr.take() {
            let reader = BufReader::new(stderr);
            for line in reader.lines().flatten() {
                if let Some(t) = parse_time(&line) {
                    let pct = if total_dur > 0.0 {
                        ((t / total_dur) * 100.0).clamp(0.0, 100.0)
                    } else {
                        0.0
                    };
                    set_status(format!("Exporting…  {pct:.0}%"), false);
                }
            }
        }
        match child.wait() {
            Ok(s) if s.success() => set_status("Export complete ✓".into(), true),
            Ok(s) => set_status(
                format!("Export failed (exit {})", s.code().unwrap_or(-1)),
                true,
            ),
            Err(e) => set_status(format!("Export error: {e}"), true),
        }
    });
    Ok(())
}

fn parse_time(line: &str) -> Option<f64> {
    let idx = line.find("time=")?;
    let t = &line[idx + 5..];
    let end = t.find(' ').unwrap_or(t.len());
    let tc = t[..end].trim();
    let parts: Vec<&str> = tc.split(':').collect();
    if parts.len() != 3 {
        return None;
    }
    let h: f64 = parts[0].parse().ok()?;
    let m: f64 = parts[1].parse().ok()?;
    let s: f64 = parts[2].parse().ok()?;
    Some(h * 3600.0 + m * 60.0 + s)
}

// ── Filter graph builder ───────────────────────────────────────────────────

fn build_args(req: &ExportRequest, project: &Project) -> Result<Vec<String>> {
    let v_clips = sorted_track_clips(project, TrackKind::Video);
    let a_clips = sorted_track_clips(project, TrackKind::Audio);

    // Resolve output dimensions/fps.
    let (out_w, out_h, target_fps) = resolve_dims(req, &v_clips, project);

    let mut args: Vec<String> = vec!["-y".into(), "-hide_banner".into()];
    let mut filter_parts: Vec<String> = Vec::new();
    let mut input_idx: usize = 0;

    // ── Video inputs + per-clip filter chains ───────────────────────────
    let mut v_labels: Vec<String> = Vec::new();
    for clip in &v_clips {
        let media = project
            .media_by_id(clip.media_id)
            .ok_or_else(|| anyhow!("media missing for clip"))?;
        let src_dur = (clip.duration * clip.speed as f64).max(0.04);
        args.push("-ss".into());
        args.push(format!("{:.4}", clip.media_in));
        args.push("-t".into());
        args.push(format!("{:.4}", src_dur));
        args.push("-i".into());
        args.push(media.path.display().to_string());

        let speed = clip.speed.max(0.05);
        let setpts = format!("setpts={:.6}*(PTS-STARTPTS)", 1.0 / speed as f64);
        let scale_to_clip = format!("scale=iw*{s:.4}:ih*{s:.4}", s = clip.transform.scale);
        // Aspect-fit into output canvas, then pad with black.
        let fit = format!(
            "scale={out_w}:{out_h}:force_original_aspect_ratio=decrease,pad={out_w}:{out_h}:(ow-iw)/2:(oh-ih)/2:color=black"
        );
        let fps_filt = format!("fps={target_fps}");
        let alpha = format!(
            "format=yuva420p,colorchannelmixer=aa={:.3}",
            clip.transform.opacity.clamp(0.0, 1.0)
        );
        let label = format!("v{input_idx}");
        // Order matters: scale-to-clip first (for resize), then fit into canvas.
        filter_parts.push(format!(
            "[{input_idx}:v]{setpts},{scale_to_clip},{fit},{fps_filt},setsar=1,{alpha},format=yuv420p[{label}]"
        ));
        v_labels.push(label);
        input_idx += 1;
    }

    // ── Audio inputs + per-clip filter chains ───────────────────────────
    let mut a_labels: Vec<String> = Vec::new();
    for clip in &a_clips {
        let media = project
            .media_by_id(clip.media_id)
            .ok_or_else(|| anyhow!("media missing for clip"))?;
        let src_dur = (clip.duration * clip.speed as f64).max(0.04);
        args.push("-ss".into());
        args.push(format!("{:.4}", clip.media_in));
        args.push("-t".into());
        args.push(format!("{:.4}", src_dur));
        args.push("-i".into());
        args.push(media.path.display().to_string());

        let atempo = build_atempo_chain(clip.speed.max(0.05) as f64);
        let vol = format!("volume={:.3}", clip.volume.clamp(0.0, 4.0));
        let label = format!("a{input_idx}");
        filter_parts.push(format!("[{input_idx}:a]{atempo},{vol},aresample=async=1[{label}]"));
        a_labels.push(label);
        input_idx += 1;
    }
    let _ = input_idx; // kept for clarity; final synthesize-black branch updates locally.

    // ── Compose final video stream ──────────────────────────────────────
    let concat_v_label = if v_labels.is_empty() {
        // Synthesize a black background for the whole timeline duration.
        let dur = project.timeline_duration().max(0.04);
        args.push("-f".into());
        args.push("lavfi".into());
        args.push("-t".into());
        args.push(format!("{dur:.4}"));
        args.push("-i".into());
        args.push(format!(
            "color=c=black:s={out_w}x{out_h}:r={target_fps}"
        ));
        let label = format!("v_bg{input_idx}");
        filter_parts.push(format!("[{input_idx}:v]copy[{label}]"));
        input_idx += 1;
        label
    } else if v_labels.len() == 1 {
        v_labels[0].clone()
    } else {
        let mut s = String::new();
        for l in &v_labels {
            s += &format!("[{l}]");
        }
        s += &format!("concat=n={}:v=1:a=0[v_concat]", v_labels.len());
        filter_parts.push(s);
        "v_concat".into()
    };

    let final_v_label = match req.format {
        ExportFormat::Mp4 => concat_v_label,
        ExportFormat::Gif => {
            let gfps = if req.fps > 0 { req.fps } else { 15 };
            let gw = if req.width > 0 { req.width } else { 480 };
            filter_parts.push(format!(
                "[{concat_v_label}]fps={gfps},scale={gw}:-1:flags=lanczos,split[g0][g1]"
            ));
            filter_parts.push("[g0]palettegen=stats_mode=diff[gp]".into());
            filter_parts.push("[g1][gp]paletteuse=dither=bayer:bayer_scale=5[g_out]".into());
            "g_out".into()
        }
    };

    // ── Compose final audio stream (MP4 only) ──────────────────────────
    let final_a_label = if matches!(req.format, ExportFormat::Mp4) && !a_labels.is_empty() {
        if a_labels.len() == 1 {
            Some(a_labels[0].clone())
        } else {
            let mut s = String::new();
            for l in &a_labels {
                s += &format!("[{l}]");
            }
            s += &format!("concat=n={}:v=0:a=1[a_concat]", a_labels.len());
            filter_parts.push(s);
            Some("a_concat".into())
        }
    } else {
        None
    };

    args.push("-filter_complex".into());
    args.push(filter_parts.join(";"));

    args.push("-map".into());
    args.push(format!("[{final_v_label}]"));
    if let Some(a) = &final_a_label {
        args.push("-map".into());
        args.push(format!("[{a}]"));
    }

    match req.format {
        ExportFormat::Mp4 => {
            args.push("-c:v".into());
            args.push("libx264".into());
            args.push("-preset".into());
            args.push("medium".into());
            let crf = match req.quality {
                ExportQuality::Low => 28,
                ExportQuality::Medium => 22,
                ExportQuality::High => 17,
            };
            args.push("-crf".into());
            args.push(crf.to_string());
            args.push("-pix_fmt".into());
            args.push("yuv420p".into());
            if final_a_label.is_some() {
                args.push("-c:a".into());
                args.push("aac".into());
                args.push("-b:a".into());
                args.push("192k".into());
            } else {
                args.push("-an".into());
            }
            args.push("-movflags".into());
            args.push("+faststart".into());
        }
        ExportFormat::Gif => {
            // gif loop forever
            args.push("-loop".into());
            args.push("0".into());
        }
    }

    args.push(req.output_path.display().to_string());
    Ok(args)
}

fn sorted_track_clips<'a>(project: &'a Project, kind: TrackKind) -> Vec<&'a TimelineClip> {
    let mut clips: Vec<&TimelineClip> = project
        .timeline_clips
        .iter()
        .filter(|c| {
            let track = match project.track_by_id(c.track_id) {
                Some(t) => t,
                None => return false,
            };
            if track.kind != kind {
                return false;
            }
            if track.mute {
                return false;
            }
            // MVP: V1 + A1 only — drop higher-index tracks for now.
            track.index == 1
        })
        .collect();
    clips.sort_by(|a, b| {
        a.start
            .partial_cmp(&b.start)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    clips
}

fn resolve_dims(
    req: &ExportRequest,
    v_clips: &[&TimelineClip],
    project: &Project,
) -> (u32, u32, u32) {
    let (mut w, mut h, mut fps) = (req.width, req.height, req.fps);
    if w == 0 || h == 0 || fps == 0 {
        if let Some(first) = v_clips.first() {
            if let Some(m) = project.media_by_id(first.media_id) {
                if w == 0 {
                    w = m.width.max(2);
                }
                if h == 0 {
                    h = m.height.max(2);
                }
                if fps == 0 {
                    fps = m.fps.max(1.0).round() as u32;
                }
            }
        }
    }
    if w == 0 {
        w = 1280;
    }
    if h == 0 {
        h = (w * 9 / 16).max(2);
    }
    if fps == 0 {
        fps = 30;
    }
    // Ensure even dimensions for libx264 (yuv420p).
    if w % 2 != 0 {
        w += 1;
    }
    if h % 2 != 0 {
        h += 1;
    }
    (w, h, fps)
}

fn build_atempo_chain(speed: f64) -> String {
    // atempo accepts 0.5..=2.0 per filter; chain for extremes.
    let mut s = speed;
    let mut parts: Vec<String> = Vec::new();
    while s > 2.0 {
        parts.push("atempo=2.0".into());
        s /= 2.0;
    }
    while s < 0.5 {
        parts.push("atempo=0.5".into());
        s *= 2.0;
    }
    parts.push(format!("atempo={s:.4}"));
    parts.join(",")
}

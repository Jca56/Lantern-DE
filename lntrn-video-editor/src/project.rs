//! Project model: media library, tracks, and timeline clips.
//!
//! Coordinates:
//!   * `clip.start`, `clip.duration` are **timeline** seconds — they describe
//!     where the clip lives on the ruler and how long it occupies.
//!   * `clip.media_in` and `clip.speed` describe the **source** mapping:
//!     timeline time `t` inside the clip plays source pts
//!     `media_in + (t - start) * speed`.
//!   * Effective source range consumed by a clip is therefore
//!     `[media_in, media_in + duration * speed]`.

use std::path::{Path, PathBuf};

use crate::playback::Playback;

pub type MediaId = u64;
pub type ClipId = u64;
pub type TrackId = u64;

/// Minimum clip length after trim, in seconds (~2 frames at 30fps).
pub const MIN_CLIP_DUR: f64 = 1.0 / 15.0;

/// Hard floor on clip.speed so we never divide by ~0 in the timeline mapping.
pub const MIN_CLIP_SPEED: f32 = 0.05;
/// Soft ceiling on speed for UX (export can override).
pub const MAX_CLIP_SPEED: f32 = 10.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackKind {
    Video,
    Audio,
}

pub struct Track {
    pub id: TrackId,
    pub kind: TrackKind,
    /// Display index within its kind (V1, V2, A1, A2 …). 1-based.
    pub index: u32,
    pub mute: bool,
    #[allow(dead_code)]
    pub lock: bool,
}

impl Track {
    pub fn label(&self) -> String {
        let prefix = match self.kind {
            TrackKind::Video => 'V',
            TrackKind::Audio => 'A',
        };
        format!("{prefix}{}", self.index)
    }
}

pub struct MediaItem {
    pub id: MediaId,
    pub path: PathBuf,
    pub name: String,
    pub duration: f64,
    pub fps: f32,
    pub width: u32,
    pub height: u32,
}

/// Per-clip visual transform applied in preview / export.
#[derive(Debug, Clone, Copy)]
pub struct ClipTransform {
    /// Multiplicative scale. 1.0 = aspect-fit into preview area.
    pub scale: f32,
    /// Translation as a fraction of preview width/height. (0,0) = centered.
    pub offset_x: f32,
    pub offset_y: f32,
    /// 0..=1.
    pub opacity: f32,
}

impl ClipTransform {
    pub const IDENTITY: ClipTransform = ClipTransform {
        scale: 1.0,
        offset_x: 0.0,
        offset_y: 0.0,
        opacity: 1.0,
    };
}

pub struct TimelineClip {
    pub id: ClipId,
    pub media_id: MediaId,
    pub track_id: TrackId,
    /// Position on the timeline (seconds).
    pub start: f64,
    /// Length on the timeline (seconds). Source range consumed is
    /// `[media_in, media_in + duration * speed]`.
    pub duration: f64,
    /// Source in-point.
    pub media_in: f64,
    /// Playback rate. 2.0 = double-speed (half timeline length per source sec).
    pub speed: f32,
    /// Per-clip volume (audio clips only — kept on every clip for symmetry).
    pub volume: f32,
    /// Visual transform (video clips only).
    pub transform: ClipTransform,
    /// If Some, this clip is linked to another — trims/moves apply to both.
    /// Used to keep V/A pairs in sync until `unlink` is invoked.
    pub linked_id: Option<ClipId>,
}

impl TimelineClip {
    /// Source range consumed by this clip given its speed.
    pub fn source_duration(&self) -> f64 {
        self.duration * self.speed as f64
    }
    /// End of source range (exclusive).
    pub fn media_out(&self) -> f64 {
        self.media_in + self.source_duration()
    }
    /// Map a timeline time inside the clip to its source PTS.
    #[allow(dead_code)]
    pub fn source_pts_for_timeline(&self, timeline_secs: f64) -> f64 {
        let offset = (timeline_secs - self.start).clamp(0.0, self.duration.max(0.0));
        self.media_in + offset * self.speed as f64
    }
}

pub struct Project {
    pub media: Vec<MediaItem>,
    pub tracks: Vec<Track>,
    pub timeline_clips: Vec<TimelineClip>,
    pub selected_media: Option<MediaId>,
    pub selected_clip: Option<ClipId>,
    next_media_id: MediaId,
    next_clip_id: ClipId,
    next_track_id: TrackId,
}

impl Project {
    pub fn new() -> Self {
        let mut p = Self {
            media: Vec::new(),
            tracks: Vec::new(),
            timeline_clips: Vec::new(),
            selected_media: None,
            selected_clip: None,
            next_media_id: 1,
            next_clip_id: 1,
            next_track_id: 1,
        };
        // Default lane setup: V2/V1 on top, A1/A2 on bottom (top-down render order).
        p.add_track(TrackKind::Video);
        p.add_track(TrackKind::Video);
        p.add_track(TrackKind::Audio);
        p.add_track(TrackKind::Audio);
        p
    }

    // ── Track management ──────────────────────────────────────────────────

    pub fn add_track(&mut self, kind: TrackKind) -> TrackId {
        let id = self.next_track_id;
        self.next_track_id += 1;
        let index = self.tracks.iter().filter(|t| t.kind == kind).count() as u32 + 1;
        self.tracks.push(Track {
            id,
            kind,
            index,
            mute: false,
            lock: false,
        });
        id
    }

    pub fn track_by_id(&self, id: TrackId) -> Option<&Track> {
        self.tracks.iter().find(|t| t.id == id)
    }

    pub fn track_by_id_mut(&mut self, id: TrackId) -> Option<&mut Track> {
        self.tracks.iter_mut().find(|t| t.id == id)
    }

    /// First track of the given kind, in declaration order (= V1/A1).
    pub fn primary_track(&self, kind: TrackKind) -> Option<TrackId> {
        self.tracks
            .iter()
            .find(|t| t.kind == kind)
            .map(|t| t.id)
    }

    pub fn toggle_mute(&mut self, id: TrackId) {
        if let Some(t) = self.track_by_id_mut(id) {
            t.mute = !t.mute;
        }
    }

    // ── Media library ─────────────────────────────────────────────────────

    pub fn import_from_playback(&mut self, path: &Path, playback: &Playback) -> MediaId {
        if let Some(existing) = self.media.iter_mut().find(|item| item.path == path) {
            existing.duration = playback.duration;
            existing.fps = playback.fps;
            existing.width = playback.video_width;
            existing.height = playback.video_height;
            self.selected_media = Some(existing.id);
            self.selected_clip = None;
            return existing.id;
        }

        let id = self.next_media_id;
        self.next_media_id += 1;
        self.media.push(MediaItem {
            id,
            path: path.to_path_buf(),
            name: media_name(path),
            duration: playback.duration,
            fps: playback.fps,
            width: playback.video_width,
            height: playback.video_height,
        });
        self.selected_media = Some(id);
        self.selected_clip = None;
        id
    }

    pub fn select_media(&mut self, id: MediaId) -> Option<&MediaItem> {
        if self.media.iter().any(|item| item.id == id) {
            self.selected_media = Some(id);
            self.selected_clip = None;
        }
        self.selected_media_item()
    }

    pub fn selected_media_item(&self) -> Option<&MediaItem> {
        let id = self.selected_media?;
        self.media.iter().find(|item| item.id == id)
    }

    pub fn media_by_id(&self, id: MediaId) -> Option<&MediaItem> {
        self.media.iter().find(|item| item.id == id)
    }

    // ── Clip operations ───────────────────────────────────────────────────

    pub fn insert_selected_at_playhead(&mut self, playhead_secs: f64) -> Option<ClipId> {
        self.insert_selected_pair(playhead_secs.max(0.0))
    }

    pub fn insert_selected_at_end(&mut self) -> Option<ClipId> {
        let start = self.timeline_duration();
        self.insert_selected_pair(start)
    }

    /// Insert the selected media as a linked V+A pair at `start`. Returns the
    /// video clip id (the audio clip is created next to it and selected via
    /// `linked_id`). For media without separate audio, only the video clip is
    /// produced.
    fn insert_selected_pair(&mut self, start: f64) -> Option<ClipId> {
        let media_id = self.selected_media?;
        let duration = self.media_by_id(media_id)?.duration;
        if duration <= 0.0 {
            return None;
        }
        let v_track = self.primary_track(TrackKind::Video)?;
        let a_track = self.primary_track(TrackKind::Audio);
        let v_id = self.next_clip_id;
        self.next_clip_id += 1;
        let a_id = if a_track.is_some() {
            let id = self.next_clip_id;
            self.next_clip_id += 1;
            Some(id)
        } else {
            None
        };
        self.timeline_clips.push(TimelineClip {
            id: v_id,
            media_id,
            track_id: v_track,
            start,
            duration,
            media_in: 0.0,
            speed: 1.0,
            volume: 1.0,
            transform: ClipTransform::IDENTITY,
            linked_id: a_id,
        });
        if let (Some(a_track_id), Some(a_id)) = (a_track, a_id) {
            self.timeline_clips.push(TimelineClip {
                id: a_id,
                media_id,
                track_id: a_track_id,
                start,
                duration,
                media_in: 0.0,
                speed: 1.0,
                volume: 1.0,
                transform: ClipTransform::IDENTITY,
                linked_id: Some(v_id),
            });
        }
        self.selected_clip = Some(v_id);
        Some(v_id)
    }

    /// Split the clip containing `secs` (and its linked partner, if any).
    pub fn split_at(&mut self, secs: f64) -> Option<ClipId> {
        let target = self.timeline_clips.iter().find(|c| {
            secs > c.start + MIN_CLIP_DUR && secs < c.start + c.duration - MIN_CLIP_DUR
        })?;
        let target_id = target.id;
        let linked = target.linked_id;
        let new_right_id = self.split_one(target_id, secs)?;
        if let Some(linked_id) = linked {
            if let Some(new_right_partner) = self.split_one(linked_id, secs) {
                // Re-link the two new right halves.
                if let Some(c) = self
                    .timeline_clips
                    .iter_mut()
                    .find(|c| c.id == new_right_id)
                {
                    c.linked_id = Some(new_right_partner);
                }
                if let Some(c) = self
                    .timeline_clips
                    .iter_mut()
                    .find(|c| c.id == new_right_partner)
                {
                    c.linked_id = Some(new_right_id);
                }
            }
        }
        self.selected_clip = Some(new_right_id);
        Some(new_right_id)
    }

    /// Split a single clip (ignores linkage). Returns new right-half id.
    fn split_one(&mut self, id: ClipId, secs: f64) -> Option<ClipId> {
        let idx = self.timeline_clips.iter().position(|c| c.id == id)?;
        let (right_start, right_dur, right_media_in, media_id, track_id, speed, volume, xform) = {
            let c = &mut self.timeline_clips[idx];
            if !(secs > c.start + MIN_CLIP_DUR && secs < c.start + c.duration - MIN_CLIP_DUR) {
                return None;
            }
            let timeline_offset = secs - c.start;
            let right_dur = c.duration - timeline_offset;
            let right_in = c.media_in + timeline_offset * c.speed as f64;
            let media_id = c.media_id;
            let track_id = c.track_id;
            let speed = c.speed;
            let volume = c.volume;
            let xform = c.transform;
            c.duration = timeline_offset;
            (secs, right_dur, right_in, media_id, track_id, speed, volume, xform)
        };
        let new_id = self.next_clip_id;
        self.next_clip_id += 1;
        self.timeline_clips.push(TimelineClip {
            id: new_id,
            media_id,
            track_id,
            start: right_start,
            duration: right_dur,
            media_in: right_media_in,
            speed,
            volume,
            transform: xform,
            linked_id: None, // re-linked by split_at after both halves exist
        });
        Some(new_id)
    }

    pub fn trim_left(&mut self, id: ClipId, new_start: f64) {
        let linked = self
            .timeline_clips
            .iter()
            .find(|c| c.id == id)
            .and_then(|c| c.linked_id);
        self.trim_left_one(id, new_start);
        if let Some(lid) = linked {
            self.trim_left_one(lid, new_start);
        }
    }

    fn trim_left_one(&mut self, id: ClipId, new_start: f64) {
        let Some(clip) = self.timeline_clips.iter_mut().find(|c| c.id == id) else {
            return;
        };
        let old_start = clip.start;
        let speed = clip.speed.max(MIN_CLIP_SPEED) as f64;
        // Can't pull the left edge past source-zero (in timeline units).
        let min_start = (old_start - clip.media_in / speed).max(0.0);
        let max_start = old_start + clip.duration - MIN_CLIP_DUR;
        let new_start = new_start.clamp(min_start, max_start);
        let delta_timeline = new_start - old_start;
        clip.start = new_start;
        clip.media_in = (clip.media_in + delta_timeline * speed).max(0.0);
        clip.duration -= delta_timeline;
    }

    pub fn trim_right(&mut self, id: ClipId, new_end: f64, source_dur: f64) {
        let linked = self
            .timeline_clips
            .iter()
            .find(|c| c.id == id)
            .and_then(|c| c.linked_id);
        self.trim_right_one(id, new_end, source_dur);
        if let Some(lid) = linked {
            self.trim_right_one(lid, new_end, source_dur);
        }
    }

    fn trim_right_one(&mut self, id: ClipId, new_end: f64, source_dur: f64) {
        let Some(clip) = self.timeline_clips.iter_mut().find(|c| c.id == id) else {
            return;
        };
        let speed = clip.speed.max(MIN_CLIP_SPEED) as f64;
        // Available source from current media_in to end of media, in timeline seconds.
        let max_timeline_dur = ((source_dur - clip.media_in) / speed).max(MIN_CLIP_DUR);
        let min_end = clip.start + MIN_CLIP_DUR;
        let max_end = clip.start + max_timeline_dur;
        let new_end = new_end.clamp(min_end, max_end);
        clip.duration = new_end - clip.start;
    }

    /// Multiply a clip's speed by `factor`. Adjusts timeline duration so the
    /// same source range stays mapped (i.e. faster speed = shorter clip).
    pub fn nudge_speed(&mut self, id: ClipId, factor: f32) {
        let Some(clip) = self.timeline_clips.iter_mut().find(|c| c.id == id) else {
            return;
        };
        let new_speed = (clip.speed * factor).clamp(MIN_CLIP_SPEED, MAX_CLIP_SPEED);
        // Keep source range the same: timeline_dur * old_speed == source_dur
        let source_range = clip.duration * clip.speed as f64;
        clip.speed = new_speed;
        clip.duration = source_range / new_speed as f64;
    }

    /// Set absolute speed for a clip (used by inspector).
    pub fn set_speed(&mut self, id: ClipId, speed: f32) {
        let Some(clip) = self.timeline_clips.iter_mut().find(|c| c.id == id) else {
            return;
        };
        let new_speed = speed.clamp(MIN_CLIP_SPEED, MAX_CLIP_SPEED);
        let source_range = clip.duration * clip.speed as f64;
        clip.speed = new_speed;
        clip.duration = source_range / new_speed as f64;
    }

    pub fn set_scale(&mut self, id: ClipId, scale: f32) {
        if let Some(clip) = self.timeline_clips.iter_mut().find(|c| c.id == id) {
            clip.transform.scale = scale.clamp(0.05, 10.0);
        }
    }

    pub fn set_volume(&mut self, id: ClipId, volume: f32) {
        if let Some(clip) = self.timeline_clips.iter_mut().find(|c| c.id == id) {
            clip.volume = volume.clamp(0.0, 4.0);
        }
    }

    /// Break the linked relationship for a clip and its partner. Returns true
    /// if a linked partner existed.
    pub fn unlink(&mut self, id: ClipId) -> bool {
        let partner = self
            .timeline_clips
            .iter_mut()
            .find(|c| c.id == id)
            .and_then(|c| c.linked_id.take());
        if let Some(pid) = partner {
            if let Some(p) = self.timeline_clips.iter_mut().find(|c| c.id == pid) {
                p.linked_id = None;
            }
            true
        } else {
            false
        }
    }

    pub fn select_clip(&mut self, id: ClipId) -> bool {
        if self.timeline_clips.iter().any(|clip| clip.id == id) {
            self.selected_clip = Some(id);
            true
        } else {
            false
        }
    }

    pub fn clear_clip_selection(&mut self) {
        self.selected_clip = None;
    }

    pub fn selected_clip_ref(&self) -> Option<&TimelineClip> {
        let id = self.selected_clip?;
        self.timeline_clips.iter().find(|c| c.id == id)
    }

    pub fn clip_by_id(&self, id: ClipId) -> Option<&TimelineClip> {
        self.timeline_clips.iter().find(|c| c.id == id)
    }

    /// Delete the selected clip and its linked partner.
    pub fn delete_selected_clip(&mut self) -> Option<TimelineClip> {
        let selected = self.selected_clip?;
        let partner = self
            .timeline_clips
            .iter()
            .find(|c| c.id == selected)
            .and_then(|c| c.linked_id);
        if let Some(pid) = partner {
            if let Some(pidx) = self.timeline_clips.iter().position(|c| c.id == pid) {
                let _ = self.timeline_clips.remove(pidx);
            }
        }
        let idx = self
            .timeline_clips
            .iter()
            .position(|clip| clip.id == selected)?;
        self.selected_clip = None;
        Some(self.timeline_clips.remove(idx))
    }

    pub fn timeline_duration(&self) -> f64 {
        self.timeline_clips
            .iter()
            .map(|clip| clip.start + clip.duration)
            .fold(0.0, f64::max)
    }
}

fn media_name(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or("Untitled Media")
        .to_string()
}

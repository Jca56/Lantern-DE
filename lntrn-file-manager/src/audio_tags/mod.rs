//! Audio tag engine — our own ID3v2 / RIFF-INFO / MPEG-frame parsers and
//! writers, no external crates. WAV and MP3 are simple enough that a few
//! hundred lines beat a dependency.
//!
//! * [`read`] sniffs the container by extension and returns tags + stream facts.
//! * [`write`] touches only metadata: WAV tags live in trailing chunks (O(1)
//!   tail append even on a 600 MB set), MP3 tags live in the leading ID3v2
//!   block (rewritten in place when the new tag fits the old padding).

pub mod genres;
pub mod id3;
pub mod id3v1;
pub mod keys;
pub mod mp3;
pub mod wav;

#[cfg(test)]
mod tests;

use std::fs::File;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// Editable tags. Empty string = unset; `artwork: None` = no picture.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AudioTags {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub year: String,
    pub genre: String,
    pub track: String,
    pub bpm: String,
    pub key: String,
    pub artwork: Option<Artwork>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Artwork {
    pub mime: String,
    pub data: Vec<u8>,
}

pub fn sniff_image_mime(b: &[u8]) -> Option<&'static str> {
    if b.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("image/jpeg")
    } else if b.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if b.len() >= 12 && &b[..4] == b"RIFF" && &b[8..12] == b"WEBP" {
        Some("image/webp")
    } else if b.starts_with(b"GIF8") {
        Some("image/gif")
    } else if b.starts_with(b"BM") {
        Some("image/bmp")
    } else {
        None
    }
}

/// Read-only stream facts — shown, never written.
#[derive(Clone, Debug, Default)]
pub struct AudioFormat {
    pub codec: String,
    pub sample_rate: u32,
    pub channels: u16,
    pub bits_per_sample: Option<u16>,
    pub bitrate_kbps: Option<u32>,
    pub vbr: bool,
    pub duration_secs: Option<f64>,
}

impl AudioFormat {
    /// "2:13 · 48 kHz · 16-bit · Stereo · PCM"
    pub fn summary(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        if let Some(d) = self.duration_secs {
            parts.push(format_duration(d));
        }
        if self.sample_rate > 0 {
            parts.push(format_sample_rate(self.sample_rate));
        }
        if let Some(b) = self.bits_per_sample {
            parts.push(format!("{b}-bit"));
        }
        if let Some(k) = self.bitrate_kbps {
            parts.push(if self.vbr {
                format!("VBR ~{k} kbps")
            } else {
                format!("{k} kbps")
            });
        }
        match self.channels {
            0 => {}
            1 => parts.push("Mono".into()),
            2 => parts.push("Stereo".into()),
            n => parts.push(format!("{n} ch")),
        }
        if !self.codec.is_empty() {
            parts.push(self.codec.clone());
        }
        parts.join(" · ")
    }
}

pub fn format_duration(secs: f64) -> String {
    let total = secs.round().max(0.0) as u64;
    let (h, m, s) = (total / 3600, (total % 3600) / 60, total % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

fn format_sample_rate(sr: u32) -> String {
    if sr % 1000 == 0 {
        format!("{} kHz", sr / 1000)
    } else {
        format!("{:.1} kHz", sr as f64 / 1000.0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Container {
    Wav,
    Mp3,
}

#[derive(Clone, Debug)]
pub struct AudioMeta {
    #[allow(dead_code)] // informative; the UI keys off the summary string
    pub container: Container,
    pub tags: AudioTags,
    pub format: AudioFormat,
}

pub fn container_for(path: &Path) -> Option<Container> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    match ext.as_str() {
        "wav" | "wave" => Some(Container::Wav),
        "mp3" => Some(Container::Mp3),
        _ => None,
    }
}

pub fn read(path: &Path) -> Result<AudioMeta, String> {
    match container_for(path).ok_or("Unsupported audio format")? {
        Container::Wav => wav::read(path),
        Container::Mp3 => mp3::read(path),
    }
}

pub fn write(path: &Path, tags: &AudioTags) -> Result<(), String> {
    match container_for(path).ok_or("Unsupported audio format")? {
        Container::Wav => wav::write(path, tags),
        Container::Mp3 => mp3::write(path, tags),
    }
}

pub(crate) fn io_err(e: io::Error) -> String {
    e.to_string()
}

/// Replace `path` via a sibling temp file + rename so a crash mid-write never
/// leaves a half-written audio file behind. `fill` gets (temp, original).
pub(crate) fn replace_file(
    path: &Path,
    fill: impl FnOnce(&mut File, &mut File) -> io::Result<()>,
) -> Result<(), String> {
    let dir = path.parent().unwrap_or(Path::new("."));
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let tmp: PathBuf = dir.join(format!(".{name}.lntrn-tmp"));
    let result = (|| -> io::Result<()> {
        let mut src = File::open(path)?;
        let perms = src.metadata()?.permissions();
        let mut dst = File::create(&tmp)?;
        fill(&mut dst, &mut src)?;
        dst.flush()?;
        dst.sync_all()?;
        std::fs::set_permissions(&tmp, perms)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result.map_err(io_err)
}

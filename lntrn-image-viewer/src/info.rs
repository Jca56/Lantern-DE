//! Per-image facts for the info overlay (I key): container format, file
//! size, modified time, and whatever EXIF the file carries. Gathered once
//! when an image opens; the overlay only formats. Also home to the small
//! date/size formatters shared with trash.

use std::path::Path;
use std::time::SystemTime;

use crate::exif::ExifInfo;

pub struct ImageInfo {
    pub format: String,
    pub file_size: u64,
    pub modified: Option<SystemTime>,
    pub exif: Option<ExifInfo>,
}

impl ImageInfo {
    pub fn gather(path: &Path, format: &str, exif_blob: Option<&[u8]>) -> Self {
        let meta = std::fs::metadata(path).ok();
        Self {
            format: format.to_string(),
            file_size: meta.as_ref().map_or(0, |m| m.len()),
            modified: meta.and_then(|m| m.modified().ok()),
            exif: exif_blob
                .and_then(crate::exif::parse)
                .filter(|e| !e.is_empty()),
        }
    }

    /// Label/value rows in display order; fields that don't apply are skipped.
    pub fn rows(&self) -> Vec<(&'static str, String)> {
        let mut rows = vec![
            ("Format", self.format.clone()),
            ("File size", format_size(self.file_size)),
        ];
        if let Some(m) = self.modified {
            rows.push(("Modified", format_time(m)));
        }
        if let Some(x) = &self.exif {
            if let Some(c) = x.camera() {
                rows.push(("Camera", c));
            }
            if let Some(l) = &x.lens {
                rows.push(("Lens", l.clone()));
            }
            if let Some(d) = &x.date_taken {
                rows.push(("Taken", d.clone()));
            }
            // Exposure triangle on one line: "1/250 s · f/2.8 · ISO 400".
            let mut tri: Vec<String> = Vec::new();
            if let Some(e) = &x.exposure {
                tri.push(e.clone());
            }
            if let Some(a) = &x.aperture {
                tri.push(a.clone());
            }
            if let Some(iso) = x.iso {
                tri.push(format!("ISO {iso}"));
            }
            if !tri.is_empty() {
                rows.push(("Exposure", tri.join("  ·  ")));
            }
            if let Some(f) = &x.focal {
                rows.push(("Focal length", f.clone()));
            }
            if let Some(sw) = &x.software {
                rows.push(("Software", sw.clone()));
            }
        }
        rows
    }
}

/// Human name for a container format ("JPEG", "PNG", …).
pub fn format_name(format: Option<image::ImageFormat>) -> String {
    use image::ImageFormat as F;
    match format {
        Some(F::Png) => "PNG".into(),
        Some(F::Jpeg) => "JPEG".into(),
        Some(F::Gif) => "GIF".into(),
        Some(F::WebP) => "WebP".into(),
        Some(F::Bmp) => "BMP".into(),
        Some(F::Ico) => "ICO".into(),
        Some(F::Tiff) => "TIFF".into(),
        Some(other) => other
            .extensions_str()
            .first()
            .map(|e| e.to_uppercase())
            .unwrap_or_else(|| "Image".into()),
        None => "Image".into(),
    }
}

/// "812 B", "96.4 KB", "2.4 MB", "120 MB".
pub fn format_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = bytes as f64;
    let mut u = 0;
    while v >= 1024.0 && u < UNITS.len() - 1 {
        v /= 1024.0;
        u += 1;
    }
    if u == 0 {
        format!("{bytes} B")
    } else if v >= 100.0 {
        format!("{v:.0} {}", UNITS[u])
    } else {
        format!("{v:.1} {}", UNITS[u])
    }
}

/// Fox-style timestamp ("Aug 30 2026, 07:48 PM"). UTC, same as Fox's
/// Properties panel, so the two never disagree about a file.
pub fn format_time(t: SystemTime) -> String {
    let secs = t
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let (y, mo, d, h, mi, _) = civil(secs);
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let month = MONTHS
        .get((mo as usize).saturating_sub(1))
        .unwrap_or(&"???");
    let h12 = match h {
        0 => 12,
        13..=23 => h - 12,
        _ => h,
    };
    let ampm = if h < 12 { "AM" } else { "PM" };
    format!("{month} {d} {y}, {h12:02}:{mi:02} {ampm}")
}

/// "2026-08-30T19:48:34" — the shape the FreeDesktop trash spec wants.
pub fn iso_timestamp(t: SystemTime) -> String {
    let secs = t
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let (y, mo, d, h, mi, s) = civil(secs);
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}")
}

/// Seconds since the Unix epoch → (year, month, day, hour, minute, second).
fn civil(secs: u64) -> (u64, u64, u64, u64, u64, u64) {
    let mut days = secs / 86400;
    let tod = secs % 86400;
    let mut y = 1970;
    loop {
        let ydays = if is_leap(y) { 366 } else { 365 };
        if days < ydays {
            break;
        }
        days -= ydays;
        y += 1;
    }
    let feb = if is_leap(y) { 29 } else { 28 };
    let mdays = [31, feb, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let mut mo = 1;
    for md in mdays {
        if days < md {
            break;
        }
        days -= md;
        mo += 1;
    }
    (y, mo, days + 1, tod / 3600, (tod % 3600) / 60, tod % 60)
}

fn is_leap(y: u64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn sizes() {
        assert_eq!(format_size(812), "812 B");
        assert_eq!(format_size(98_714), "96.4 KB");
        assert_eq!(format_size(2_516_582), "2.4 MB");
        assert_eq!(format_size(125_829_120), "120 MB");
    }

    #[test]
    fn dates() {
        // 2026-08-30 19:48:34 UTC
        let t = SystemTime::UNIX_EPOCH + Duration::from_secs(1_788_119_314);
        assert_eq!(iso_timestamp(t), "2026-08-30T19:48:34");
        assert_eq!(format_time(t), "Aug 30 2026, 07:48 PM");
        // Leap day + midnight roll.
        let leap = SystemTime::UNIX_EPOCH + Duration::from_secs(1_772_323_200);
        assert_eq!(iso_timestamp(leap), "2026-03-01T00:00:00");
        let feb29 = SystemTime::UNIX_EPOCH + Duration::from_secs(951_782_400);
        assert_eq!(iso_timestamp(feb29), "2000-02-29T00:00:00");
    }
}

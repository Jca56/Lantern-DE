//! Disk usage tile: two stacked usage bars for the root (`/`) and home
//! (`~`) filesystems. Backed by `statfs(2)` (via the already-present
//! `libc` dep) so there's no `df` subprocess. Usage changes slowly, so
//! we re-poll on a lazy interval from `tick()`.

use std::time::{Duration, Instant};

use lntrn_render::{Color, Painter, Rect, TextRenderer};

use crate::controls::tile::TileLayout;

/// Logical width reserved for the disk tile in the controls row.
pub const TILE_WIDTH: f32 = 138.0;

/// How often to re-stat the filesystems. Disk usage barely moves, so a
/// slow poll keeps the syscall cost negligible.
const POLL_INTERVAL: Duration = Duration::from_secs(20);

const LABEL_FONT: f32 = 15.0;
const PCT_FONT: f32 = 15.0;
const BAR_H: f32 = 8.0;
const LEFT_PAD: f32 = 8.0;
/// Logical px reserved for the leading "/" / "~" mount label.
const LABEL_W: f32 = 14.0;
/// Logical px reserved for the trailing "100%" readout.
const PCT_W: f32 = 40.0;

/// Usage thresholds → bar color. Green under 70%, amber under 90%, red above.
const OK_RGB: (u8, u8, u8) = (0x4c, 0xd1, 0x64);
const WARN_RGB: (u8, u8, u8) = (0xff, 0xd6, 0x3c);
const FULL_RGB: (u8, u8, u8) = (0xff, 0x50, 0x40);

/// Used-space fraction for one filesystem plus the label to show for it.
struct Entry {
    label: &'static str,
    frac: f32,
}

pub struct Disk {
    root_frac: Option<f32>,
    home_frac: Option<f32>,
    home_path: Option<String>,
    last_poll: Option<Instant>,
}

impl Disk {
    pub fn new() -> Self {
        // Home is only shown when it's a distinct mount from root —
        // otherwise the two bars would always read identically.
        let home_path = std::env::var("HOME").ok().filter(|h| h != "/");
        let mut d = Self {
            root_frac: None,
            home_frac: None,
            home_path,
            last_poll: None,
        };
        d.poll();
        d
    }

    /// Root always stat-able, so the tile is always present.
    pub const fn is_present(&self) -> bool {
        true
    }

    /// Re-stat on the lazy interval. Cheap to call every frame.
    pub fn tick(&mut self) {
        let due = self
            .last_poll
            .map(|p| p.elapsed() >= POLL_INTERVAL)
            .unwrap_or(true);
        if due {
            self.poll();
        }
    }

    fn poll(&mut self) {
        self.root_frac = statfs_used_fraction("/");
        self.home_frac = self.home_path.as_deref().and_then(statfs_used_fraction);
        self.last_poll = Some(Instant::now());
    }

    /// The rows to render: always root, plus home when it's a separate
    /// mount with a distinct usage reading.
    fn entries(&self) -> Vec<Entry> {
        let mut out = Vec::new();
        if let Some(f) = self.root_frac {
            out.push(Entry {
                label: "/",
                frac: f,
            });
        }
        // Only show home if it differs from root (separate partition).
        if let (Some(h), Some(r)) = (self.home_frac, self.root_frac) {
            if (h - r).abs() > 0.001 {
                out.push(Entry {
                    label: "~",
                    frac: h,
                });
            }
        } else if let Some(h) = self.home_frac {
            out.push(Entry {
                label: "~",
                frac: h,
            });
        }
        out
    }
}

impl Default for Disk {
    fn default() -> Self {
        Self::new()
    }
}

pub fn draw_inline(
    painter: &mut Painter,
    text: &mut TextRenderer,
    disk: &Disk,
    layout: &TileLayout,
    scale: f32,
    alpha: f32,
    surface_w: u32,
    surface_h: u32,
) {
    let entries = disk.entries();
    if entries.is_empty() {
        return;
    }

    let label_font = LABEL_FONT * scale;
    let pct_font = PCT_FONT * scale;
    let bar_h = BAR_H * scale;
    let left_pad = LEFT_PAD * scale;
    let label_w = LABEL_W * scale;
    let pct_w = PCT_W * scale;

    let track_x = layout.x + left_pad + label_w;
    let track_w = (layout.x + layout.w - left_pad - pct_w - track_x).max(8.0 * scale);

    let n = entries.len() as f32;
    // Vertically center the stack of rows in the tile.
    let row_centers: Vec<f32> = (0..entries.len())
        .map(|i| layout.y + layout.h * (i as f32 + 0.5) / n)
        .collect();

    let white = Color::from_rgb8(0xff, 0xff, 0xff).with_alpha(alpha * 0.85);

    for (entry, cy) in entries.iter().zip(row_centers) {
        // Mount label ("/" or "~").
        text.queue(
            entry.label,
            label_font,
            layout.x + left_pad,
            cy - label_font / 2.0,
            white,
            label_w,
            surface_w,
            surface_h,
        );

        // Track + fill.
        let bar_y = cy - bar_h / 2.0;
        painter.rect_filled(
            Rect::new(track_x, bar_y, track_w, bar_h),
            bar_h * 0.5,
            Color::rgba(1.0, 1.0, 1.0, 0.10 * alpha),
        );
        let fill_w = (track_w * entry.frac.clamp(0.0, 1.0)).max(bar_h);
        let (r, g, b) = usage_color(entry.frac);
        painter.rect_filled(
            Rect::new(track_x, bar_y, fill_w, bar_h),
            bar_h * 0.5,
            Color::from_rgb8(r, g, b).with_alpha(alpha),
        );

        // Percent readout, right-aligned.
        let pct = format!("{}%", (entry.frac * 100.0).round() as i32);
        let pw = text.measure_width(&pct, pct_font);
        text.queue(
            &pct,
            pct_font,
            layout.x + layout.w - pw,
            cy - pct_font / 2.0,
            white,
            pw + 4.0 * scale,
            surface_w,
            surface_h,
        );
    }
}

fn usage_color(frac: f32) -> (u8, u8, u8) {
    if frac < 0.7 {
        OK_RGB
    } else if frac < 0.9 {
        WARN_RGB
    } else {
        FULL_RGB
    }
}

/// `statfs(2)` a path and return its used-space fraction (0..1), matching
/// `df`'s reserved-aware accounting. `None` on any error.
fn statfs_used_fraction(path: &str) -> Option<f32> {
    use std::ffi::CString;
    let c = CString::new(path).ok()?;
    let mut buf: libc::statfs = unsafe { std::mem::zeroed() };
    let rc = unsafe { libc::statfs(c.as_ptr(), &mut buf) };
    if rc != 0 {
        return None;
    }
    let blocks = buf.f_blocks as f64;
    let bfree = buf.f_bfree as f64;
    let bavail = buf.f_bavail as f64;
    if blocks <= 0.0 {
        return None;
    }
    let used = blocks - bfree;
    let denom = used + bavail;
    if denom <= 0.0 {
        return None;
    }
    Some((used / denom).clamp(0.0, 1.0) as f32)
}

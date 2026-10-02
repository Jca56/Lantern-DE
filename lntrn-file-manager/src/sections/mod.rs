use lntrn_render::{Color, Painter, Rect};
use lntrn_ui::gpu::{FoxPalette, GradientStrip, InteractionState, Scrollbar};

pub mod crumbs;
pub mod emblem;
mod grid;
mod icons;
mod nav;
pub mod root_badge;
mod sidebar;
mod sidebar_zones;
mod split;
mod status;

pub use grid::{draw_content_grid, draw_rubber_band};
pub use nav::draw_nav_bar;
pub use sidebar::draw_sidebar;
pub use sidebar_zones::{register_sidebar_zones, SidebarHovered};
pub use split::{draw_split_divider, draw_split_toggle_icon, render_inactive_pane, InactivePane};
pub use status::draw_status_bar;

pub fn selection_tint(_palette: &FoxPalette) -> Color {
    Color::from_rgb8(255, 200, 0)
}

// ── Gradient separators ─────────────────────────────────────────────────────

/// When true, the rainbow gradient dividers render as solid accent-colored
/// lines instead. Persisted via `Settings::solid_dividers`, toggled live from
/// the View menu (same static-atomic pattern as `layout::CHROME_HIDDEN`).
pub static SOLID_DIVIDERS: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

fn solid_dividers() -> bool {
    SOLID_DIVIDERS.load(std::sync::atomic::Ordering::Relaxed)
}

pub fn draw_gradient_h(
    painter: &mut Painter,
    palette: &FoxPalette,
    x: f32,
    y: f32,
    width: f32,
    s: f32,
) {
    if solid_dividers() {
        painter.rect_filled(Rect::new(x, y, width, 4.0 * s), 0.0, palette.accent);
        return;
    }
    let mut bar = GradientStrip::new(x, y, width);
    bar.height = 4.0 * s;
    bar.colors = palette.file_manager_gradient_stops();
    bar.draw(painter);
}

pub fn draw_gradient_v(
    painter: &mut Painter,
    palette: &FoxPalette,
    x: f32,
    y: f32,
    height: f32,
    s: f32,
) {
    if solid_dividers() {
        painter.rect_filled(Rect::new(x, y, 4.0 * s, height), 0.0, palette.accent);
        return;
    }
    let colors = palette.file_manager_gradient_stops();
    let w = 4.0 * s;
    let segments = height.max(1.0).ceil() as usize;
    let step = height / segments as f32;
    for i in 0..segments {
        let sy = y + i as f32 * step;
        let sh = if i + 1 == segments {
            y + height - sy
        } else {
            step
        };
        let t = i as f32 / segments as f32;
        let color = sample_gradient_5(&colors, t);
        painter.rect_filled(Rect::new(x, sy, w, sh), 0.0, color);
    }
}

fn sample_gradient_5(colors: &[Color; 5], t: f32) -> Color {
    let t = t.clamp(0.0, 1.0);
    let stops = [0.0_f32, 0.25, 0.50, 0.75, 1.0];
    for i in 0..4 {
        if t <= stops[i + 1] {
            let local = (t - stops[i]) / (stops[i + 1] - stops[i]);
            return lerp_color(colors[i], colors[i + 1], local);
        }
    }
    colors[4]
}

fn lerp_color(a: Color, b: Color, t: f32) -> Color {
    Color {
        r: a.r + (b.r - a.r) * t,
        g: a.g + (b.g - a.g) * t,
        b: a.b + (b.b - a.b) * t,
        a: a.a + (b.a - a.a) * t,
    }
}

// ── Scrollbar ───────────────────────────────────────────────────────────────

/// The file list's scrollbar (built by `crate::scrollbar::bar`): hidden
/// until the pointer is on it.
pub fn draw_scrollbar(
    painter: &mut Painter,
    scrollbar: &Scrollbar,
    state: InteractionState,
    palette: &FoxPalette,
) {
    crate::scrollbar::draw(painter, scrollbar, state, palette, false);
}

// ── Breadcrumb helpers ──────────────────────────────────────────────────────

/// Split a path into breadcrumb segments: (display_name, full_path).
/// Replaces home prefix with "Home".
pub fn breadcrumb_segments(path: &std::path::Path, _s: f32) -> Vec<(String, std::path::PathBuf)> {
    let home = crate::app::dirs_home();
    let mut segments = Vec::new();

    if path.starts_with(&home) {
        segments.push(("Home".to_string(), home.clone()));
        if let Ok(rel) = path.strip_prefix(&home) {
            let mut accum = home.clone();
            for comp in rel.components() {
                accum = accum.join(comp);
                segments.push((
                    comp.as_os_str().to_string_lossy().to_string(),
                    accum.clone(),
                ));
            }
        }
    } else {
        let mut accum = std::path::PathBuf::new();
        for comp in path.components() {
            accum = accum.join(comp);
            let name = if accum == std::path::PathBuf::from("/") {
                "/".to_string()
            } else {
                comp.as_os_str().to_string_lossy().to_string()
            };
            segments.push((name, accum.clone()));
        }
    }
    segments
}

// ── Text helpers ────────────────────────────────────────────────────────────

// Grid labels are cut and wrapped by measured width, which costs several
// text-layout lookups per label. Done afresh every frame for every visible
// label, that overflows the text engine's layout cache on a large grid and
// every label is then re-shaped every frame. The result for a given
// (name, width, font size) never changes, so remember it.
type LabelKey = (String, u32, u32);
const LABEL_MEMO_MAX: usize = 4096;
thread_local! {
    static LABEL_FIT: std::cell::RefCell<std::collections::HashMap<LabelKey, (String, f32)>> =
        Default::default();
    static LABEL_WRAP: std::cell::RefCell<std::collections::HashMap<LabelKey, Vec<(String, f32)>>> =
        Default::default();
}

/// `name` cut to fit `max_w` (with an ellipsis if needed) and the measured
/// width of what is left. Memoised.
pub fn fit_label(
    text: &mut lntrn_render::TextRenderer,
    name: &str,
    max_w: f32,
    font_px: f32,
) -> (String, f32) {
    let key = (name.to_string(), max_w.to_bits(), font_px.to_bits());
    if let Some(hit) = LABEL_FIT.with(|m| m.borrow().get(&key).cloned()) {
        return hit;
    }
    let shown = truncate_to_width(text, name, max_w, font_px);
    let width = text.measure_width(&shown, font_px);
    LABEL_FIT.with(|m| {
        let mut m = m.borrow_mut();
        if m.len() >= LABEL_MEMO_MAX {
            m.clear();
        }
        m.insert(key, (shown.clone(), width));
    });
    (shown, width)
}

/// `name` wrapped to lines that fit `max_w`, each with its measured width.
/// Memoised.
pub fn wrap_label(
    text: &mut lntrn_render::TextRenderer,
    name: &str,
    max_w: f32,
    font_px: f32,
) -> Vec<(String, f32)> {
    let key = (name.to_string(), max_w.to_bits(), font_px.to_bits());
    if let Some(hit) = LABEL_WRAP.with(|m| m.borrow().get(&key).cloned()) {
        return hit;
    }
    let lines: Vec<(String, f32)> = wrap_to_width(text, name, max_w, font_px)
        .into_iter()
        .map(|line| {
            let width = text.measure_width(&line, font_px);
            (line, width)
        })
        .collect();
    LABEL_WRAP.with(|m| {
        let mut m = m.borrow_mut();
        if m.len() >= LABEL_MEMO_MAX {
            m.clear();
        }
        m.insert(key, lines.clone());
    });
    lines
}

/// Break `name` into lines that each fit `max_w`, by MEASURED width. A
/// fixed per-character estimate is wrong both ways: wide names overflow the
/// line (and wrap again on their own, onto the next label), multi-byte names
/// break early.
pub fn wrap_to_width(
    text: &mut lntrn_render::TextRenderer,
    name: &str,
    max_w: f32,
    font_px: f32,
) -> Vec<String> {
    if text.measure_width(name, font_px) <= max_w {
        return vec![name.to_string()];
    }
    let chars: Vec<char> = name.chars().collect();
    let mut lines = Vec::new();
    let mut start = 0;
    while start < chars.len() {
        // Longest run from `start` that fits; always at least one char.
        let (mut lo, mut hi) = (1usize, chars.len() - start);
        while lo < hi {
            let mid = (lo + hi + 1) / 2;
            let candidate: String = chars[start..start + mid].iter().collect();
            if text.measure_width(&candidate, font_px) <= max_w {
                lo = mid;
            } else {
                hi = mid - 1;
            }
        }
        lines.push(chars[start..start + lo].iter().collect());
        start += lo;
    }
    lines
}

/// Truncate `name` to fit `max_w` pixels using the renderer's *actual* glyph
/// measurements (cached), with a trailing ellipsis. A per-character width
/// estimate underestimates wide glyphs (m, w, …), which lets the leftover
/// wrap onto a second line; measuring is exact. Binary-searches the
/// longest prefix that fits — ~log2(len) cached measurements per long name.
pub fn truncate_to_width(
    text: &mut lntrn_render::TextRenderer,
    name: &str,
    max_w: f32,
    font_px: f32,
) -> String {
    if max_w <= 0.0 {
        return String::new();
    }
    if text.measure_width(name, font_px) <= max_w {
        return name.to_string();
    }
    let chars: Vec<char> = name.chars().collect();
    let mut lo = 0usize;
    let mut hi = chars.len();
    while lo < hi {
        let mid = (lo + hi + 1) / 2;
        let mut candidate: String = chars[..mid].iter().collect();
        candidate.push('\u{2026}');
        if text.measure_width(&candidate, font_px) <= max_w {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    let mut out: String = chars[..lo].iter().collect();
    out.push('\u{2026}');
    out
}

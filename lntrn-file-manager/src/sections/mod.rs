use lntrn_render::{Color, Painter, Rect};
use lntrn_ui::gpu::{FoxPalette, GradientStrip, InteractionState, Scrollbar};

mod grid;
mod icons;
mod nav;
mod sidebar;
mod status;

pub use grid::{draw_content_grid, draw_rubber_band};
pub use nav::draw_nav_bar;
pub use sidebar::{draw_sidebar, SidebarHovered};
pub use status::draw_status_bar;

pub fn selection_tint(_palette: &FoxPalette) -> Color {
    Color::from_rgb8(255, 200, 0)
}

// ── Gradient separators ─────────────────────────────────────────────────────

/// When true, the rainbow gradient dividers render as solid accent-colored
/// lines instead. Persisted via `Settings::solid_dividers`, toggled live from
/// the View menu (same static-atomic pattern as `layout::CHROME_HIDDEN`).
pub static SOLID_DIVIDERS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn solid_dividers() -> bool {
    SOLID_DIVIDERS.load(std::sync::atomic::Ordering::Relaxed)
}

pub fn draw_gradient_h(painter: &mut Painter, palette: &FoxPalette, x: f32, y: f32, width: f32, s: f32) {
    if solid_dividers() {
        painter.rect_filled(Rect::new(x, y, width, 4.0 * s), 0.0, palette.accent);
        return;
    }
    let mut bar = GradientStrip::new(x, y, width);
    bar.height = 4.0 * s;
    bar.colors = palette.file_manager_gradient_stops();
    bar.draw(painter);
}

pub fn draw_gradient_v(painter: &mut Painter, palette: &FoxPalette, x: f32, y: f32, height: f32, s: f32) {
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
        let sh = if i + 1 == segments { y + height - sy } else { step };
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

pub fn draw_scrollbar(
    painter: &mut Painter,
    scrollbar: &Scrollbar,
    state: InteractionState,
    palette: &FoxPalette,
) {
    scrollbar.draw(painter, state, palette);
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
                segments.push((comp.as_os_str().to_string_lossy().to_string(), accum.clone()));
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

pub fn wrap_lines(name: &str, max_w: f32, char_w: f32) -> Vec<String> {
    let max_chars = (max_w / char_w).floor().max(1.0) as usize;
    let chars: Vec<char> = name.chars().collect();
    if chars.len() <= max_chars {
        return vec![name.to_string()];
    }
    let mut lines = Vec::new();
    let mut start = 0;
    while start < chars.len() {
        let end = (start + max_chars).min(chars.len());
        lines.push(chars[start..end].iter().collect());
        start = end;
    }
    lines
}

pub fn truncate_with_ellipsis(name: &str, max_w: f32, char_w: f32) -> String {
    let est_w = name.len() as f32 * char_w;
    if est_w <= max_w {
        return name.to_string();
    }
    let ellipsis_w = 3.0 * char_w; // "…"
    let max_chars = ((max_w - ellipsis_w) / char_w).floor().max(1.0) as usize;
    let truncated: String = name.chars().take(max_chars).collect();
    format!("{truncated}\u{2026}")
}

/// Truncate `name` to fit `max_w` pixels using the renderer's *actual* glyph
/// measurements (cached), with a trailing ellipsis. The char-width estimate in
/// `truncate_with_ellipsis` underestimates wide glyphs (m, w, …), which lets the
/// leftover wrap onto a second line; measuring is exact. Binary-searches the
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

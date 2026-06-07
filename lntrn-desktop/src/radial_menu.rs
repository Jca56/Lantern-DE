//! Desktop right-click *radial* menu — a ring of circular icon buttons that
//! blooms around the cursor. Opened by right-pressing empty desktop space;
//! the gesture is "hold → drag to a button → release to fire", with a quick
//! tap latching the ring open for point-and-click.
//!
//! All geometry lives in **logical pixels** (the same space input events use)
//! and is multiplied by the fractional `scale` only at draw time — exactly the
//! convention the rest of lntrn-desktop follows.

use std::collections::HashMap;
use std::f32::consts::{FRAC_PI_2, TAU};
use std::time::Instant;

use lntrn_render::{Color, GpuContext, GpuTexture, Painter, Rect, TextRenderer, TextureDraw, TexturePass};

// ── Geometry (logical px) ─────────────────────────────────────────────────────

/// Orbit radius of the button centers around the cursor. Bigger = buttons sit
/// farther from the hub and spread out more (button size is fixed by BUTTON_R).
const RING_RADIUS: f32 = 184.0;
/// Radius of each circular button.
const BUTTON_R: f32 = 46.0;
/// Radius of the little center hub dot.
const HUB_R: f32 = 16.0;
/// Label text size (logical). Big, per the user's eyesight preference.
const LABEL_SIZE: f32 = 18.0;
/// Extra slop added to the button radius when hit-testing, so the ring feels
/// forgiving to click.
const HIT_SLOP: f32 = 8.0;

/// Bloom-in animation length (seconds).
const BLOOM_SECS: f32 = 0.2;
/// A right-press shorter than this with little movement latches the ring open
/// instead of dismissing it (tap-to-stay).
pub const TAP_LATCH_MS: u128 = 350;
/// Movement (logical px) beyond which a press counts as a drag, not a tap.
pub const TAP_MOVE_THRESHOLD: f32 = 12.0;

// ── Items ─────────────────────────────────────────────────────────────────────

/// What a ring button does when fired. Loaded from config (see `radial_config`),
/// so the set of actions stays small and data-driven — `Launch` covers every
/// app/command, while the two desktop-native actions are built in.
#[derive(Clone, PartialEq, Eq)]
pub enum RadialAction {
    /// Run a command (program + optional whitespace-separated args), cwd = desktop.
    Launch(String),
    /// Create a new folder at the ring's anchor point.
    NewFolder,
    /// Re-scan the desktop directory.
    Refresh,
}

/// One button in the ring. Owned strings so the contents can come from a config
/// file the user edits, not just a compiled-in table.
#[derive(Clone)]
pub struct RadialItem {
    pub action: RadialAction,
    pub label: String,
    pub icon: String,
}

// ── State ─────────────────────────────────────────────────────────────────────

pub struct RadialMenuState {
    /// Ring center (logical px) — also the visual origin of the bloom.
    pub cx: f32,
    pub cy: f32,
    /// Currently hovered button index, if any.
    pub hover: Option<usize>,
    /// True once the ring is "stuck" open for point-and-click (after a tap).
    pub latched: bool,
    /// When the ring opened — drives the bloom animation.
    pub opened_at: Instant,
    /// Cursor position at open time — used to tell a tap from a drag.
    pub press_x: f32,
    pub press_y: f32,
}

impl RadialMenuState {
    pub fn open(cx: f32, cy: f32) -> Self {
        Self {
            cx,
            cy,
            hover: None,
            latched: false,
            opened_at: Instant::now(),
            press_x: cx,
            press_y: cy,
        }
    }

    /// True while the open animation is still playing (so the main loop keeps
    /// the frame pump alive even if the cursor is perfectly still).
    pub fn needs_anim(&self) -> bool {
        self.opened_at.elapsed().as_secs_f32() < BLOOM_SECS + 0.03
    }

    /// Half-extent of the whole widget (logical px) including labels — used to
    /// clamp the center so the ring stays on-screen.
    pub fn extent() -> f32 {
        RING_RADIUS + BUTTON_R + LABEL_SIZE + 32.0
    }

    /// Clamp the ring *center* so the whole widget stays within the surface.
    /// Leaves `press_x/press_y` untouched — those track the real cursor for the
    /// tap-vs-drag test, which must not be skewed by edge clamping. Also called
    /// on output resize so an open ring re-fits a shrunk surface.
    pub fn clamp_to_surface(&mut self, surface_w: f32, surface_h: f32) {
        let ext = Self::extent();
        self.cx = clamp_axis(self.cx, ext, surface_w);
        self.cy = clamp_axis(self.cy, ext, surface_h);
    }
}

/// Clamp a center coordinate so an `ext` half-extent stays inside `span`;
/// falls back to centering when the surface is too small for the whole ring.
fn clamp_axis(v: f32, ext: f32, span: f32) -> f32 {
    let lo = ext.min(span * 0.5);
    let hi = (span - ext).max(span * 0.5);
    v.clamp(lo, hi)
}

/// Center (logical or physical, depending on the radius passed) of button `i`.
/// Angles start at the top and advance clockwise.
fn button_center(cx: f32, cy: f32, i: usize, n: usize, radius: f32) -> (f32, f32) {
    let step = TAU / n as f32;
    let a = -FRAC_PI_2 + i as f32 * step;
    (cx + radius * a.cos(), cy + radius * a.sin())
}

/// Hit-test the cursor (logical px) against the settled button positions.
/// Returns the hovered button index, or None for the dead zone / outside.
pub fn hit(st: &RadialMenuState, items: &[RadialItem], mx: f32, my: f32) -> Option<usize> {
    let n = items.len();
    if n == 0 {
        return None;
    }
    let r2 = (BUTTON_R + HIT_SLOP).powi(2);
    for i in 0..n {
        let (bx, by) = button_center(st.cx, st.cy, i, n, RING_RADIUS);
        if (mx - bx).powi(2) + (my - by).powi(2) <= r2 {
            return Some(i);
        }
    }
    None
}

// ── Theme colors ──────────────────────────────────────────────────────────────

struct RadialColors {
    accent: Color,
    surface: Color,
    text: Color,
    bg: Color,
}

impl RadialColors {
    /// Pull live colors from the user's active Lantern theme + accent override.
    fn current() -> Self {
        let conv = |c: lntrn_theme::Rgba| Color::from_rgba8(c.r, c.g, c.b, c.a);
        let variant = lntrn_theme::active_variant();
        let p = variant.palette();
        let accent = lntrn_theme::active_accent().unwrap_or_else(|| variant.accent());
        Self {
            accent: conv(accent),
            surface: conv(p.surface_2),
            text: conv(p.text),
            bg: conv(p.bg),
        }
    }
}

// ── Icon cache ────────────────────────────────────────────────────────────────

/// Rasterizes the ring's embedded SVG icons (via `lntrn_icons`) into GPU
/// textures, reusing the desktop's existing resvg pipeline.
pub struct RadialIconCache {
    textures: HashMap<String, GpuTexture>,
    size_px: u32,
}

impl RadialIconCache {
    pub fn new(size_px: u32) -> Self {
        Self { textures: HashMap::new(), size_px: size_px.max(24) }
    }

    /// Drop all textures (e.g. on output-scale change) and re-target a new size.
    pub fn clear(&mut self, size_px: u32) {
        self.textures.clear();
        self.size_px = size_px.max(24);
    }

    fn ensure(&mut self, gpu: &GpuContext, tex_pass: &TexturePass, name: &str) {
        if self.textures.contains_key(name) {
            return;
        }
        let Some(bytes) = lntrn_icons::get(name) else {
            return;
        };
        if let Some(rgba) = crate::assets::rasterize_svg(bytes, self.size_px) {
            let tex = tex_pass.upload(gpu, &rgba, self.size_px, self.size_px);
            self.textures.insert(name.to_string(), tex);
        }
    }

    fn get(&self, name: &str) -> Option<&GpuTexture> {
        self.textures.get(name)
    }
}

// ── Easing (inlined — we build our own; no lntrn-ui dependency) ────────────────

fn clamp01(t: f32) -> f32 {
    t.clamp(0.0, 1.0)
}
fn progress(elapsed: f32, dur: f32) -> f32 {
    if dur <= 0.0 { 1.0 } else { (elapsed / dur).clamp(0.0, 1.0) }
}
fn ease_out(t: f32) -> f32 {
    let inv = 1.0 - clamp01(t);
    1.0 - inv * inv * inv
}
/// Cubic "back" ease-out — overshoots above 1.0 then settles, for a spring pop.
fn ease_out_back(t: f32) -> f32 {
    let t = clamp01(t);
    let c1 = 1.70158_f32;
    let c3 = c1 + 1.0;
    1.0 + c3 * (t - 1.0).powi(3) + c1 * (t - 1.0).powi(2)
}
fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

// ── Draw ──────────────────────────────────────────────────────────────────────

/// Draw the radial menu as an overlay. Queues ring/button shapes into `painter`
/// and labels into `text`, and returns the icon `TextureDraw`s for the caller
/// to flush in a texture pass *after* the painter overlay pass.
///
/// Everything is positioned in physical pixels (logical * `scale`). The bloom
/// animates the orbit + button radii outward with a spring overshoot and fades
/// the whole thing in.
pub fn draw_radial_menu<'a>(
    painter: &mut Painter,
    text: &mut TextRenderer,
    icons: &'a mut RadialIconCache,
    gpu: &GpuContext,
    tex_pass: &TexturePass,
    st: &RadialMenuState,
    items: &[RadialItem],
    scale: f32,
    surface_w: u32,
    surface_h: u32,
) -> Vec<TextureDraw<'a>> {
    let cols = RadialColors::current();
    let n = items.len();
    if n == 0 {
        return Vec::new();
    }

    let elapsed = st.opened_at.elapsed().as_secs_f32();
    let t = progress(elapsed, BLOOM_SECS);
    let pop = ease_out_back(t); // spring overshoot, for radii
    let fade = ease_out(t); // 0..1 alpha

    let cx = st.cx * scale;
    let cy = st.cy * scale;
    // Orbit + button radii grow from a smaller start with a subtle overshoot.
    let rr = RING_RADIUS * scale * lerp(0.35, 1.0, pop);
    let br = BUTTON_R * scale * lerp(0.55, 1.0, pop);
    let hub = HUB_R * scale * fade;

    // Soft focus disc behind the ring — dims the wallpaper/icons so the menu pops.
    let disc_r = rr + br + 44.0 * scale;
    painter.circle_filled(cx, cy, disc_r, cols.bg.with_alpha(0.18 * fade));

    // The ring track the buttons sit on.
    painter.circle_stroke(cx, cy, rr, 3.0 * scale, cols.accent.with_alpha(0.45 * fade));

    // Center hub marking the cursor origin.
    painter.circle_filled(cx, cy, hub, cols.surface.with_alpha(0.95 * fade));
    painter.circle_stroke(cx, cy, hub, 2.0 * scale, cols.accent.with_alpha(0.85 * fade));

    // Pass 1: make sure every icon texture is rasterized (mutable borrow).
    for it in items {
        icons.ensure(gpu, tex_pass, &it.icon);
    }

    // Pass 2: queue shapes + labels, collect icon draws (immutable borrow).
    let mut draws: Vec<TextureDraw<'a>> = Vec::with_capacity(n);
    let shadow = Color::from_rgba8(0, 0, 0, 255);
    for (i, it) in items.iter().enumerate() {
        let (bx, by) = button_center(cx, cy, i, n, rr);
        let hovered = st.hover == Some(i);
        let r = if hovered { br * 1.16 } else { br };

        // Drop shadow.
        painter.circle_filled(bx, by + 3.0 * scale, r, shadow.with_alpha(0.30 * fade));
        // Hover glow halo.
        if hovered {
            painter.circle_filled(bx, by, r + 9.0 * scale, cols.accent.with_alpha(0.30 * fade));
        }
        // Button base.
        let base = if hovered { cols.accent } else { cols.surface };
        let base_a = if hovered { 0.98 } else { 0.92 };
        painter.circle_filled(bx, by, r, base.with_alpha(base_a * fade));
        // Rim.
        let rim_a = if hovered { 0.95 } else { 0.5 };
        painter.circle_stroke(bx, by, r, 2.0 * scale, cols.accent.with_alpha(rim_a * fade));

        // Icon centered in the button.
        if let Some(tex) = icons.get(&it.icon) {
            let isz = r * 1.16;
            draws.push(
                TextureDraw::new(tex, bx - isz * 0.5, by - isz * 0.5, isz, isz).opacity(fade),
            );
        }

        // Label pill below the button.
        let fsz = LABEL_SIZE * scale;
        let tw = text.measure_width(&it.label, fsz);
        let pill_h = fsz + 10.0 * scale;
        let pill_w = tw + 18.0 * scale;
        let ly = by + r + 10.0 * scale;
        let pill_x = bx - pill_w * 0.5;
        let (pill_col, txt_col) = if hovered {
            (cols.accent.with_alpha(0.95 * fade), cols.bg)
        } else {
            (cols.bg.with_alpha(0.62 * fade), cols.text)
        };
        painter.rect_filled(Rect::new(pill_x, ly, pill_w, pill_h), pill_h * 0.5, pill_col);
        text.queue(
            &it.label,
            fsz,
            bx - tw * 0.5,
            ly + 5.0 * scale,
            txt_col.with_alpha(fade),
            surface_w as f32,
            surface_w,
            surface_h,
        );
    }

    draws
}

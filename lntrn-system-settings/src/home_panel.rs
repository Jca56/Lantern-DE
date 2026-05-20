//! Home (landing) page. Stays intentionally minimal for now — just shows the
//! Lantern logo + a "Favorites coming soon" caption — so we have a place to
//! land before the favorites grid lands.
//!
//! Rasterizes the bundled lantern SVG once into a GPU texture and re-uses it
//! across frames. The icon scales with window size so it stays readable on
//! both the laptop (single ~1080p panel) and the desktop (2560×1440 main).

use lntrn_render::{GpuContext, GpuTexture, Painter, TextRenderer, TexturePass, TextureDraw};
use lntrn_ui::gpu::FoxPalette;

const LANTERN_SVG: &[u8] = include_bytes!("../../icons/apps/lntrn.svg");

pub(crate) struct HomeState {
    /// `(texture, rendered_size_px)` — re-rasterized only when the target size
    /// changes by more than a few pixels.
    icon: Option<(GpuTexture, u32)>,
}

impl HomeState {
    pub fn new() -> Self { Self { icon: None } }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_home_panel<'a>(
    state: &'a mut HomeState,
    _painter: &mut Painter,
    text: &mut TextRenderer,
    tex_pass: &TexturePass,
    gpu: &GpuContext,
    fox: &FoxPalette,
    tex_draws: &mut Vec<TextureDraw<'a>>,
    x: f32, y: f32, w: f32, panel_h: f32, s: f32, sw: u32, sh: u32,
) {
    // Target lantern size: ~48% of the smaller content dimension, capped so it
    // doesn't dwarf the page on huge monitors.
    let target_px = (w.min(panel_h) * 0.48).clamp(240.0 * s, 520.0 * s);
    let target_px_u = target_px.round().max(1.0) as u32;

    // Re-rasterize only if the cached texture is missing or noticeably stale.
    let needs_rebuild = match state.icon {
        Some((_, sz)) => (sz as i32 - target_px_u as i32).abs() > 8,
        None => true,
    };
    if needs_rebuild {
        if let Some(tex) = rasterize_lantern(tex_pass, gpu, target_px_u) {
            state.icon = Some((tex, target_px_u));
        }
    }

    if let Some((tex, _)) = state.icon.as_ref() {
        let icon_x = x + (w - target_px) * 0.5;
        let icon_y = y + (panel_h - target_px) * 0.5 - 30.0 * s;
        tex_draws.push(TextureDraw::new(tex, icon_x, icon_y, target_px, target_px));

        // Caption under the lantern, approximately centered.
        let caption = "Favorites coming soon";
        let cap_size = 22.0 * s;
        let cap_w_est = caption.chars().count() as f32 * cap_size * 0.55;
        let cap_x = x + (w - cap_w_est).max(0.0) * 0.5;
        let cap_y = icon_y + target_px + 24.0 * s;
        text.queue(caption, cap_size, cap_x, cap_y, fox.muted, cap_w_est, sw, sh);
    }
}

fn rasterize_lantern(tex_pass: &TexturePass, gpu: &GpuContext, sz: u32) -> Option<GpuTexture> {
    let tree = resvg::usvg::Tree::from_data(LANTERN_SVG, &Default::default()).ok()?;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(sz, sz)?;
    let sx = sz as f32 / tree.size().width();
    let sy = sz as f32 / tree.size().height();
    let scale = sx.min(sy);
    let tx = (sz as f32 - tree.size().width()  * scale) * 0.5;
    let ty = (sz as f32 - tree.size().height() * scale) * 0.5;
    let xf = resvg::tiny_skia::Transform::from_scale(scale, scale).post_translate(tx, ty);
    resvg::render(&tree, xf, &mut pixmap.as_mut());
    Some(tex_pass.upload(gpu, pixmap.data(), sz, sz))
}

//! Shared runtime for rice windows: ClientSide-decoration toplevel that draws
//! whatever the scene closure asks for. ESC closes, drag-anywhere moves,
//! edge-resize works, corners are rounded per `[window_manager].corner_radius`.

mod app;
mod dispatch;

pub use app::{run, AppConfig, FrameCtx};

pub use lntrn_render::{Color, Painter, Rect};

/// Solid theme background — `[appearance].background_color` (or active variant
/// bg) tinted by `[windows].background_opacity`. Honors corner radius.
pub fn draw_theme_background(painter: &mut Painter, ctx: &FrameCtx) {
    let opacity = lntrn_theme::background_opacity();
    let rgba = lntrn_theme::active_background_color()
        .unwrap_or_else(|| lntrn_theme::active_variant().palette().bg);
    let color = Color::from_rgba8(rgba.r, rgba.g, rgba.b, rgba.a).with_alpha(opacity);
    painter.rect_filled(ctx.window_rect(), ctx.corner_radius, color);
}

/// Pull the active theme accent (`[appearance].accent` or variant default).
pub fn theme_accent() -> Color {
    let rgba = lntrn_theme::active_accent()
        .unwrap_or_else(|| lntrn_theme::active_variant().accent());
    Color::from_rgba8(rgba.r, rgba.g, rgba.b, rgba.a)
}

/// Active variant's base bg as a `Color` (alpha from variant, not opacity-tinted).
pub fn theme_bg() -> Color {
    let rgba = lntrn_theme::active_variant().palette().bg;
    Color::from_rgba8(rgba.r, rgba.g, rgba.b, rgba.a)
}

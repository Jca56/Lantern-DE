//! Panel chrome rendering.
//!
//! Phase 1: glassy panel chrome + open/close animation.
//! Phase 2.1: also draws the search input row.
//!
//! The animation is applied entirely in shader-space: the wl_surface
//! never resizes. We scale the panel rect around its own center and
//! modulate alpha by the same eased factor.

use lntrn_render::{Color, Painter, Rect, TextRenderer};

use crate::app::{AppState, PanelRect, ANIM_SCALE_START, PANEL_CORNER_RADIUS};

/// Panel surface color — Fox Dark `bg` from lntrn-terminal/src/theme.rs:29
/// (#181818, rgb 24,24,24). Keeping the whole DE visually consistent.
///
/// IMPORTANT: the wgpu surface format is `Bgra8UnormSrgb`, so the GPU
/// applies sRGB gamma encoding at output. That means values written to
/// the framebuffer are interpreted as **linear**, not sRGB. We use
/// `Color::from_rgb8` which does the sRGB→linear conversion for us;
/// passing raw `Color::rgba(0.094, ...)` would treat 0.094 as linear and
/// display way too bright (about #5d5d5d on screen).
const SURFACE_BYTES: (u8, u8, u8) = (24, 24, 24);
/// Surface alpha when fully open. Fully opaque — matches the terminal.
const SURFACE_ALPHA: f32 = 0.92;

/// Result of `draw_panel` — the (animated) panel rect and alpha, so
/// content layers can position themselves over the same region.
pub struct PanelDraw {
    pub rect: Rect,
    pub alpha: f32,
    pub scale_factor: f32,
}

/// One pending icon-render request. Drawing collects these as it walks
/// the layout; the render loop then turns each into a `TextureDraw` by
/// asking the `IconCache` for the actual texture (loading on miss).
///
/// We split it this way so launcher/search modules don't need to touch
/// the GPU directly — they just push owned `IconRequest`s into a Vec.
#[derive(Debug, Clone)]
pub struct IconRequest {
    pub app_id: String,
    pub icon_name: Option<String>,
    pub x: f32,
    pub y: f32,
    pub size: f32,
    pub opacity: f32,
    /// Optional scissor `[x, y, w, h]` in physical pixels. Used by the
    /// scrollable grid so icons crossing the viewport edge get clipped
    /// instead of bleeding into the search bar / panel chrome.
    pub clip: Option<[f32; 4]>,
}

/// Draw the Command Center chrome at the current animation state.
///
/// Caller is responsible for clearing the surface to transparent before
/// calling this — we draw only the panel, not the fullscreen backdrop.
pub fn draw_panel(
    painter: &mut Painter,
    state: &AppState,
    surface_w: u32,
    scale: f32,
) -> Option<PanelDraw> {
    // Caller is responsible for `painter.clear()` at the start of the
    // frame. We just queue draws.
    let factor = state.anim_factor();
    if factor <= 0.0 {
        return None;
    }

    let alpha = factor;
    // Scale: ANIM_SCALE_START → 1.0 as factor goes 0 → 1.
    let s = ANIM_SCALE_START + (1.0 - ANIM_SCALE_START) * factor;

    let base = PanelRect::compute_with_height(surface_w, scale, state.desired_panel_h_logical());

    // Scale the rect around its own center so it grows from the middle.
    let cx = base.x + base.w / 2.0;
    let cy = base.y + base.h / 2.0;
    let scaled_w = base.w * s;
    let scaled_h = base.h * s;
    let rect = Rect::new(
        cx - scaled_w / 2.0,
        cy - scaled_h / 2.0,
        scaled_w,
        scaled_h,
    );

    let radius = PANEL_CORNER_RADIUS * scale;

    // Drop shadow (pure black, soft, slightly offset down).
    let shadow_alpha = 0.35 * alpha;
    painter.shadow(
        rect,
        radius,
        24.0 * scale,
        Color::BLACK.with_alpha(shadow_alpha),
        0.0,
        4.0 * scale,
    );

    // Panel surface (matches Fox Dark terminal bg, sRGB-aware).
    let (sr, sg, sb) = SURFACE_BYTES;
    let surface = Color::from_rgb8(sr, sg, sb).with_alpha(SURFACE_ALPHA * alpha);
    painter.rect_filled(rect, radius, surface);

    // No border — keeps the panel reading as a single soft slab.

    // Scale the *content* by the same animation factor as the chrome,
    // so search/launcher/controls all grow from the panel center together.
    Some(PanelDraw {
        rect,
        alpha,
        scale_factor: scale * s,
    })
}

/// Draw all panel content. Called only when `draw_panel` returned `Some`.
/// Returns a Vec of icon requests that the render loop turns into
/// `TextureDraw`s.
///
/// Layout: the controls row + underline always sits at the top. Below
/// it, exactly one of two views fills the rest of the panel:
/// - `PanelMode::Launcher` → search input + pinned/results
/// - `PanelMode::Control(id)` → that control's full-content view
pub fn draw_content(
    painter: &mut Painter,
    text: &mut TextRenderer,
    state: &AppState,
    panel: &PanelDraw,
    surface_w: u32,
    surface_h: u32,
) -> Vec<IconRequest> {
    let mut icons = Vec::new();

    // Highlight the tile whose view is currently showing.
    let selected_tile = match state.mode {
        crate::app::PanelMode::Control(id) => Some(id),
        crate::app::PanelMode::Launcher => None,
    };

    // 1. Controls row + underline (always).
    crate::controls::draw_row(
        painter,
        text,
        &state.controls,
        selected_tile,
        panel.rect,
        panel.scale_factor,
        panel.alpha,
        surface_w,
        surface_h,
        &mut icons,
    );

    // 1b. Power column floating to the right of the panel. Fades with
    //     the collapse animation so the side rail disappears at the
    //     same rate as the body content.
    let collapse_p = state.collapse_progress();
    let chrome_alpha_mult = (1.0 - collapse_p).clamp(0.0, 1.0);
    if chrome_alpha_mult > 0.005 {
        crate::power::draw(
            painter,
            &mut icons,
            panel.rect,
            panel.scale_factor,
            panel.alpha * chrome_alpha_mult,
            state.power_hover,
        );
    }

    // 1d. Mini-dock of pinned apps under the panel — visible while
    //     collapsed (or fading in/out of collapse). Lives outside the
    //     main panel rect so it stays a single click away when the bar
    //     is in tiny mode.
    {
        let dock_alpha_mult = collapse_p.clamp(0.0, 1.0);
        if dock_alpha_mult > 0.005 {
            let pinned = state.launcher.pinned_entries(&state.apps);
            crate::mini_dock::draw(
                painter,
                &mut icons,
                &pinned,
                panel.rect,
                panel.scale_factor,
                panel.alpha * dock_alpha_mult,
                state.mini_dock_hover,
                &state.apps,
            );
        }
    }

    // 1c. Collapse chevron (top-right of the row). Drawn separately so
    //     it can read the `collapsed` state from AppState.
    if let Some(layout) = state.controls.tile_layout(
        crate::controls::TileId::Collapse,
        panel.rect,
        panel.scale_factor,
    ) {
        crate::controls::collapse::draw_inline(
            painter,
            text,
            state.collapsed,
            &layout,
            panel.scale_factor,
            panel.alpha,
        );
    }

    // 2. Body of the panel, based on mode. Faded out during collapse
    //    so the content gracefully disappears as the panel shrinks
    //    (and back in on expand).
    if chrome_alpha_mult < 0.005 {
        return icons;
    }
    let body_panel = PanelDraw {
        rect: panel.rect,
        alpha: panel.alpha * chrome_alpha_mult,
        scale_factor: panel.scale_factor,
    };
    let panel = &body_panel;
    match state.mode {
        crate::app::PanelMode::Launcher => {
            crate::search::draw_input(
                painter,
                text,
                &state.search,
                panel.rect,
                panel.scale_factor,
                panel.alpha,
                surface_w,
                surface_h,
            );

            let selected_pin = match state.selection {
                crate::app::Selection::Pin(i) => Some(i),
                _ => None,
            };
            let selected_result = match state.selection {
                crate::app::Selection::Result(i) => Some(i),
                _ => None,
            };

            if state.search.input.is_empty() && !state.search.all_apps_mode {
                let top_y = crate::search::content_top_y(panel.rect, panel.scale_factor);
                let pins_bottom = crate::launcher::draw(
                    painter,
                    text,
                    &mut icons,
                    &state.launcher,
                    &state.apps,
                    selected_pin,
                    panel.rect,
                    top_y,
                    panel.scale_factor,
                    panel.alpha,
                    surface_w,
                    surface_h,
                );
                crate::launcher::open::draw(
                    painter,
                    text,
                    &mut icons,
                    &state.toplevels,
                    &state.apps,
                    None,
                    panel.rect,
                    pins_bottom,
                    panel.scale_factor,
                    panel.alpha,
                    surface_w,
                    surface_h,
                );

                // Pin drag overlay (ghost + drop indicator) — drawn over
                // the regular pinned row, under the open section.
                if let Some(drag) = state.pin_drag.as_ref() {
                    let row_top = crate::launcher::pins_row_top_y(top_y, panel.scale_factor);
                    crate::launcher::draw_pin_drag_overlay(
                        painter,
                        &mut icons,
                        &state.launcher,
                        &state.apps,
                        drag,
                        panel.rect,
                        row_top,
                        panel.scale_factor,
                        panel.alpha,
                    );
                }
            } else {
                crate::search::draw_results(
                    painter,
                    text,
                    &mut icons,
                    &state.search,
                    &state.apps,
                    selected_result,
                    panel.rect,
                    panel.scale_factor,
                    panel.alpha,
                    surface_w,
                    surface_h,
                );
            }
        }
        crate::app::PanelMode::Control(tile_id) => {
            crate::controls::draw_view(
                painter,
                text,
                &state.controls,
                tile_id,
                panel.rect,
                panel.scale_factor,
                panel.alpha,
                surface_w,
                surface_h,
            );
        }
    }

    // 3. Right-click context menu — drawn on layer 1 so it sits above
    //    grid tiles, labels, and any other panel content.
    if let Some(menu) = &state.context_menu {
        painter.set_layer(1);
        text.set_layer(1);
        crate::launcher::context_menu::draw(
            painter,
            text,
            menu,
            panel.rect,
            panel.scale_factor,
            surface_w,
            surface_h,
        );
        painter.set_layer(0);
        text.set_layer(0);
    }

    // 4. Calendar event context menu — same overlay layer so it sits
    //    above the day-detail panel.
    if let Some(menu) = &state.controls.clock.event_menu {
        painter.set_layer(1);
        text.set_layer(1);
        crate::controls::clock::draw_event_menu(
            painter,
            text,
            menu,
            panel.rect,
            panel.scale_factor,
            surface_w,
            surface_h,
        );
        painter.set_layer(0);
        text.set_layer(0);
    }

    // 5. Power confirm modal — overlay so it sits above everything.
    if let Some(action) = state.power_confirm {
        painter.set_layer(1);
        text.set_layer(1);
        crate::power::draw_confirm(
            painter,
            text,
            &mut icons,
            action,
            surface_w,
            surface_h,
            panel.scale_factor,
            panel.alpha,
        );
        painter.set_layer(0);
        text.set_layer(0);
    }

    icons
}

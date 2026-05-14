//! Per-frame render pass — draws the panel, dispatches icon-cache
//! prefetches, streams thumb-slot rects to the compositor, presents the
//! GPU frame, and schedules the next frame callback.

use lntrn_render::{
    Color, GpuContext, Painter, SurfaceError, TextRenderer, TextureDraw, TexturePass,
};
use wayland_client::{protocol::wl_surface, QueueHandle};
use wayland_protocols::wp::viewporter::client::wp_viewport;

use super::util::commit_transparent;
use super::WlState;
use crate::app::AppState;
use crate::launcher::icons::IconCache;

#[allow(clippy::too_many_arguments)]
pub(super) fn render_frame(
    wl: &mut WlState,
    app: &mut AppState,
    gpu: &mut GpuContext,
    surface: &wl_surface::WlSurface,
    viewport: &Option<wp_viewport::WpViewport>,
    painter: &mut Painter,
    text: &mut TextRenderer,
    mono_text: &mut TextRenderer,
    thumbs: &mut crate::thumbs::CcThumbsClient,
    icon_cache: &mut IconCache,
    tex_pass: &TexturePass,
    qh: &QueueHandle<WlState>,
    scale_f: f32,
) {
    if wl.configured {
        wl.configured = false;
        gpu.resize(wl.phys_width().max(1), wl.phys_height().max(1));
        surface.set_buffer_scale(1);
        if let Some(vp) = &viewport {
            vp.set_destination(wl.width as i32, wl.height as i32);
        }
    }

    let phys_w = wl.phys_width().max(1);
    let phys_h = wl.phys_height().max(1);

    // Reset both render queues at the start of every frame. The text
    // renderer in particular accumulates glyphs across calls; without
    // this, a fully-hidden frame still re-renders the last frame's
    // text on top of the transparent surface, leaving a ghost visible
    // after the close animation finishes.
    painter.clear();
    text.clear();
    mono_text.clear();

    let panel_draw = crate::render::draw_panel(painter, &app, phys_w, scale_f);
    let icon_requests = if let Some(p) = &panel_draw {
        crate::render::draw_content(
            painter, text, mono_text, &app, p, phys_w, phys_h,
        )
    } else {
        Vec::new()
    };

    // Stream thumbnail slots to the compositor so it can paint live
    // window content into each Open-section tile. Sent only when the
    // Open section is actually being drawn — i.e. fully visible,
    // Launcher mode, and the empty/non-all-apps search state that
    // render.rs uses to draw the section. Otherwise the compositor
    // keeps painting thumbnails at orphaned rects.
    // Active whenever Default is being displayed in some form —
    // either as the resting view OR as the from/to of an in-flight
    // slide. The slide-x for the rect is computed below.
    let default_in_view = app.panel_view == crate::app::PanelView::Default
        || match app.view_slide() {
            Some(s) => s.from == crate::app::PanelView::Default
                || s.to == crate::app::PanelView::Default,
            None => false,
        };
    let open_section_active = matches!(app.mode, crate::app::PanelMode::Launcher)
        && app.search.input.is_empty()
        && !app.search.all_apps_mode
        && !app.collapsed
        && !app.collapse_animating()
        // Skip thumbs during a view-slide so the compositor doesn't
        // keep painting window previews that have travelled past the
        // panel edge into empty space.
        && !app.view_animating()
        && default_in_view
        && !app.settings_open
        && !app.emojis.open
        && !app.clipboard.open
        && !app.notes.open
        && !app.usage.open;
    if let Some(p) = &panel_draw {
        if matches!(app.visibility, crate::app::Visibility::Visible) && open_section_active {
            let panel_logical = lntrn_render::Rect::new(p.rect.x, p.rect.y, p.rect.w, p.rect.h);
            let pin_top_y = panel_logical.y
                + crate::controls::total_logical_height() * scale_f
                + (crate::search::input::SEARCH_HORIZONTAL_PAD * 0.5
                    + crate::search::input::SEARCH_ROW_HEIGHT)
                    * scale_f;
            let pinned_count = app.launcher.pinned_entries(&app.apps).len();
            let pins_bottom = crate::launcher::pins_section_bottom(
                panel_logical,
                pin_top_y,
                scale_f,
                pinned_count,
            );
            let visible_open = crate::launcher::open::visible_entries(&app.toplevels);
            let row_top = pins_bottom
                + crate::launcher::open::OPEN_SECTION_TOP_MARGIN * scale_f
                + crate::launcher::open::heading_advance(scale_f);

            // If we're mid-slide, shift the thumbnail rects by
            // Default's current slide offset so they glide with
            // the rest of the body instead of popping.
            let default_slide_offset = match app.view_slide() {
                Some(s) if s.from == crate::app::PanelView::Default => s.from_offset,
                Some(s) if s.to == crate::app::PanelView::Default => s.to_offset,
                _ => 0.0,
            };
            let slide_x = default_slide_offset * panel_logical.w;
            let mut slots = Vec::with_capacity(visible_open.len());
            let phys_cx = wl.cursor_x as f32 * scale_f;
            let phys_cy = wl.cursor_y as f32 * scale_f;
            for (i, group) in visible_open.iter().enumerate() {
                let Some(rep) = group.close_target() else { continue };
                let r = crate::launcher::open::tile_rect(panel_logical, row_top, scale_f, i);
                let inv = 1.0 / scale_f;
                let close_btn_r = crate::launcher::open::close_button_rect(
                    panel_logical, row_top, scale_f, i,
                );
                let close_hovered = phys_cx >= close_btn_r.x
                    && phys_cx <= close_btn_r.x + close_btn_r.w
                    && phys_cy >= close_btn_r.y
                    && phys_cy <= close_btn_r.y + close_btn_r.h;
                let close = crate::thumbs::CloseBtn {
                    x: ((close_btn_r.x + slide_x) * inv).round() as i32,
                    y: (close_btn_r.y * inv).round() as i32,
                    w: (close_btn_r.w * inv).round() as i32,
                    h: (close_btn_r.h * inv).round() as i32,
                    hovered: close_hovered,
                };
                slots.push(crate::thumbs::ThumbSlot {
                    app_id: rep.app_id.clone(),
                    title: rep.title.clone(),
                    x: ((r.x + slide_x) * inv).round() as i32,
                    y: (r.y * inv).round() as i32,
                    w: (r.w * inv).round() as i32,
                    h: (r.h * inv).round() as i32,
                    close: Some(close),
                });
            }
            thumbs.update(&slots);
        } else if matches!(app.visibility, crate::app::Visibility::Visible)
            && app.collapse_progress() > 0.5
            && app.mini_dock_hover.is_some()
            && app.panel_view == crate::app::PanelView::Default
            && !app.settings_open
            && !app.emojis.open
            && !app.clipboard.open
            && !app.notes.open
            && !app.usage.open
        {
            // Dock hover preview: one thumbnail slot per window of
            // the hovered dock app, arrayed horizontally.
            let mut slots: Vec<crate::thumbs::ThumbSlot> = Vec::new();
            let panel_logical = lntrn_render::Rect::new(p.rect.x, p.rect.y, p.rect.w, p.rect.h);
            let pinned = app.launcher.pinned_entries(&app.apps);
            let idx = app.mini_dock_hover.unwrap();
            let layout = crate::mini_dock::compute_layout(
                panel_logical,
                phys_h as f32,
                scale_f,
                &pinned,
                &app.toplevels,
                &app.apps,
                Some((
                    wl.cursor_x as f32 * scale_f,
                    wl.cursor_y as f32 * scale_f,
                )),
            );
            let entry = layout.as_ref().and_then(|l| l.entries.get(idx)).cloned();
            if let (Some(layout), Some(entry)) = (layout.as_ref(), entry) {
                let windows = crate::mini_dock::windows_for_app(
                    &app.toplevels, &entry.app_id,
                );
                if !windows.is_empty() {
                    let tiles = crate::mini_dock::preview_tile_rects(
                        layout, panel_logical, idx, windows.len(),
                    );
                    let inv = 1.0 / scale_f;
                    let phys_cx = wl.cursor_x as f32 * scale_f;
                    let phys_cy = wl.cursor_y as f32 * scale_f;
                    for (tile, window) in tiles.iter().zip(windows.iter()) {
                        let close = crate::mini_dock::preview_close_button_rect(*tile, scale_f);
                        let close_hovered = phys_cx >= close.x
                            && phys_cx <= close.x + close.w
                            && phys_cy >= close.y
                            && phys_cy <= close.y + close.h;
                        slots.push(crate::thumbs::ThumbSlot {
                            app_id: window.app_id.clone(),
                            title: window.title.clone(),
                            x: (tile.x * inv).round() as i32,
                            y: (tile.y * inv).round() as i32,
                            w: (tile.w * inv).round() as i32,
                            h: (tile.h * inv).round() as i32,
                            close: Some(crate::thumbs::CloseBtn {
                                x: (close.x * inv).round() as i32,
                                y: (close.y * inv).round() as i32,
                                w: (close.w * inv).round() as i32,
                                h: (close.h * inv).round() as i32,
                                hovered: close_hovered,
                            }),
                        });
                    }
                }
            }
            thumbs.update(&slots);
        } else {
            thumbs.clear();
        }
    } else {
        thumbs.clear();
    }

    // Materialize icon requests into TextureDraws in two phases so
    // we don't ask the borrow checker to juggle &mut + & on the same
    // cache. Phase A: ensure each icon is loaded. Phase B: read-only
    // peek to build the draw list.
    for req in &icon_requests {
        icon_cache.ensure_loaded(&gpu, &tex_pass, &req.app_id, req.icon_name.as_deref());
    }
    let tex_draws: Vec<TextureDraw> = icon_requests
        .iter()
        .filter_map(|req| {
            icon_cache.peek(&req.app_id).map(|tex| {
                let mut d = TextureDraw::new(tex, req.x, req.y, req.size, req.size);
                d.opacity = req.opacity;
                d.clip = req.clip;
                d
            })
        })
        .collect();

    match gpu.begin_frame("CommandCenter") {
        Ok(mut frame) => {
            let view = frame.view().clone();

            // Layered render so modals (BT pair, BT incoming, WiFi
            // password) draw over previously-queued text. Layer 0 is
            // base content; layer 1 is overlays. See
            // lntrn-render/TEXT_OCCLUSION_FIX.md.
            let layers = painter
                .layer_count()
                .max(text.layer_count())
                .max(mono_text.layer_count());

            // Layer 0: base painter, textures, base text, mono text.
            painter.render_layer(
                0,
                &gpu,
                frame.encoder_mut(),
                &view,
                Some(Color::TRANSPARENT),
            );
            if !tex_draws.is_empty() {
                tex_pass.render_pass(&gpu, frame.encoder_mut(), &view, &tex_draws, None);
            }
            text.render_layer(0, &gpu, frame.encoder_mut(), &view);
            mono_text.render_layer(0, &gpu, frame.encoder_mut(), &view);

            // Overlay layers (modals).
            if layers > 1 {
                // Flush so the next layer's text prepare() doesn't
                // stomp on layer-0 vertices still in the queue.
                frame.flush(&gpu);
                for li in 1..layers {
                    painter.render_layer(li, &gpu, frame.encoder_mut(), &view, None);
                    text.render_layer(li, &gpu, frame.encoder_mut(), &view);
                    mono_text.render_layer(li, &gpu, frame.encoder_mut(), &view);
                }
            }

            frame.submit(&gpu.queue);
        }
        Err(SurfaceError::Lost | SurfaceError::Outdated) => {
            gpu.resize(wl.phys_width().max(1), wl.phys_height().max(1));
        }
        Err(_) => {}
    }

    // Schedule the next frame callback while we're still active.
    surface.frame(&qh, ());
    surface.commit();

    // After the close animation finishes, transition to idle. We
    // commit one final transparent frame so the compositor doesn't
    // keep our last visible buffer pinned.
    if app.is_hidden() {
        commit_transparent(gpu, surface);
    }
}

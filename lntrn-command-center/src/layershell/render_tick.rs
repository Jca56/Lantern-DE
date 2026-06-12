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

    // Floating sticky notes render on layer 0 — beneath everything.
    // The panel + outer chrome go on layer 1 (modals on 2), so the
    // Command Center always draws over any sticky it overlaps. The
    // painter/text layer groups render as separate passes below, which
    // is what lets the panel plate genuinely occlude sticky text.
    let sticky_alpha = app.anim_factor();
    let mut drew_stickies = false;
    if sticky_alpha > 0.0 {
        let sw = phys_w as f32;
        let sh = phys_h as f32;
        let hover = if app.sticky_drag.is_none() {
            let panel = crate::app::PanelRect::compute_with_dims(
                phys_w, scale_f, app.desired_panel_w_logical(), app.desired_panel_h_logical(),
            );
            let phys_cx = wl.cursor_x as f32 * scale_f;
            let phys_cy = wl.cursor_y as f32 * scale_f;
            if panel.contains(phys_cx, phys_cy) {
                None
            } else {
                crate::notes::sticky::hit(&app.notes.notes, scale_f, sw, sh, phys_cx, phys_cy)
            }
        } else {
            None
        };
        drew_stickies = crate::notes::sticky::draw(
            painter, text, &app.notes, hover, app.sticky_drag,
            scale_f, app.config.text_size, sticky_alpha, phys_w, phys_h,
        );
    }
    painter.set_layer(1);
    text.set_layer(1);
    mono_text.set_layer(1);

    let panel_draw = crate::render::draw_panel(painter, &app, phys_w, scale_f);
    let icon_requests = if let Some(p) = &panel_draw {
        crate::render::draw_content(
            painter, text, mono_text, app, p, phys_w, phys_h,
        )
    } else {
        Vec::new()
    };

    // (The workspace number now lives inside the Command Center as a
    // chip to the left of the clock — see `controls::clock::draw_inline`
    // — instead of floating outside the panel.)

    // Stream thumbnail slots to the compositor for the mini-dock
    // hover preview only. (The legacy "Open" section on the main page
    // has been retired — its thumbnail streaming logic with it.)
    if let Some(p) = &panel_draw {
        if matches!(app.visibility, crate::app::Visibility::Visible)
            && app.collapse_progress() > 0.5
            && app.mini_dock_hover.is_some()
            && app.panel_view == crate::app::PanelView::Default
            && !app.settings_open
            && !app.emojis.open
            && !app.clipboard.open
            && !app.notes.open
            && !app.usage.open
            && !app.desktop_settings_open
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
                    for (wi, (tile, window)) in tiles.iter().zip(windows.iter()).enumerate() {
                        let close = crate::mini_dock::preview_close_button_rect(*tile, scale_f);
                        let close_hovered = phys_cx >= close.x
                            && phys_cx <= close.x + close.w
                            && phys_cy >= close.y
                            && phys_cy <= close.y + close.h;
                        slots.push(crate::thumbs::ThumbSlot {
                            app_id: window.app_id.clone(),
                            title: window.title.clone(),
                            instance: wi as u32,
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

            // Layered render so the panel draws over sticky notes and
            // modals (BT pair, BT incoming, WiFi password) draw over
            // previously-queued text. Layer 0 is sticky notes; layer 1
            // is the panel + content; layer 2+ is modal overlays. See
            // lntrn-render/TEXT_OCCLUSION_FIX.md.
            let layers = painter
                .layer_count()
                .max(text.layer_count())
                .max(mono_text.layer_count());

            // Layer 0: sticky notes (this pass also clears the frame).
            painter.render_layer(
                0,
                &gpu,
                frame.encoder_mut(),
                &view,
                Some(Color::TRANSPARENT),
            );
            text.render_layer(0, &gpu, frame.encoder_mut(), &view);
            if drew_stickies {
                // Flush so layer 1's text prepare() doesn't stomp the
                // sticky glyph vertices still queued in the shared
                // buffer.
                frame.flush(&gpu);
            }

            // Layer 1: panel plate + content, with icon textures
            // sandwiched between the plates and the text.
            painter.render_layer(1, &gpu, frame.encoder_mut(), &view, None);
            if !tex_draws.is_empty() {
                tex_pass.render_pass(&gpu, frame.encoder_mut(), &view, &tex_draws, None);
            }
            text.render_layer(1, &gpu, frame.encoder_mut(), &view);
            mono_text.render_layer(1, &gpu, frame.encoder_mut(), &view);

            // Overlay layers (modals).
            if layers > 2 {
                // Flush so the next layer's text prepare() doesn't
                // stomp on earlier vertices still in the queue.
                frame.flush(&gpu);
                for li in 2..layers {
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

    // Only pull another frame callback while something is still in
    // motion. When the panel is settled (steady Visible, no view /
    // grow / collapse anim, no chat stream) we commit the rendered
    // buffer and stop scheduling callbacks — the compositor keeps
    // displaying our last buffer and the daemon parks in poll() until
    // input arrives, an IPC command fires, or the idle-tick timeout
    // expires. The main loop's `dispatch_with_timeout` watches both
    // the wayland fd and the IPC fd so we never miss a wake.
    if app.is_animating() {
        surface.frame(&qh, ());
    }
    surface.commit();

    // After the close animation finishes, transition to idle. We
    // commit one final transparent frame so the compositor doesn't
    // keep our last visible buffer pinned.
    if app.is_hidden() {
        commit_transparent(gpu, surface);
    }
}

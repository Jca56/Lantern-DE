//! In-loop drag continuations — slider drags (audio / brightness /
//! settings) and notes-editor text drag-select. Each helper drains the
//! relevant slice of `WlState` state and applies the resulting
//! mutations to `app`. Lives outside `layershell.rs` so the run loop
//! stays a list of named pumps.

use lntrn_render::TextRenderer;

use super::WlState;
use crate::app::{AppState, PanelRect};

/// Drain every in-flight drag for this frame:
///
/// - End any drag whose mouse button was released this frame.
/// - For an active settings-page slider drag, recompute the bound
///   config field from the current cursor x and persist.
/// - For an active audio / brightness slider drag, recompute the
///   volume / brightness from the cursor x.
/// - For an active notes-editor drag (filter / title / body), extend
///   the text selection to the byte under the cursor.
pub(super) fn handle_drag(
    wl: &mut WlState,
    app: &mut AppState,
    text: &mut TextRenderer,
) {
    // End slider drags once the button is released.
    if !wl.left_held {
        app.dragging = None;
        app.settings_drag = None;
    }

    // Settings page slider drag: while the user has a settings slider
    // grabbed, motion events update the bound config field and persist
    // on release.
    if let Some(key) = app.settings_drag {
        let scale_f = wl.fractional_scale() as f32;
        let phys_w = wl.phys_width().max(1);
        let panel = PanelRect::compute_with_dims(
            phys_w, scale_f, app.desired_panel_w_logical(), app.desired_panel_h_logical(),
        );
        let panel_rect = lntrn_render::Rect::new(panel.x, panel.y, panel.w, panel.h);
        let top_y = crate::controls::content_top_y(panel_rect, scale_f);
        let rows = crate::settings::layout(panel_rect, top_y, scale_f);
        let phys_cx = wl.cursor_x as f32 * scale_f;
        if let Some(value) = crate::settings::hit_slider_only(&rows, key, phys_cx) {
            crate::settings::apply_value(
                &mut app.config,
                key,
                crate::settings::SettingValue::F(value),
            );
            app.config.save();
        }
    }

    // Audio / brightness slider drag.
    if let Some(target) = app.dragging {
        let scale_f = wl.fractional_scale() as f32;
        let phys_w = wl.phys_width().max(1);
        let panel = PanelRect::compute_with_dims(phys_w, scale_f, app.desired_panel_w_logical(), app.desired_panel_h_logical());
        let panel_rect = lntrn_render::Rect::new(panel.x, panel.y, panel.w, panel.h);
        let view_top_y = crate::controls::content_top_y(panel_rect, scale_f);
        let phys_cx = wl.cursor_x as f32 * scale_f;

        use crate::app::DragTarget;
        use crate::controls::audio::Direction;
        let track = match target {
            DragTarget::AudioOutputSlider => crate::controls::audio::slider_rect_for(
                panel_rect, view_top_y, Direction::Output, scale_f,
            ),
            DragTarget::AudioInputSlider => crate::controls::audio::slider_rect_for(
                panel_rect, view_top_y, Direction::Input, scale_f,
            ),
            DragTarget::BrightnessSlider => crate::controls::brightness::slider_rect(
                panel_rect, view_top_y, scale_f,
            ),
        };
        let frac = ((phys_cx - track.x) / track.w).clamp(0.0, 1.0);
        match target {
            DragTarget::AudioOutputSlider => app.controls.audio.set_volume(frac),
            DragTarget::AudioInputSlider => app.controls.audio.set_input_volume(frac),
            DragTarget::BrightnessSlider => app.controls.brightness.set_fraction(frac),
        }
    }

    // Notes-editor text drag-select. Runs every frame so the selection
    // keeps extending as the user moves the cursor with the button
    // held.
    if app.notes.open && wl.left_held && app.notes.drag_field.is_some() {
        let scale_f = wl.fractional_scale() as f32;
        let phys_w = wl.phys_width().max(1);
        let panel = PanelRect::compute_with_dims(
            phys_w, scale_f, app.desired_panel_w_logical(), app.desired_panel_h_logical(),
        );
        let panel_rect = lntrn_render::Rect::new(panel.x, panel.y, panel.w, panel.h);
        let top_y = crate::controls::content_top_y(panel_rect, scale_f);
        let panel_bottom = panel_rect.y + panel_rect.h;
        let phys_cx = wl.cursor_x as f32 * scale_f;
        let phys_cy = wl.cursor_y as f32 * scale_f;
        match app.notes.drag_field {
            Some(crate::notes::DragField::Filter) => {
                let bar = crate::notes::filter_rect(panel_rect, top_y, scale_f);
                let pad_left = crate::notes::filter_text_left_pad(bar, scale_f);
                let font = (app.config.text_size * scale_f).max(14.0);
                let q = app.notes.filter.query().to_string();
                let byte = crate::notes::input_byte_at(
                    bar, pad_left, &q, font, phys_cx, text,
                );
                app.notes.filter.drag_to(byte);
            }
            Some(crate::notes::DragField::Title) => {
                let editor = crate::notes::editor_rect(panel_rect, top_y, scale_f, panel_bottom);
                let title_field = crate::notes::title_field_rect(editor, scale_f);
                let title_pad = 12.0 * scale_f;
                let title_font = (app.config.text_size * scale_f).max(15.0);
                let q = app.notes.title.query().to_string();
                let byte = crate::notes::input_byte_at(
                    title_field, title_pad, &q, title_font, phys_cx, text,
                );
                app.notes.title.drag_to(byte);
            }
            Some(crate::notes::DragField::Body) => {
                let editor = crate::notes::editor_rect(panel_rect, top_y, scale_f, panel_bottom);
                let body = crate::notes::body_rect(editor, scale_f);
                let pad = 12.0 * scale_f;
                let inner = lntrn_render::Rect::new(
                    body.x + pad, body.y + pad, body.w - pad * 2.0, body.h - pad * 2.0,
                );
                let buf = app.notes.body.raw().to_string();
                let byte = crate::notes::body_byte_at(
                    inner, &buf, app.notes.body_scroll,
                    app.config.text_size, scale_f, phys_cx, phys_cy, text,
                );
                app.notes.body.drag_to(byte);
            }
            None => {}
        }
    }
    // End the notes drag once the button is released.
    if !wl.left_held && app.notes.drag_field.is_some() {
        app.notes.drag_field = None;
    }
}

/// Terminal body selection: press starts a drag-select, motion extends
/// it, release finalizes it. Consumes `wl.left_clicked` when the press
/// lands on a terminal cell so downstream click handlers don't double-fire.
pub(super) fn handle_terminal_selection(wl: &mut WlState, app: &mut AppState) {
    if app.panel_view != crate::app::PanelView::Terminal {
        return;
    }
    let scale_f = wl.fractional_scale() as f32;
    let phys_w = wl.phys_width().max(1);
    let panel = PanelRect::compute_with_dims(
        phys_w, scale_f, app.desired_panel_w_logical(), app.desired_panel_h_logical(),
    );
    let panel_rect = lntrn_render::Rect::new(panel.x, panel.y, panel.w, panel.h);
    let top_y = crate::controls::content_top_y(panel_rect, scale_f);
    let phys_cx = wl.cursor_x as f32 * scale_f;
    let phys_cy = wl.cursor_y as f32 * scale_f;
    let (g_cols, g_rows) = app
        .terminal
        .grid()
        .map(|g| (g.cols, g.rows))
        .unwrap_or((0, 0));
    let cell = crate::terminal::cell_at(
        panel_rect, top_y, scale_f, app.config.text_size,
        g_cols, g_rows, phys_cx, phys_cy,
    );
    if wl.left_clicked {
        if let Some((row, col)) = cell {
            // Right-click context menu eats the click — only start a
            // fresh selection when the body itself was clicked with
            // the LEFT button.
            if app.context_menu.is_none() {
                app.terminal.begin_selection(row, col);
                wl.left_clicked = false;
            }
        } else {
            // Click outside the terminal body — drop any selection so
            // the highlight doesn't linger after a chrome click.
            app.terminal.clear_selection();
        }
    } else if app.terminal.selecting && wl.left_held {
        if let Some((row, col)) = cell {
            app.terminal.update_selection(row, col);
        }
    }
    if wl.left_released_this_frame && app.terminal.selecting {
        app.terminal.end_selection();
    }
}

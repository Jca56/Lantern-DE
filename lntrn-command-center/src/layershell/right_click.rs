//! Right-click handler — opens context menus appropriate to the
//! current view (launcher pin/unpin, terminal copy/paste, files
//! open/reveal, calendar event delete).

use lntrn_render::TextRenderer;

use super::WlState;
use crate::app::{AppState, PanelRect};

pub(super) fn handle_right_click(
    wl: &mut WlState,
    app: &mut AppState,
    text: &mut TextRenderer,
) {
    if !wl.right_clicked {
        return;
    }
    wl.right_clicked = false;
    let scale_f = wl.fractional_scale() as f32;
    let phys_w = wl.phys_width().max(1);
    let panel = PanelRect::compute_with_dims(phys_w, scale_f, app.desired_panel_w_logical(), app.desired_panel_h_logical());
    let phys_cx = wl.cursor_x as f32 * scale_f;
    let phys_cy = wl.cursor_y as f32 * scale_f;
    match app.mode {
        crate::app::PanelMode::Launcher
        if app.panel_view == crate::app::PanelView::Default =>
        {
        if let Some(target) =
            app.hit_test_launcher(panel, scale_f, phys_cx, phys_cy)
        {
            tracing::debug!(?target, "right-click → open context menu");
            app.open_context_menu_at(target, phys_cx, phys_cy);
        } else {
            app.context_menu = None;
        }
        }
        crate::app::PanelMode::Launcher
        if app.panel_view == crate::app::PanelView::Terminal =>
        {
        // Right-click in the terminal body opens a small
        // Copy / Paste / Clear menu anchored at the cursor.
        let panel_rect =
            lntrn_render::Rect::new(panel.x, panel.y, panel.w, panel.h);
        let top_y = crate::controls::content_top_y(panel_rect, scale_f);
        let (g_cols, g_rows) = app
            .terminal
            .grid()
            .map(|g| (g.cols, g.rows))
            .unwrap_or((0, 0));
        if crate::terminal::cell_at(
            panel_rect, top_y, scale_f, app.config.text_size,
            g_cols, g_rows, phys_cx, phys_cy,
        )
        .is_some()
        {
            let mut items = Vec::new();
            if app.terminal.has_selection() {
            items.push(crate::launcher::context_menu::MenuItem {
                label: "Copy".into(),
                action: crate::launcher::context_menu::MenuAction::TerminalCopy,
            });
            items.push(crate::launcher::context_menu::MenuItem {
                label: "Clear selection".into(),
                action:
                crate::launcher::context_menu::MenuAction::TerminalClearSelection,
            });
            }
            items.push(crate::launcher::context_menu::MenuItem {
            label: "Paste".into(),
            action: crate::launcher::context_menu::MenuAction::TerminalPaste,
            });
            app.context_menu =
            Some(crate::launcher::context_menu::ContextMenu {
                app_id: String::new(),
                window_title: String::new(),
                anchor_x: phys_cx,
                anchor_y: phys_cy,
                items,
            });
        } else {
            app.context_menu = None;
        }
        }
        crate::app::PanelMode::Launcher
        if app.panel_view == crate::app::PanelView::Files =>
        {
        let panel_rect =
            lntrn_render::Rect::new(panel.x, panel.y, panel.w, panel.h);
        let top_y = crate::controls::content_top_y(panel_rect, scale_f);
        if let crate::files::FilesHit::Entry(idx) = crate::files::hit_body(
            &app.files, panel_rect, top_y, scale_f,
            app.config.text_size, phys_cx, phys_cy,
        ) {
            if let Some(entry) = app.files.entry_for_visible(idx) {
            let path_str = entry.path.to_string_lossy().into_owned();
            let items = vec![
                crate::launcher::context_menu::MenuItem {
                label: "Open".into(),
                action: crate::launcher::context_menu::MenuAction::FilesOpen,
                },
                crate::launcher::context_menu::MenuItem {
                label: "Open in Terminal".into(),
                action: crate::launcher::context_menu::MenuAction::FilesOpenInTerminal,
                },
                crate::launcher::context_menu::MenuItem {
                label: "Reveal in File Manager".into(),
                action: crate::launcher::context_menu::MenuAction::FilesRevealInFM,
                },
                crate::launcher::context_menu::MenuItem {
                label: "Copy path".into(),
                action: crate::launcher::context_menu::MenuAction::FilesCopyPath,
                },
            ];
            app.context_menu =
                Some(crate::launcher::context_menu::ContextMenu {
                app_id: path_str,
                window_title: String::new(),
                anchor_x: phys_cx,
                anchor_y: phys_cy,
                items,
                });
            } else {
            app.context_menu = None;
            }
        } else {
            app.context_menu = None;
        }
        }
        crate::app::PanelMode::Launcher => {}
        crate::app::PanelMode::Control(crate::controls::TileId::Clock) => {
        let panel_rect =
            lntrn_render::Rect::new(panel.x, panel.y, panel.w, panel.h);
        let view_top_y = crate::controls::content_top_y(panel_rect, scale_f);
        if app.controls.clock.selected_day.is_some() {
            if let Some(crate::controls::clock::DetailHit::EventRow(idx)) =
            crate::controls::clock::hit_test_detail(
                panel_rect,
                view_top_y,
                scale_f,
                &app.controls.clock,
                &app.controls.events,
                text,
                phys_cx,
                phys_cy,
            )
            {
            if let Some(date) = app.controls.clock.selected_day {
                app.controls.clock.event_menu =
                Some(crate::controls::clock::EventContextMenu {
                    date,
                    idx_in_date: idx,
                    anchor_x: phys_cx,
                    anchor_y: phys_cy,
                });
            }
            } else {
            app.controls.clock.event_menu = None;
            }
        }
        }
        _ => {}
    }
}

use std::time::{Duration, Instant};

use anyhow::Result;
use lntrn_ui::gpu::{
    ContextMenu, FoxPalette, InteractionContext, MenuEvent, ScrollArea,
};
use wayland_client::{
    protocol::wl_surface,
    Connection, EventQueue, QueueHandle,
};
use wayland_protocols::wp::viewporter::client::wp_viewport;

use crate::app::App;
use crate::desktop::DesktopApp;
use crate::icons::IconCache;
use crate::layout::{content_rect, grid_columns, grid_content_height, list_content_height, tree_content_height};
use crate::settings::Settings;
use crate::terminal_panel::TerminalPanel;
use crate::wayland::State;
use crate::wayland_actions::{
    handle_click, handle_ctx_event, handle_key, handle_right_click,
    update_rubber_band,
};
use crate::{
    ClickAction, DesktopPanel, Gpu, ZONE_GLOBAL_TAB_BASE,
    VIEW_SLIDER_ID, VIEW_OPACITY_SLIDER_ID,
};

#[allow(clippy::too_many_arguments)]
pub(crate) fn run_loop(
    _conn: &Connection,
    event_queue: &mut EventQueue<State>,
    state: &mut State,
    qh: &QueueHandle<State>,
    surface: &wl_surface::WlSurface,
    viewport: &Option<wp_viewport::WpViewport>,
    gpu: &mut Gpu,
    palette: &FoxPalette,
    view_menu: &mut ContextMenu,
    context_menu: &mut ContextMenu,
    open_with_apps: &mut Vec<DesktopApp>,
    app: &mut App,
    input: &mut InteractionContext,
    icon_cache: &mut IconCache,
    file_info: &mut crate::file_info::FileInfoCache,
    settings: &mut Settings,
) -> Result<()> {
    let mut last_frame = Instant::now();
    let mut needs_anim = false;
    let mut last_dir_check = Instant::now();
    let mut last_dir_mtime: Option<std::time::SystemTime> = None;
    let mut last_dir_path = app.current_dir.clone();
    let mut last_tab_click: Option<(usize, Instant)> = None;
    // Pinned tab drag reorder state
    let mut tab_drag: Option<usize> = None;
    let mut tab_drag_press: Option<(usize, f32)> = None;
    let mut active_panel = DesktopPanel::Home;
    let mut transition = crate::PanelTransition::new();
    // Create terminal panel with initial size (will resize on first configure)
    let initial_cols = 80;
    let initial_rows = 24;
    let mut terminal_panel = TerminalPanel::new(initial_cols, initial_rows);
    terminal_panel.font_size = settings.term_font_size;
    let panel_file = {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
        std::path::PathBuf::from(home).join(".lantern/config/desktop-panel")
    };
    let _ = std::fs::write(&panel_file, "home");

    eprintln!("[desktop] entering main loop, size={}x{}", state.width, state.height);

    while state.running {
        // Event dispatch
        if needs_anim {
            if let Err(e) = event_queue.flush() {
                eprintln!("[desktop] flush error: {e}");
                break;
            }
            if let Some(guard) = event_queue.prepare_read() {
                let _ = guard.read();
            }
            if let Err(e) = event_queue.dispatch_pending(state) {
                eprintln!("[desktop] dispatch_pending error: {e}");
                break;
            }
            std::thread::sleep(Duration::from_millis(16));
            state.frame_done = true;
        } else {
            if let Err(e) = event_queue.blocking_dispatch(state) {
                eprintln!("[desktop] blocking_dispatch error: {e}");
                break;
            }
        }
        if !state.frame_done { continue; }
        state.frame_done = false;

        let scale_f = state.fractional_scale() as f32;
        let now = Instant::now();
        let dt = now.duration_since(last_frame).as_secs_f32().min(0.05);
        last_frame = now;

        // Handle resize
        if state.configured {
            state.configured = false;
            gpu.ctx.resize(state.phys_width().max(1), state.phys_height().max(1));
            surface.set_buffer_scale(1);
            if let Some(vp) = viewport {
                vp.set_destination(state.width as i32, state.height as i32);
            }
            view_menu.set_scale(scale_f);
            context_menu.set_scale(scale_f);
        }

        let wf = gpu.ctx.width() as f32;
        let hf = gpu.ctx.height() as f32;
        let s = scale_f;

        // ── Cursor routing ──────────────────────────────────────────────
        let cx = (state.cursor_x as f32) * s;
        let cy = (state.cursor_y as f32) * s;

        if state.pointer_in_surface {
            input.on_cursor_moved(cx, cy);
        } else {
            input.on_cursor_left();
        }

        // ── Rubber band update ──────────────────────────────────────────
        if state.pointer_in_surface && app.rubber_band_start.is_some() {
            app.rubber_band_end = Some((cx, cy));
            update_rubber_band(app, wf, hf, s);
        }

        // ── Drag detection ──────────────────────────────────────────────
        if state.pointer_in_surface && app.drag_item.is_none() {
            if let (Some(idx), Some((px, py))) = (app.pending_open, app.press_pos) {
                let dist = ((cx - px).powi(2) + (cy - py).powi(2)).sqrt();
                if dist > 5.0 {
                    app.drag_item = Some(idx);
                    app.drag_pos = Some((cx, cy));
                    app.pending_open = None;
                    app.press_pos = None;
                }
            }

            // Pinned tab drag detection
            if tab_drag.is_none() {
                if let Some((tab_idx, press_x)) = tab_drag_press {
                    if (cx - press_x).abs() > 5.0 {
                        tab_drag = Some(tab_idx);
                        tab_drag_press = None;
                    }
                }
            }
        }
        if app.drag_item.is_some() && state.pointer_in_surface {
            app.drag_pos = Some((cx, cy));
        }

        // ── Keyboard ────────────────────────────────────────────────────
        if let Some(key) = state.key_pressed.take() {
            if active_panel == DesktopPanel::Terminal {
                terminal_panel.handle_key(key, state.ctrl, state.shift, state.alt);
            } else {
                handle_key(app, settings, context_menu, key, state.ctrl, state.shift, &mut state.running);
            }
        }

        // Key repeat
        if let Some(key) = state.held_key {
            if active_panel == DesktopPanel::Terminal {
                if std::time::Instant::now() >= state.repeat_deadline {
                    terminal_panel.handle_key(key, state.ctrl, state.shift, state.alt);
                    let interval = if state.repeat_started { 30 } else { 300 };
                    state.repeat_deadline = std::time::Instant::now()
                        + std::time::Duration::from_millis(interval);
                    state.repeat_started = true;
                    state.frame_done = true;
                }
            } else if (app.renaming.is_some() || app.path_editing || app.searching)
                && std::time::Instant::now() >= state.repeat_deadline
            {
                handle_key(app, settings, context_menu, key, state.ctrl, state.shift, &mut state.running);
                let interval = if state.repeat_started { 30 } else { 300 };
                state.repeat_deadline = std::time::Instant::now()
                    + std::time::Duration::from_millis(interval);
                state.repeat_started = true;
                state.frame_done = true;
            }
        }

        // ── Scroll ──────────────────────────────────────────────────────
        if state.scroll_delta.abs() > 0.01 {
            let scroll = state.scroll_delta * s;
            input.on_scroll(scroll);
            if active_panel == DesktopPanel::Terminal {
                // Terminal: scroll through scrollback history
                let session = terminal_panel.active_session_mut();
                let lines = if scroll > 0.0 { -3i32 } else { 3 };
                let new_offset = session.terminal.scroll_offset as i32 + lines;
                let max = session.terminal.scrollback.len();
                session.terminal.scroll_offset = new_offset.clamp(0, max as i32) as usize;
            } else if active_panel == DesktopPanel::Files {
                let content = content_rect(wf, hf, s);
                let zoom = app.icon_zoom;
                let total_h = match app.view_mode {
                    crate::app::ViewMode::Grid => {
                        let cols = grid_columns(content.w, s, zoom);
                        grid_content_height(app.entries.len(), cols, s, zoom)
                    }
                    crate::app::ViewMode::List => list_content_height(app.entries.len(), s),
                    crate::app::ViewMode::Tree => tree_content_height(app.tree_entries.len(), s),
                };
                ScrollArea::apply_scroll(&mut app.scroll_offset, scroll, total_h, content.h);
            }
            state.scroll_delta = 0.0;
        }

        // ── Left press ──────────────────────────────────────────────────
        if state.left_pressed {
            state.left_pressed = false;
            let in_ctx = context_menu.is_open() && context_menu.contains(cx, cy);
            let in_view = view_menu.is_open() && view_menu.contains(cx, cy);
            if context_menu.is_open() && !in_ctx {
                context_menu.close();
            } else if view_menu.is_open() && !in_view {
                view_menu.close();
            } else {
                let action = handle_click(
                    input, app, view_menu,
                    &mut last_tab_click, &mut tab_drag_press, s,
                    settings.bg_opacity,
                );
                match action {
                    ClickAction::None => {
                        // Terminal tab clicks
                        if active_panel == DesktopPanel::Terminal {
                            if let Some(zone_id) = input.active_zone_id() {
                                if zone_id == crate::ZONE_TERM_TAB_NEW {
                                    terminal_panel.new_tab();
                                } else if zone_id >= crate::ZONE_TERM_TAB_CLOSE_BASE
                                    && zone_id < crate::ZONE_TERM_TAB_CLOSE_BASE + 50
                                {
                                    terminal_panel.close_tab((zone_id - crate::ZONE_TERM_TAB_CLOSE_BASE) as usize);
                                } else if zone_id >= crate::ZONE_TERM_TAB_BASE
                                    && zone_id < crate::ZONE_TERM_TAB_BASE + 50
                                {
                                    terminal_panel.switch_tab((zone_id - crate::ZONE_TERM_TAB_BASE) as usize);
                                }
                            }
                        } else if app.pending_open.is_none() {
                            let cr = content_rect(wf, hf, s);
                            if cr.contains(cx, cy) {
                                app.clear_selection();
                                app.rubber_band_start = Some((cx, cy));
                                app.rubber_band_end = Some((cx, cy));
                            }
                        }
                    }
                    ClickAction::Close => {
                        state.running = false;
                    }
                    ClickAction::SwitchPanel(panel) => {
                        if panel != active_panel {
                            transition.start(active_panel, panel);
                            active_panel = panel;
                        }
                    }
                }
            }
        }

        // ── Left release ────────────────────────────────────────────────
        if state.left_released {
            state.left_released = false;
            if app.rubber_band_start.is_some() {
                app.rubber_band_start = None;
                app.rubber_band_end = None;
            }
            // Pinned tab drag release — reorder
            if let Some(src_idx) = tab_drag.take() {
                let tab_bar_rect = crate::layout::tab_bar_rect(wf, s);
                let tab_labels = app.tab_labels();
                let tab_label_refs: Vec<&str> = tab_labels.iter().map(|s| s.as_str()).collect();
                let rects = lntrn_ui::gpu::TabBar::new(tab_bar_rect)
                    .tabs(&tab_label_refs)
                    .scale(s)
                    .tab_rects();
                if let Some((cursor_x, _)) = input.cursor() {
                    let target_idx = rects.iter().position(|r| r.contains(cursor_x, r.y + r.h * 0.5))
                        .unwrap_or(src_idx);
                    if target_idx != src_idx
                        && target_idx < app.tabs.len()
                        && app.tabs[target_idx].pinned
                    {
                        let tab = app.tabs.remove(src_idx);
                        app.tabs.insert(target_idx, tab);
                        if app.current_tab == src_idx {
                            app.current_tab = target_idx;
                        } else if src_idx < app.current_tab && target_idx >= app.current_tab {
                            app.current_tab -= 1;
                        } else if src_idx > app.current_tab && target_idx <= app.current_tab {
                            app.current_tab += 1;
                        }
                    }
                }
            } else if let Some(_drag_idx) = app.drag_item.take() {
                app.pending_open = None;
            } else if let Some(idx) = app.pending_open.take() {
                app.on_item_click(idx);
            }
            tab_drag_press = None;
            input.on_left_released();
        }

        // ── Right click ─────────────────────────────────────────────────
        if state.right_clicked {
            state.right_clicked = false;
            if view_menu.is_open() {
                view_menu.close();
            }
            if context_menu.is_open() {
                context_menu.close();
            }
            if active_panel == DesktopPanel::Terminal {
                // Terminal right-click: font size + opacity sliders
                use lntrn_ui::gpu::MenuItem;
                if let Some((cx, cy)) = input.cursor() {
                    // Font slider: map font_size 14..32 to 0..1
                    let font_val = ((terminal_panel.font_size - 14.0) / 18.0).clamp(0.0, 1.0);
                    let opacity_val = settings.term_opacity;
                    context_menu.set_scale(s);
                    context_menu.open(cx, cy, vec![
                        MenuItem::slider(crate::TERM_FONT_SLIDER_ID, "Font Size", font_val),
                        MenuItem::slider(crate::TERM_OPACITY_SLIDER_ID, "Opacity", opacity_val),
                    ]);
                }
            } else {
                handle_right_click(app, context_menu, input, open_with_apps, wf, hf, s);
            }
        }

        // ── Update menus + transition ────────────────────────────────────
        view_menu.update(dt);
        context_menu.update(dt);
        let transition_animating = transition.tick(dt);

        // ── Read active panel from bar / compositor ───────────────────
        if let Ok(s) = std::fs::read_to_string(&panel_file) {
            let new_panel = match s.trim() {
                "terminal" => DesktopPanel::Terminal,
                "files" => DesktopPanel::Files,
                _ => DesktopPanel::Home,
            };
            if new_panel != active_panel {
                transition.start(active_panel, new_panel);
                active_panel = new_panel;
            }
        }

        // ── Terminal panel tick ──────────────────────────────────────────
        if active_panel == DesktopPanel::Terminal {
            terminal_panel.tick();
            // Resize terminal to match surface (minus tab bar)
            let tab_h = crate::terminal_panel::TAB_BAR_HEIGHT * s;
            let (cols, rows) = TerminalPanel::calc_grid_size(wf, hf - tab_h, terminal_panel.font_size);
            if cols != terminal_panel.active_session().terminal.cols
                || rows != terminal_panel.active_session().terminal.rows
            {
                terminal_panel.resize(cols, rows);
            }
        }

        // ── Render ──────────────────────────────────────────────────────
        let opacity = settings.bg_opacity;
        let render_palette = palette.with_bg_opacity(opacity);
        let (ctx_evt, view_evt) = crate::render::render_frame(
            gpu, app, input, icon_cache, file_info,
            &render_palette, s, view_menu, context_menu,
            tab_drag, opacity, active_panel, &mut terminal_panel, settings.term_opacity,
            &transition,
        );
        // Handle inline context menu events
        if let Some(evt) = ctx_evt {
            match &evt {
                MenuEvent::SliderChanged { id, value } => {
                    if *id == crate::TERM_FONT_SLIDER_ID {
                        // Map 0..1 → 14..32
                        let font = 14.0 + value * 18.0;
                        terminal_panel.font_size = font;
                        settings.term_font_size = font;
                        // Resize terminal grid to match new cell size
                        let tab_h = crate::terminal_panel::TAB_BAR_HEIGHT * s;
                        let (cols, rows) = TerminalPanel::calc_grid_size(wf, hf - tab_h, font * s);
                        terminal_panel.resize(cols, rows);
                        settings.save();
                    } else if *id == crate::TERM_OPACITY_SLIDER_ID {
                        settings.term_opacity = value.clamp(0.0, 1.0);
                        settings.save();
                    }
                }
                MenuEvent::Action(_) => {
                    context_menu.close();
                    handle_ctx_event(app, settings, context_menu, open_with_apps, evt);
                }
                _ => {
                    handle_ctx_event(app, settings, context_menu, open_with_apps, evt);
                }
            }
        }

        // Handle inline view menu events
        if let Some(evt) = view_evt {
            if let MenuEvent::SliderChanged { id, value } = evt {
                if id == VIEW_SLIDER_ID {
                    app.icon_zoom = value;
                    let content = content_rect(wf, hf, s);
                    ScrollArea::apply_scroll(
                        &mut app.scroll_offset, 0.0,
                        grid_content_height(app.entries.len(),
                            grid_columns(content.w, s, value), s, value),
                        content.h,
                    );
                } else if id == VIEW_OPACITY_SLIDER_ID {
                    settings.bg_opacity = value.clamp(0.0, 1.0);
                    settings.save();
                }
            }
        }

        surface.frame(qh, ());
        surface.commit();

        // Poll search results from background thread
        app.poll_search();

        // ── Auto-refresh: check directory mtime every 3 seconds ─────
        if app.current_dir != last_dir_path {
            last_dir_path = app.current_dir.clone();
            last_dir_mtime = std::fs::metadata(&app.current_dir)
                .and_then(|m| m.modified()).ok();
            last_dir_check = Instant::now();
        } else if last_dir_check.elapsed() >= Duration::from_secs(3) {
            last_dir_check = Instant::now();
            let current_mtime = std::fs::metadata(&app.current_dir)
                .and_then(|m| m.modified())
                .ok();
            if current_mtime != last_dir_mtime {
                last_dir_mtime = current_mtime;
                app.reload();
            }
        }

        needs_anim = view_menu.is_open() || context_menu.is_open()
            || app.drag_item.is_some() || app.rubber_band_start.is_some()
            || state.held_key.is_some()
            || app.search_rx.is_some()
            || tab_drag.is_some()
            || active_panel == DesktopPanel::Terminal
            || transition_animating;
    }

    Ok(())
}

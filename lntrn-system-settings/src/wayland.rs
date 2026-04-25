use std::ffi::c_void;
use std::ptr::NonNull;

use anyhow::{anyhow, Result};
use lntrn_render::{Color, GpuContext, GpuTexture, Painter, Rect, TextureDraw, TexturePass, TextRenderer};
use lntrn_ui::gpu::{
    FoxPalette, InteractionContext, MenuBar, MenuEvent, MenuItem, PopupSurface,
    WaylandPopupBackend,
};

use crate::config::{LanternConfig, WindowMode};
use crate::display_panel::{self, DisplayPanelState};
use crate::icon_panel;
use crate::icons;
use crate::input_panel;
use crate::monitor_arrange;
use crate::monitor_settings::persist_monitor_settings;
use crate::panels::{self, PanelState};
use crate::text_edit::{KeyboardState, keycode_to_char};
use crate::wayland_state::WaylandHandle;
// Re-exported so sibling modules (popup_backend, output_manager) can import
// them via the stable `crate::wayland::` path.
pub(crate) use crate::wayland_state::{OutputInfo, State};
use wayland_client::{Connection, EventQueue, Proxy};
use wayland_protocols::wp::cursor_shape::v1::client::wp_cursor_shape_device_v1;
use wayland_protocols::xdg::shell::client::xdg_toplevel;

const KEY_ESC: u32 = 1;
use crate::chrome::{self, TITLE_BAR_H, CORNER_RADIUS};

const SIDEBAR_W: f32 = 300.0;
const SIDEBAR_ITEM_H: f32 = 76.0;
const ICON_SIZE: u32 = 72; // rasterized icon size in pixels
const SIDEBAR_ICON_DRAW: f32 = 36.0; // logical draw size for icons

const ZONE_SIDEBAR_BASE: u32 = 200;

// View menu actions
const MENU_MODE_FOX: u32 = 600;
const MENU_MODE_NIGHT_SKY: u32 = 601;
const MENU_MODE_GROUP: u32 = 1;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Panel { WindowManager, Input, Display, Power, AppIcons }

const PANELS: &[(Panel, &str)] = &[
    (Panel::WindowManager, "Window Manager"),
    (Panel::Input, "Mouse"),
    (Panel::Display, "Display"),
    (Panel::Power, "Power"),
    (Panel::AppIcons, "App Icons"),
];

/// Build the menu bar menus for the given window mode. Currently just "View"
/// with a radio group for Fox / Night Sky.
fn build_view_menus(mode: WindowMode) -> Vec<(&'static str, Vec<MenuItem>)> {
    let is_fox = mode == WindowMode::Fox;
    vec![
        ("View", vec![
            MenuItem::header("Window Style"),
            MenuItem::radio(MENU_MODE_FOX, MENU_MODE_GROUP, "Fox", is_fox),
            MenuItem::radio(MENU_MODE_NIGHT_SKY, MENU_MODE_GROUP, "Night Sky", !is_fox),
        ]),
    ]
}

fn parse_panel_arg() -> Option<Panel> {
    let args: Vec<String> = std::env::args().collect();
    let idx = args.iter().position(|a| a == "--panel")?;
    match args.get(idx + 1)?.as_str() {
        "window-manager" => Some(Panel::WindowManager),
        "input" => Some(Panel::Input),
        "display" => Some(Panel::Display),
        "power" => Some(Panel::Power),
        "app-icons" => Some(Panel::AppIcons),
        _ => None,
    }
}

// ── Edge resize helper ──────────────────────────────────────────────────────

fn edge_resize(cx: f32, cy: f32, w: f32, h: f32, border: f32, controls_x: f32) -> Option<xdg_toplevel::ResizeEdge> {
    let left = cx < border;
    let right = cx > w - border;
    let top = cy < border;
    let bottom = cy > h - border;
    // Don't resize in the window controls area (top-right)
    if top && cx > controls_x { return None; }
    match (left, right, top, bottom) {
        (true, _, true, _) => Some(xdg_toplevel::ResizeEdge::TopLeft),
        (_, true, true, _) => Some(xdg_toplevel::ResizeEdge::TopRight),
        (true, _, _, true) => Some(xdg_toplevel::ResizeEdge::BottomLeft),
        (_, true, _, true) => Some(xdg_toplevel::ResizeEdge::BottomRight),
        (true, _, _, _) => Some(xdg_toplevel::ResizeEdge::Left),
        (_, true, _, _) => Some(xdg_toplevel::ResizeEdge::Right),
        (_, _, true, _) => Some(xdg_toplevel::ResizeEdge::Top),
        (_, _, _, true) => Some(xdg_toplevel::ResizeEdge::Bottom),
        _ => None,
    }
}

fn resize_edge_to_cursor_shape(edge: xdg_toplevel::ResizeEdge) -> wp_cursor_shape_device_v1::Shape {
    use wp_cursor_shape_device_v1::Shape;
    match edge {
        xdg_toplevel::ResizeEdge::Top => Shape::NResize,
        xdg_toplevel::ResizeEdge::Bottom => Shape::SResize,
        xdg_toplevel::ResizeEdge::Left => Shape::WResize,
        xdg_toplevel::ResizeEdge::Right => Shape::EResize,
        xdg_toplevel::ResizeEdge::TopLeft => Shape::NwResize,
        xdg_toplevel::ResizeEdge::TopRight => Shape::NeResize,
        xdg_toplevel::ResizeEdge::BottomLeft => Shape::SwResize,
        xdg_toplevel::ResizeEdge::BottomRight => Shape::SeResize,
        _ => Shape::Default,
    }
}

// ── Entry point ─────────────────────────────────────────────────────────────

pub fn run() -> Result<()> {
    let conn = Connection::connect_to_env()?;
    let display = conn.display();
    let mut event_queue: EventQueue<State> = conn.new_event_queue();
    let qh = event_queue.handle();
    let mut state = State::new();

    display.get_registry(&qh, ());
    event_queue.roundtrip(&mut state)?;

    let compositor = state.compositor.clone()
        .ok_or_else(|| anyhow!("wl_compositor not available"))?;
    let wm_base = state.wm_base.clone()
        .ok_or_else(|| anyhow!("xdg_wm_base not available"))?;

    if state.width == 0 { state.width = 960; }
    if state.height == 0 { state.height = 700; }

    let surface = compositor.create_surface(&qh, ());
    let xdg_surface = wm_base.get_xdg_surface(&surface, &qh, ());
    let toplevel = xdg_surface.get_toplevel(&qh, ());
    toplevel.set_title("System Settings".into());
    toplevel.set_app_id("lntrn-system-settings".into());
    toplevel.set_min_size(640, 480);
    surface.commit();

    state.surface = Some(surface.clone());
    state.xdg_surface = Some(xdg_surface);
    state.toplevel = Some(toplevel.clone());

    // Wait for initial configure
    while !state.configured {
        event_queue.blocking_dispatch(&mut state)?;
    }
    state.configured = false;

    surface.set_buffer_scale(1);
    let viewport = state.viewporter.as_ref().map(|vp| {
        let vp = vp.get_viewport(&surface, &qh, ());
        vp.set_destination(state.width as i32, state.height as i32);
        vp
    });

    // wgpu setup
    let display_ptr = conn.backend().display_ptr() as *mut c_void;
    let surface_ptr = Proxy::id(&surface).as_ptr() as *mut c_void;
    let wl_handle = WaylandHandle {
        display: NonNull::new(display_ptr).ok_or_else(|| anyhow!("null wl_display"))?,
        surface: NonNull::new(surface_ptr).ok_or_else(|| anyhow!("null wl_surface"))?,
    };

    let phys_w = state.phys_width().max(1);
    let phys_h = state.phys_height().max(1);
    let mut gpu = GpuContext::from_window(&wl_handle, phys_w, phys_h)
        .map_err(|e| anyhow!("GPU init failed: {e}"))?;
    let mut painter = Painter::new(&gpu);
    let mut text = TextRenderer::new(&gpu);
    let mut ix = InteractionContext::new();
    // Palette will be rebuilt each frame from the current window mode.
    let mut fox = FoxPalette::dark();
    // Menu bar in the title bar (View menu with theme switcher).
    let mut menu_bar = MenuBar::new(&fox);

    // Initialize popup backend
    {
        let xdg_surf = state.xdg_surface.as_ref().unwrap().clone();
        let vp = state.viewporter.as_ref();
        let scale = state.fractional_scale() as f32;
        state.popup_backend = Some(WaylandPopupBackend::new(
            &conn, &compositor, &wm_base, &xdg_surf, vp, &gpu, scale, &qh,
        ));
    }

    // Rasterize sidebar icons into GPU textures
    let tex_pass = TexturePass::new(&gpu);
    let icon_defs: [(Vec<icons::PathCmd>, Color); 5] = [
        (icons::icon_window_manager(), Color::from_rgb8(130, 170, 255)), // soft blue
        (icons::icon_input(),          Color::from_rgb8(180, 140, 220)), // lavender
        (icons::icon_display(),        Color::from_rgb8(100, 200, 180)), // teal
        (icons::icon_power(),          Color::from_rgb8(120, 210, 120)), // green
        (icons::icon_app_icons(),      Color::from_rgb8(230, 130, 180)), // pink
    ];
    let icon_textures: Vec<GpuTexture> = icon_defs.iter().map(|(cmds, color)| {
        let rgba = icons::rasterize_path(cmds, 24.0, 24.0, ICON_SIZE, ICON_SIZE, *color);
        tex_pass.upload(&gpu, &rgba, ICON_SIZE, ICON_SIZE)
    }).collect();

    let mut active_panel = parse_panel_arg().unwrap_or(Panel::Display);
    let mut config = LanternConfig::load();
    let mut saved_config = config.clone();
    // Seed the palette from the persisted window style.
    fox = chrome::content_palette(config.appearance.window_mode());
    let mut panel_state = PanelState::new(&fox);
    let mut display_state = DisplayPanelState::new(&config);
    let mut icon_panel_state = icon_panel::IconPanelState::new();
    let mut input_state = input_panel::InputPanelState::new();
    let mut kbd = KeyboardState::new();

    while state.running {
        if let Err(e) = event_queue.blocking_dispatch(&mut state) {
            eprintln!("[system-settings] dispatch error: {e}");
            break;
        }
        if !state.frame_done { continue; }
        state.frame_done = false;

        let s = state.fractional_scale() as f32;

        // Handle resize
        if state.configured {
            state.configured = false;
            gpu.resize(state.phys_width().max(1), state.phys_height().max(1));
            surface.set_buffer_scale(1);
            if let Some(vp) = &viewport {
                vp.set_destination(state.width as i32, state.height as i32);
            }
        }

        let wf = gpu.width() as f32;
        let hf = gpu.height() as f32;

        // Pre-compute content area layout (needed for both click handling and rendering)
        let title_h = TITLE_BAR_H * s;
        let body_y = title_h + 4.0 * s; // strip height
        let sidebar_w = SIDEBAR_W * s;
        let content_x = sidebar_w + 1.0 * s;
        let content_w = wf - content_x;
        // header_y (16) + header_size (26) + gap (12) + sep (1) + pad (16) = 71
        let panel_y = body_y + 16.0 * s + 26.0 * s + 12.0 * s + 1.0 * s + 16.0 * s;

        // Pointer routing
        let pointer_on_popup = state.pointer_surface.as_ref().and_then(|ps| {
            state.popup_backend.as_ref()?.find_popup_id_by_wl_surface(ps)
        });

        let cx = (state.cursor_x as f32) * s;
        let cy = (state.cursor_y as f32) * s;
        if pointer_on_popup.is_some() {
            ix.on_cursor_left();
        } else if state.pointer_in_surface {
            ix.on_cursor_moved(cx, cy);
        } else {
            ix.on_cursor_left();
        }
        if let Some(backend) = &mut state.popup_backend {
            let active = if state.pointer_in_surface { pointer_on_popup } else { None };
            backend.route_cursor(active, cx, cy);
        }

        // Process pending keymap/modifiers
        if let Some((fd, size)) = state.keymap_pending.take() {
            kbd.update_keymap(fd, size);
        }
        if let Some((dep, lat, lock, grp)) = state.modifiers_pending.take() {
            kbd.update_modifiers(dep, lat, lock, grp);
            state.shift = dep & 1 != 0;
        }

        // Keyboard
        if let Some(key) = state.key_pressed.take() {
            let sym = kbd.key_get_sym(key);
            let utf8 = kbd.key_to_utf8(key);

            // Fallback: if xkb didn't produce a keysym, use raw keycode mapping
            let (sym, utf8) = if sym.raw() == 0 {
                let fallback_sym = match key {
                    1 => xkbcommon::xkb::Keysym::new(0xff1b),  // Escape
                    14 => xkbcommon::xkb::Keysym::new(0xff08), // Backspace
                    28 => xkbcommon::xkb::Keysym::new(0xff0d), // Return
                    _ => sym,
                };
                let fallback_utf8 = utf8.or_else(|| keycode_to_char(key, state.shift).map(|c| c.to_string()));
                (fallback_sym, fallback_utf8)
            } else {
                (sym, utf8)
            };

            // Let focused text inputs consume the key first
            let consumed = display_panel::handle_display_key(
                &mut config, &mut display_state, sym, utf8.clone(),
            );
            let consumed = consumed || icon_panel_state.handle_key(sym, utf8);
            if !consumed && key == KEY_ESC {
                state.running = false;
            }
        }

        // Left press
        if state.left_pressed {
            state.left_pressed = false;
            // Has the menu bar's label or open dropdown consumed this click?
            let menu_consumed_click = if state.pointer_in_surface && pointer_on_popup.is_none() {
                let on_dropdown = menu_bar.is_open()
                    && menu_bar.context_menu.contains(cx, cy);
                if on_dropdown {
                    true
                } else {
                    let menus = build_view_menus(config.appearance.window_mode());
                    menu_bar.on_click(&mut ix, &menus, s)
                }
            } else {
                false
            };
            if let Some(pid) = pointer_on_popup {
                if let Some(backend) = &mut state.popup_backend {
                    if let Some(ctx) = backend.popup_render(pid) {
                        ctx.interaction.on_left_pressed();
                    }
                }
            } else if menu_consumed_click {
                // Menu bar (label or dropdown) consumed the click — nothing else to do.
            } else {
                let border = 10.0 * s;
                let controls_x = wf - 110.0 * s;
                if let Some(edge) = edge_resize(cx, cy, wf, hf, border, controls_x) {
                    if let Some(seat) = &state.seat {
                        toplevel.resize(seat, state.pointer_serial, edge);
                    }
                } else if cy < title_h {
                    // Chrome-style window controls (distance-based hit detection)
                    let hit_r = chrome::CONTROL_HIT_R * s;
                    let btn_y = title_h * 0.5;
                    let close_cx = wf - chrome::CLOSE_OFFSET * s;
                    let max_cx = wf - chrome::MAX_OFFSET * s;
                    let min_cx = wf - chrome::MIN_OFFSET * s;
                    let dist_close = ((cx - close_cx).powi(2) + (cy - btn_y).powi(2)).sqrt();
                    let dist_max = ((cx - max_cx).powi(2) + (cy - btn_y).powi(2)).sqrt();
                    let dist_min = ((cx - min_cx).powi(2) + (cy - btn_y).powi(2)).sqrt();
                    if dist_close < hit_r {
                        state.running = false;
                    } else if dist_max < hit_r {
                        if state.maximized { toplevel.unset_maximized(); }
                        else { toplevel.set_maximized(); }
                    } else if dist_min < hit_r {
                        toplevel.set_minimized();
                    } else {
                        // Drag to move
                        if let Some(seat) = &state.seat {
                            toplevel._move(seat, state.pointer_serial);
                        }
                    }
                } else if let Some(zone_id) = ix.on_left_pressed() {
                    match zone_id {
                        id if id >= ZONE_SIDEBAR_BASE && id < ZONE_SIDEBAR_BASE + PANELS.len() as u32 => {
                            active_panel = PANELS[(id - ZONE_SIDEBAR_BASE) as usize].0;
                            panel_state.close_dropdown();
                        }
                        panels::ZONE_SAVE => {
                            let wifi_changed =
                                config.power.wifi_power_save != saved_config.power.wifi_power_save
                                || config.power.wifi_power_scheme != saved_config.power.wifi_power_scheme;
                            // Apply output settings via wlr-output-management first so
                            // the new values are folded into config.monitors before save.
                            if display_state.monitor_settings.dirty {
                                if let Some(selected_name) = display_state.monitor_arrange.selected_output_name() {
                                    if let Some(hi) = state.output_mgr.heads.iter().position(|h| h.name == selected_name) {
                                        let changes = vec![crate::output_manager::HeadChange {
                                            head_idx: hi,
                                            mode_idx: display_state.monitor_settings.selected_mode_idx,
                                            position: None,
                                            scale: display_state.monitor_settings.selected_scale,
                                        }];
                                        crate::output_manager::apply_config(&state, &qh, &changes);
                                        persist_monitor_settings(
                                            &mut config,
                                            &state.output_mgr,
                                            hi,
                                            &selected_name,
                                            display_state.monitor_settings.selected_scale,
                                            display_state.monitor_settings.selected_mode_idx,
                                        );
                                        display_state.monitor_settings.dirty = false;
                                    }
                                }
                            }
                            config.save();
                            if wifi_changed {
                                config.apply_wifi_power();
                            }
                            saved_config = config.clone();
                        }
                        panels::ZONE_CANCEL => {
                            config = saved_config.clone();
                        }
                        id => {
                            // If a context menu is open, let it handle its own clicks
                            let menu_consumed = panel_state.dropdown_menu.is_open()
                                && panel_state.dropdown_menu.contains(cx, cy);
                            if !menu_consumed {
                                match active_panel {
                                    Panel::WindowManager => panels::handle_wm_click(&mut config, id),
                                    Panel::Power => {
                                        crate::power_panel::handle_power_click(
                                            &mut config, &mut panel_state, id, cx, cy,
                                        );
                                    }
                                    Panel::Display => {
                                        display_panel::handle_display_click(
                                            &mut config, &mut display_state, id,
                                            cx, cy, &state.output_mgr,
                                        );
                                    }
                                    Panel::Input => {
                                        input_panel::handle_input_click(&mut config, &input_state, id);
                                    }
                                    Panel::AppIcons => {
                                        icon_panel_state.on_click(id);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // Left release
        if state.left_released {
            state.left_released = false;
            // End monitor drag on release
            if monitor_arrange::is_dragging(&display_state.monitor_arrange) {
                monitor_arrange::handle_arrange_release(&mut display_state.monitor_arrange);
                if display_state.monitor_arrange.dirty {
                    config.monitors = display_state.monitor_arrange.to_config(&config.monitors);
                }
            }
            if let Some(pid) = pointer_on_popup {
                if let Some(backend) = &mut state.popup_backend {
                    if let Some(ctx) = backend.popup_render(pid) {
                        ctx.interaction.on_left_released();
                    }
                }
            } else {
                ix.on_left_released();
            }
        }

        // Auto-apply monitor settings (scale, mode) immediately when changed.
        // Display changes shouldn't need the Save button.
        if display_state.monitor_settings.dirty {
            if let Some(selected_name) = display_state.monitor_arrange.selected_output_name() {
                if let Some(hi) = state.output_mgr.heads.iter().position(|h| h.name == selected_name) {
                    let changes = vec![crate::output_manager::HeadChange {
                        head_idx: hi,
                        mode_idx: display_state.monitor_settings.selected_mode_idx,
                        position: None,
                        scale: display_state.monitor_settings.selected_scale,
                    }];
                    crate::output_manager::apply_config(&state, &qh, &changes);
                    persist_monitor_settings(
                        &mut config,
                        &state.output_mgr,
                        hi,
                        &selected_name,
                        display_state.monitor_settings.selected_scale,
                        display_state.monitor_settings.selected_mode_idx,
                    );
                    config.save();
                    saved_config = config.clone();
                    display_state.monitor_settings.dirty = false;
                }
            }
        }

        // Monitor drag update on pointer motion
        if monitor_arrange::is_dragging(&display_state.monitor_arrange) {
            monitor_arrange::handle_arrange_drag(&mut display_state.monitor_arrange, cx, cy);
        }

        // Right press (no context menu yet, just consume)
        if state.right_pressed {
            state.right_pressed = false;
        }

        // Handle popup_done
        if state.popup_closed {
            state.popup_closed = false;
        }

        // Capture scroll before reset
        let frame_scroll = state.scroll_delta;
        state.scroll_delta = 0.0;

        // ── Cursor shape ────────────────────────────────────────────────
        if state.pointer_in_surface {
            let border = 10.0 * s;
            let controls_x = wf - 110.0 * s;
            let desired = match edge_resize(cx, cy, wf, hf, border, controls_x) {
                Some(edge) => resize_edge_to_cursor_shape(edge),
                None => wp_cursor_shape_device_v1::Shape::Default,
            };
            if state.current_cursor_shape != Some(desired) {
                if let Some(dev) = &state.cursor_shape_device {
                    dev.set_shape(state.enter_serial, desired);
                }
                state.current_cursor_shape = Some(desired);
            }
        }

        // ── Render ──────────────────────────────────────────────────────
        ix.begin_frame();
        painter.clear();

        let sw = gpu.width();
        let sh = gpu.height();
        let r = if state.maximized { 0.0 } else { CORNER_RADIUS * s };

        // Pick the palette from the current window mode each frame so live
        // theme switches take effect immediately.
        let mode = config.appearance.window_mode();
        fox = chrome::content_palette(mode);
        let chrome_pal = chrome::ChromePalette::for_mode(mode);

        // Window chrome: background, title, controls, border
        chrome::draw_background(&mut painter, mode, wf, hf, r);
        chrome::draw_title(&mut text, "System Settings", s, wf, title_h, &chrome_pal, sw, sh);
        chrome::draw_controls(&mut painter, cx, cy, s, wf, title_h, &chrome_pal);

        // ── Menu bar (View menu with theme switcher) ───────────────────
        let menu_area = Rect::new(10.0 * s, 0.0, 200.0 * s, title_h);
        let view_menus = build_view_menus(mode);
        menu_bar.update(&mut ix, &view_menus, menu_area, s);
        let labels: Vec<&str> = view_menus.iter().map(|(l, _)| *l).collect();
        menu_bar.draw_with_labels(&mut painter, &mut text, &fox, &labels, sw, sh, s);

        // ── Sidebar ────────────────────────────────────────────────────
        let item_h = SIDEBAR_ITEM_H * s;
        let label_size = 22.0 * s;
        let icon_draw = SIDEBAR_ICON_DRAW * s;
        let mut tex_draws: Vec<TextureDraw> = Vec::new();

        // Both modes: let the window background flow through the sidebar so
        // there's no visible color seam between left and right. The active /
        // hover highlights and the divider line provide enough delineation.
        // Divider line between sidebar and content — subtle in both modes.
        painter.rect_filled(
            Rect::new(sidebar_w, body_y, 1.0 * s, hf - body_y),
            0.0,
            fox.muted.with_alpha(0.35),
        );

        for (i, (panel, label)) in PANELS.iter().enumerate() {
            let y = body_y + 8.0 * s + i as f32 * item_h;
            let zone_id = ZONE_SIDEBAR_BASE + i as u32;
            let rect = Rect::new(0.0, y, sidebar_w, item_h);
            let zone_state = ix.add_zone(zone_id, rect);
            let is_active = *panel == active_panel;

            // Highlight active or hovered item.
            // For active items we draw an inset rounded "pill" using a custom
            // bright-gold color (rather than `fox.accent.with_alpha`) so the
            // result reads as gold, not as muddy orange — low-alpha gold over
            // a near-black background blends to brown.
            if is_active {
                let inset_x = 10.0 * s;
                let inset_y = 6.0 * s;
                let pill = Rect::new(
                    inset_x,
                    y + inset_y,
                    sidebar_w - inset_x * 2.0,
                    item_h - inset_y * 2.0,
                );
                let radius = 8.0 * s;
                // Bright gold tint at high alpha — reads clearly as gold even
                // when blended over the dark Fox bg.
                painter.rect_filled(
                    pill,
                    radius,
                    lntrn_render::Color::from_rgba8(255, 180, 30, 56),
                );
                // 1px gold border to define the pill edge
                painter.rect_stroke_sdf(
                    pill, radius, 1.0 * s,
                    lntrn_render::Color::from_rgba8(255, 180, 30, 110),
                );
                // Active indicator bar on the left
                painter.rect_filled(
                    Rect::new(0.0, y + 8.0 * s, 4.0 * s, item_h - 16.0 * s),
                    2.0 * s,
                    fox.accent,
                );
            } else if zone_state.is_hovered() {
                let inset_x = 10.0 * s;
                let inset_y = 6.0 * s;
                let pill = Rect::new(
                    inset_x,
                    y + inset_y,
                    sidebar_w - inset_x * 2.0,
                    item_h - inset_y * 2.0,
                );
                painter.rect_filled(pill, 8.0 * s, fox.text.with_alpha(0.06));
            }

            // Icon
            let icon_x = 24.0 * s;
            let icon_y = y + (item_h - icon_draw) / 2.0;
            let draw = TextureDraw::new(&icon_textures[i], icon_x, icon_y, icon_draw, icon_draw);
            tex_draws.push(draw);

            // Label text
            let text_x = icon_x + icon_draw + 16.0 * s;
            let text_y = y + (item_h - label_size) / 2.0;
            let text_color = if is_active { fox.accent } else { fox.text };
            text.queue(label, label_size, text_x, text_y, text_color, sidebar_w - text_x, sw, sh);
        }

        // ── Content area header ────────────────────────────────────────
        let header_label = PANELS.iter().find(|(p, _)| *p == active_panel).map(|(_, l)| *l).unwrap_or("");
        let header_size = 26.0 * s;
        let header_y = body_y + 16.0 * s;
        text.queue(header_label, header_size, content_x + 24.0 * s, header_y, fox.text, content_w, sw, sh);

        // Separator under content header
        let sep_y = header_y + header_size + 12.0 * s;
        painter.rect_filled(
            Rect::new(content_x + 16.0 * s, sep_y, content_w - 32.0 * s, 1.0 * s),
            0.0,
            fox.muted.with_alpha(0.4),
        );

        // ── Panel content ───────────────────────────────────────────────
        match active_panel {
            Panel::WindowManager => {
                let panel_h = hf - panel_y;
                panels::draw_wm_panel(
                    &mut config, &mut panel_state, &mut painter, &mut text, &mut ix, &fox,
                    content_x, panel_y, content_w, panel_h, s, sw, sh, frame_scroll,
                );
            }
            Panel::Power => {
                let panel_h = hf - panel_y;
                crate::power_panel::draw_power_panel(
                    &mut config, &mut panel_state, &mut painter, &mut text, &mut ix, &fox,
                    content_x, panel_y, content_w, panel_h, s, sw, sh, frame_scroll,
                );
            }
            Panel::Display => {
                display_state.sync_from_config(&config);
                let panel_h = hf - panel_y;
                display_panel::draw_display_panel(
                    &mut config, &mut display_state,
                    &mut painter, &mut text, &mut ix, &tex_pass, &fox, &gpu,
                    content_x, panel_y, content_w, panel_h, s, sw, sh,
                    frame_scroll, &state.outputs, &state.output_mgr,
                );
                let thumb_draws = display_panel::collect_thumb_draws(&display_state, s);
                for td in thumb_draws {
                    tex_draws.push(td);
                }
            }
            Panel::Input => {
                let panel_h = hf - panel_y;
                input_panel::draw_input_panel(
                    &mut config, &mut input_state,
                    &mut painter, &mut text, &mut ix,
                    &tex_pass, &fox, &gpu,
                    content_x, panel_y, content_w, panel_h, s, sw, sh,
                    frame_scroll, &mut tex_draws,
                );
            }
            Panel::AppIcons => {
                let panel_h = hf - panel_y;
                icon_panel::draw_icon_panel(
                    &mut icon_panel_state,
                    &mut painter, &mut text, &mut ix, &tex_pass, &fox, &gpu,
                    content_x, panel_y, content_w, panel_h, s, sw, sh,
                    frame_scroll, &mut tex_draws,
                );
            }
        }

        // Save/Cancel bar (only when config has unsaved changes)
        let dirty = config != saved_config;
        if dirty {
            panels::draw_save_cancel_bar(
                &mut painter, &mut text, &mut ix, &fox,
                content_x, content_w, hf, s, sw, sh,
            );
        }

        // Window border (skip when maximized)
        if !state.maximized {
            chrome::draw_border(&mut painter, wf, hf, r, &chrome_pal);
        }

        // ── Menu bar dropdown overlay (drawn last so it's on top) ──────
        menu_bar.context_menu.set_scale(s);
        menu_bar.context_menu.update(0.016);
        if let Some(evt) = menu_bar.context_menu.draw(&mut painter, &mut text, &mut ix, sw, sh) {
            if let MenuEvent::RadioSelected { id, group: _ } = evt {
                let new_style = match id {
                    MENU_MODE_FOX => Some("fox"),
                    MENU_MODE_NIGHT_SKY => Some("night_sky"),
                    _ => None,
                };
                if let Some(style) = new_style {
                    if config.appearance.window_style != style {
                        config.appearance.window_style = style.into();
                        // Persist immediately — theme changes shouldn't need the Save button.
                        saved_config.appearance.window_style = style.into();
                        config.save();
                    }
                    menu_bar.close();
                }
            }
        }

        // ── Render pass ─────────────────────────────────────────────────
        if let Ok(mut frame) = gpu.begin_frame("system-settings") {
            let view = frame.view().clone();
            painter.render_pass(&gpu, frame.encoder_mut(), &view, Color::rgba(0.0, 0.0, 0.0, 0.0));
            if !tex_draws.is_empty() {
                tex_pass.render_pass(&gpu, frame.encoder_mut(), &view, &tex_draws, None);
            }
            text.render_queued(&gpu, frame.encoder_mut(), &view);
            frame.submit(&gpu.queue);
        }

        // Render popup surfaces
        if let Some(backend) = &mut state.popup_backend {
            backend.render_all();
        }

        ix.clear_scroll();
        surface.frame(&qh, ());
        surface.commit();
    }

    Ok(())
}

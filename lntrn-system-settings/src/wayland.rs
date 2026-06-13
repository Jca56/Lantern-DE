use std::ffi::c_void;
use std::ptr::NonNull;

use anyhow::{anyhow, Result};
use lntrn_render::{Color, GpuContext, GpuTexture, Painter, Rect, TextureDraw, TexturePass, TextRenderer};
use lntrn_ui::gpu::{
    FoxPalette, InteractionContext, PopupSurface, WaylandPopupBackend,
};

use crate::config::LanternConfig;
use crate::display_panel::{self, DisplayPanelState};
use crate::icon_panel;
use crate::icons;
use crate::input_panel;
use crate::keybinds_panel::{self, KeybindsPanelState};
use crate::monitor_arrange;
use crate::monitor_settings::persist_monitor_settings;
use crate::notifications_panel::{self, NotifPanelState};
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

use crate::sidebar::{draw_sidebar, SidebarState, SIDEBAR_W};

const ICON_SIZE: u32 = 72; // rasterized icon size in pixels

/// Sidebar zone ids are allocated dynamically per-row by `draw_sidebar`. The
/// click router maps `zone_id - ZONE_SIDEBAR_BASE` back through
/// `SidebarState::row_actions`. Reserve a wide block to fit every possible
/// (parent + child) row across all categories.
pub(crate) const ZONE_SIDEBAR_BASE: u32 = 200;

/// Every selectable destination in the sidebar tree. Categories themselves
/// (Appearance, Display, etc.) are toggles only — see `Category` in
/// `sidebar.rs` for the parent grouping.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub(crate) enum Panel {
    // Appearance
    Themes, WindowSizes, Animations,
    // Display (also hosts the wallpaper picker)
    Monitors,
    // Input
    Mouse, Keybindings,
    // Notifications
    NotifBehavior, NotifSound, NotifTesting,
    // Power
    LidIdle, Battery,
    // Apps
    AppIcons,
    // Lock Screen
    LockWallpaper, LockStyle,
}

fn parse_panel_arg() -> Option<Panel> {
    let args: Vec<String> = std::env::args().collect();
    let idx = args.iter().position(|a| a == "--panel")?;
    match args.get(idx + 1)?.as_str() {
        // Appearance subpanels (legacy "colors"/"windows"/"focus" all fold into Themes)
        "appearance" | "themes" | "colors" | "windows" | "focus" => Some(Panel::Themes),
        "window-sizes" | "sizes" => Some(Panel::WindowSizes),
        "animations"    => Some(Panel::Animations),
        // Display (wallpaper now lives here too)
        "home" | "display" | "monitors" | "wallpaper" => Some(Panel::Monitors),
        // Input (legacy "scrolling"/"clicking"/"cursor" all roll into Mouse)
        "input" | "mouse" | "scrolling" | "clicking" | "cursor" => Some(Panel::Mouse),
        "keybindings" | "keybinds" | "shortcuts" | "keyboard" => Some(Panel::Keybindings),
        // Notifications
        "notifications" | "notif-behavior" => Some(Panel::NotifBehavior),
        "notif-sound"   => Some(Panel::NotifSound),
        "notif-testing" => Some(Panel::NotifTesting),
        // Power
        "power" | "lid-idle" => Some(Panel::LidIdle),
        "battery"       => Some(Panel::Battery),
        // Apps
        "app-icons" | "apps" => Some(Panel::AppIcons),
        // Lock Screen
        "lock-screen" | "lockscreen" => Some(Panel::LockWallpaper),
        "lock-style"    => Some(Panel::LockStyle),
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

    if state.width == 0 { state.width = 1500; }
    if state.height == 0 { state.height = 1000; }

    let surface = compositor.create_surface(&qh, ());
    // Ask the compositor for this surface's fractional scale. Robust across
    // window sizes and monitors — and immune to a second output's wl_output
    // events clobbering the global scale fields (which shrank the window when
    // the secondary monitor was disabled).
    let frac_scale = state.frac_scale_mgr.as_ref()
        .map(|mgr| mgr.get_fractional_scale(&surface, &qh, ()));
    let xdg_surface = wm_base.get_xdg_surface(&surface, &qh, ());
    let toplevel = xdg_surface.get_toplevel(&qh, ());
    toplevel.set_title("System Settings".into());
    toplevel.set_app_id("lntrn-system-settings".into());
    toplevel.set_min_size(640, 480);
    surface.commit();

    state.surface = Some(surface.clone());
    state.xdg_surface = Some(xdg_surface);
    state.toplevel = Some(toplevel.clone());
    state.frac_scale = frac_scale;

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

    // Initialize popup backend
    {
        let xdg_surf = state.xdg_surface.as_ref().unwrap().clone();
        let vp = state.viewporter.as_ref();
        let scale = state.fractional_scale() as f32;
        state.popup_backend = Some(WaylandPopupBackend::new(
            &conn, &compositor, &wm_base, &xdg_surf, vp, &gpu, scale, &qh,
        ));
    }

    // Rasterize sidebar icons into GPU textures. Indices line up with
    // `sidebar::CategoryDef::icon_idx`.
    let tex_pass = TexturePass::new(&gpu);
    let icon_defs: [(Vec<icons::PathCmd>, Color); 7] = [
        (icons::icon_appearance(),     Color::from_rgb8(255, 180, 120)), // warm peach
        (icons::icon_display(),        Color::from_rgb8(100, 200, 180)), // teal
        (icons::icon_input(),          Color::from_rgb8(180, 140, 220)), // lavender
        (icons::icon_notifications(),  Color::from_rgb8(255, 200, 100)), // amber
        (icons::icon_power(),          Color::from_rgb8(120, 210, 120)), // green
        (icons::icon_app_icons(),      Color::from_rgb8(230, 130, 180)), // pink
        (icons::icon_lock(),           Color::from_rgb8(150, 180, 255)), // periwinkle
    ];
    let icon_textures: Vec<GpuTexture> = icon_defs.iter().map(|(cmds, color)| {
        let rgba = icons::rasterize_path(cmds, 24.0, 24.0, ICON_SIZE, ICON_SIZE, *color);
        tex_pass.upload(&gpu, &rgba, ICON_SIZE, ICON_SIZE)
    }).collect();

    let mut active_panel = parse_panel_arg().unwrap_or(Panel::Monitors);
    let mut sidebar_state = SidebarState::new(active_panel);
    let mut config = LanternConfig::load();
    let mut saved_config = config.clone();
    // Seed the palette from the persisted window style.
    fox = chrome::content_palette(config.appearance.window_mode());
    let mut panel_state = PanelState::new(&fox);
    let mut display_state = DisplayPanelState::new(&config);
    let mut lock_wp_state = crate::lock_wallpaper_panel::LockWallpaperState::new();
    let mut lock_style_state = crate::lock_style_panel::LockStyleState::new();
    let mut icon_panel_state = icon_panel::IconPanelState::new();
    let mut input_state = input_panel::InputPanelState::new();
    let mut notif_state = NotifPanelState::new();
    let mut keybinds_state = KeybindsPanelState::new();
    let mut themes_state = crate::appearance_themes::ThemesPanelState::new();
    let mut kbd = KeyboardState::new();

    while state.running {
        if let Err(e) = event_queue.blocking_dispatch(&mut state) {
            eprintln!("[system-settings] dispatch error: {e}");
            break;
        }
        if !state.frame_done { continue; }
        state.frame_done = false;

        // Pull any HDR capability updates from the compositor.
        state.hdr_client.poll();

        let s = state.fractional_scale() as f32;

        // Handle resize — on an explicit reconfigure, OR when the fractional
        // scale changed the physical surface size out from under us (a late
        // `preferred_scale` arriving after the first frame, or moving the
        // window to a monitor with a different scale).
        let target_w = state.phys_width().max(1);
        let target_h = state.phys_height().max(1);
        if state.configured || gpu.width() != target_w || gpu.height() != target_h {
            state.configured = false;
            gpu.resize(target_w, target_h);
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

            // Let focused text inputs consume the key first. Themes modal
            // takes precedence over the icon panel's text input because it
            // floats over everything else.
            let consumed = themes_state.handle_key(sym, utf8.clone(), &mut config)
                || icon_panel_state.handle_key(sym, utf8);
            if !consumed && key == KEY_ESC {
                state.running = false;
            }
        }

        // Left press
        if state.left_pressed {
            state.left_pressed = false;
            if let Some(pid) = pointer_on_popup {
                if let Some(backend) = &mut state.popup_backend {
                    if let Some(ctx) = backend.popup_render(pid) {
                        ctx.interaction.on_left_pressed();
                    }
                }
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
                    // "Keep HDR" confirmation button — handled here where
                    // hdr_client is mutable, before the (immutable-state) router.
                    if zone_id == crate::hdr_panel::ZONE_HDR_KEEP {
                        if let Some(name) = display_state.monitor_arrange.selected_output_name() {
                            state.hdr_client.confirm_hdr(&name);
                        }
                    }
                    // If a context menu is open, let it handle its own clicks
                    // first; otherwise route to sidebar / save-cancel / panel.
                    let menu_consumed = panel_state.dropdown_menu.is_open()
                        && panel_state.dropdown_menu.contains(cx, cy);
                    if !menu_consumed {
                        crate::click_router::route_zone_click(
                            zone_id,
                            &mut active_panel,
                            &mut sidebar_state,
                            &mut config,
                            &mut saved_config,
                            &mut panel_state,
                            &mut themes_state,
                            &mut display_state,
                            &mut lock_wp_state,
                            &mut icon_panel_state,
                            &input_state,
                            &mut keybinds_state,
                            &state,
                            &qh,
                            cx,
                            cy,
                        );
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
                    // Push the new positions to the compositor so reality
                    // matches the canvas immediately — without this the outputs
                    // stay at their old (often overlapping) coordinates and
                    // the cursor crosses an invisible duplicate boundary.
                    let changes = display_state.monitor_arrange
                        .position_changes(&state.output_mgr);
                    if !changes.is_empty() {
                        crate::output_manager::apply_config(&state, &qh, &changes);
                    }
                    config.monitors = display_state.monitor_arrange.to_config(&config.monitors);
                    config.save();
                    saved_config = config.clone();
                    display_state.monitor_arrange.dirty = false;
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
                        enabled: display_state.monitor_settings.selected_enabled,
                    }];
                    crate::output_manager::apply_config(&state, &qh, &changes);
                    persist_monitor_settings(
                        &mut config,
                        &state.output_mgr,
                        hi,
                        &selected_name,
                        display_state.monitor_settings.selected_scale,
                        display_state.monitor_settings.selected_mode_idx,
                        display_state.monitor_settings.selected_hdr,
                        display_state.monitor_settings.selected_sdr_brightness,
                        display_state.monitor_settings.selected_enabled,
                    );
                    // Live HDR apply over the compositor IPC socket.
                    if let Some(hdr_on) = display_state.monitor_settings.selected_hdr {
                        let nits = display_state.monitor_settings.selected_sdr_brightness.unwrap_or(203);
                        state.hdr_client.set_hdr(&selected_name, hdr_on, nits);
                    }
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

        // Window chrome: background + controls + border. No title text and
        // no View menu — categories live in the sidebar now.
        chrome::draw_background(&mut painter, mode, wf, hf, r);
        chrome::draw_controls(&mut painter, cx, cy, s, wf, title_h, &chrome_pal);

        // ── Sidebar ────────────────────────────────────────────────────
        let mut tex_draws: Vec<TextureDraw> = Vec::new();
        draw_sidebar(
            &mut sidebar_state,
            &mut painter, &mut text, &mut ix, &fox,
            &icon_textures, &mut tex_draws,
            active_panel, sidebar_w, body_y, hf, s, sw, sh,
        );

        // ── Content area header ────────────────────────────────────────
        let header_label = crate::sidebar::panel_label(active_panel);
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
        let panel_h = hf - panel_y;
        match active_panel {
            Panel::Themes | Panel::Animations => {
                crate::appearance_panel::draw_appearance_panel(
                    active_panel,
                    &mut config, &mut panel_state, &mut themes_state,
                    &mut painter, &mut text, &mut ix, &tex_pass, &gpu, &fox,
                    &mut tex_draws,
                    content_x, panel_y, content_w, panel_h, s, sw, sh, frame_scroll,
                );
                // Themes modal + thumbnails — only on the Themes subpanel.
                // Otherwise stale tile_layouts from the last visit would keep
                // pushing thumbnails over Windows / Animations.
                if active_panel == Panel::Themes {
                    if themes_state.modal_open() {
                        crate::appearance_themes::draw_themes_modal(
                            &mut themes_state, &mut painter, &mut text, &mut ix, &fox,
                            wf, hf, s, sw, sh,
                        );
                    }
                    for td in crate::appearance_themes::collect_theme_thumbs(&themes_state) {
                        tex_draws.push(td);
                    }
                }
            }
            Panel::WindowSizes => {
                crate::appearance_window_sizes::draw_window_sizes_page(
                    &mut config, &mut panel_state,
                    &mut painter, &mut text, &mut ix, &fox,
                    content_x, panel_y, content_w, panel_h, s, sw, sh, frame_scroll,
                );
            }
            Panel::Monitors => {
                display_panel::draw_display_panel(
                    active_panel,
                    &mut config, &mut display_state,
                    &mut painter, &mut text, &mut ix, &tex_pass, &fox, &gpu,
                    content_x, panel_y, content_w, panel_h, s, sw, sh,
                    frame_scroll, &state.outputs, &state.output_mgr, &state.hdr_client,
                );
                let thumb_draws = display_panel::collect_thumb_draws(&display_state, s);
                for td in thumb_draws { tex_draws.push(td); }
            }
            Panel::Mouse => {
                input_panel::draw_input_panel(
                    active_panel,
                    &mut config, &mut input_state,
                    &mut painter, &mut text, &mut ix,
                    &tex_pass, &fox, &gpu,
                    content_x, panel_y, content_w, panel_h, s, sw, sh,
                    frame_scroll, &mut tex_draws,
                );
            }
            Panel::Keybindings => {
                keybinds_panel::draw_keybinds_panel(
                    &mut config, &mut keybinds_state,
                    &mut painter, &mut text, &mut ix, &fox,
                    content_x, panel_y, content_w, panel_h, s, sw, sh, frame_scroll,
                );
            }
            Panel::NotifBehavior | Panel::NotifSound | Panel::NotifTesting => {
                notifications_panel::draw_notifications_panel(
                    active_panel,
                    &mut config, &mut notif_state,
                    &mut painter, &mut text, &mut ix, &fox,
                    content_x, panel_y, content_w, panel_h, s, sw, sh, frame_scroll,
                );
            }
            Panel::LidIdle | Panel::Battery => {
                crate::power_panel::draw_power_panel(
                    active_panel,
                    &mut config, &mut panel_state, &mut painter, &mut text, &mut ix, &fox,
                    content_x, panel_y, content_w, panel_h, s, sw, sh, frame_scroll,
                );
            }
            Panel::AppIcons => {
                icon_panel::draw_icon_panel(
                    &mut icon_panel_state,
                    &mut painter, &mut text, &mut ix, &tex_pass, &fox, &gpu,
                    content_x, panel_y, content_w, panel_h, s, sw, sh,
                    frame_scroll, &mut tex_draws,
                );
            }
            Panel::LockWallpaper => {
                crate::lock_wallpaper_panel::draw_lock_wallpaper_panel(
                    &mut config, &mut lock_wp_state,
                    &mut painter, &mut text, &mut ix, &tex_pass, &fox, &gpu,
                    content_x, panel_y, content_w, panel_h, s, sw, sh, frame_scroll,
                );
                let thumb_draws = crate::lock_wallpaper_panel::collect_thumb_draws(&lock_wp_state, s);
                for td in thumb_draws { tex_draws.push(td); }
            }
            Panel::LockStyle => {
                crate::lock_style_panel::draw_lock_style_panel(
                    &mut config, &mut lock_style_state,
                    &mut painter, &mut text, &mut ix, &fox,
                    content_x, panel_y, content_w, panel_h, s, sw, sh, frame_scroll,
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

        // ── Render pass ─────────────────────────────────────────────────
        // Iterate layers so context-menu overlays (Painter+TextRenderer
        // layer 1) fully cover the panel's text on layer 0 instead of having
        // base-layer text render on top of the menu's background.
        if let Ok(mut frame) = gpu.begin_frame("system-settings") {
            let view = frame.view().clone();
            let layers = painter.layer_count().max(text.layer_count());

            // Layer 0: base shapes + thumbnail textures + base text.
            painter.render_layer(
                0, &gpu, frame.encoder_mut(), &view,
                Some(Color::rgba(0.0, 0.0, 0.0, 0.0)),
            );
            if !tex_draws.is_empty() {
                tex_pass.render_pass(&gpu, frame.encoder_mut(), &view, &tex_draws, None);
            }
            text.render_layer(0, &gpu, frame.encoder_mut(), &view);

            // Layer 1+: overlays (menus, dropdowns) composited on top.
            for li in 1..layers {
                painter.render_layer(li, &gpu, frame.encoder_mut(), &view, None);
                text.render_layer(li, &gpu, frame.encoder_mut(), &view);
            }

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

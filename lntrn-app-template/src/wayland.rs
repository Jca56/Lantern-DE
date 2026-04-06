use std::ffi::c_void;
use std::ptr::NonNull;

use anyhow::{anyhow, Result};
use lntrn_render::{Color, GpuContext, Painter, Rect, TextRenderer};
use lntrn_ui::gpu::{
    FoxPalette, InteractionContext, MenuBar, MenuItem, PopupSurface,
    WaylandPopupBackend,
};
use raw_window_handle::{
    DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, RawDisplayHandle,
    RawWindowHandle, WaylandDisplayHandle, WaylandWindowHandle, WindowHandle,
};
use wayland_client::{
    protocol::{wl_compositor, wl_pointer, wl_seat, wl_surface},
    Connection, EventQueue, Proxy,
};
use wayland_protocols::wp::cursor_shape::v1::client::{
    wp_cursor_shape_device_v1, wp_cursor_shape_manager_v1,
};
use wayland_protocols::wp::viewporter::client::wp_viewporter;
use wayland_protocols::xdg::decoration::zv1::client::{
    zxdg_decoration_manager_v1, zxdg_toplevel_decoration_v1,
};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};

pub const BTN_LEFT: u32 = 0x110;
pub const BTN_RIGHT: u32 = 0x111;
const KEY_ESC: u32 = 1;
use crate::chrome::{TITLE_BAR_H, CORNER_RADIUS};

// ── WaylandHandle for wgpu ──────────────────────────────────────────────────

struct WaylandHandle {
    display: NonNull<c_void>,
    surface: NonNull<c_void>,
}
impl HasDisplayHandle for WaylandHandle {
    fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
        let raw = RawDisplayHandle::Wayland(WaylandDisplayHandle::new(self.display));
        Ok(unsafe { DisplayHandle::borrow_raw(raw) })
    }
}
impl HasWindowHandle for WaylandHandle {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        let raw = RawWindowHandle::Wayland(WaylandWindowHandle::new(self.surface));
        Ok(unsafe { WindowHandle::borrow_raw(raw) })
    }
}

// ── Wayland state ───────────────────────────────────────────────────────────

pub(crate) struct State {
    pub(crate) running: bool,
    pub(crate) configured: bool,
    pub(crate) frame_done: bool,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) scale: i32,
    pub(crate) output_phys_width: u32,
    pub(crate) maximized: bool,
    pub(crate) compositor: Option<wl_compositor::WlCompositor>,
    pub(crate) wm_base: Option<xdg_wm_base::XdgWmBase>,
    pub(crate) viewporter: Option<wp_viewporter::WpViewporter>,
    pub(crate) surface: Option<wl_surface::WlSurface>,
    pub(crate) xdg_surface: Option<xdg_surface::XdgSurface>,
    pub(crate) toplevel: Option<xdg_toplevel::XdgToplevel>,
    pub(crate) seat: Option<wl_seat::WlSeat>,
    pub(crate) cursor_x: f64,
    pub(crate) cursor_y: f64,
    pub(crate) pointer_in_surface: bool,
    pub(crate) left_pressed: bool,
    pub(crate) left_released: bool,
    pub(crate) right_pressed: bool,
    pub(crate) scroll_delta: f32,
    pub(crate) pointer_serial: u32,
    pub(crate) enter_serial: u32,
    pub(crate) cursor_shape_mgr: Option<wp_cursor_shape_manager_v1::WpCursorShapeManagerV1>,
    pub(crate) cursor_shape_device: Option<wp_cursor_shape_device_v1::WpCursorShapeDeviceV1>,
    pub(crate) current_cursor_shape: Option<wp_cursor_shape_device_v1::Shape>,
    pub(crate) pointer: Option<wl_pointer::WlPointer>,
    pub(crate) key_pressed: Option<u32>,
    pub(crate) decoration_mgr: Option<zxdg_decoration_manager_v1::ZxdgDecorationManagerV1>,
    pub(crate) popup_backend: Option<WaylandPopupBackend<State>>,
    pub(crate) popup_closed: bool,
    pub(crate) pointer_surface: Option<wl_surface::WlSurface>,
}

impl State {
    fn new() -> Self {
        Self {
            running: true, configured: false, frame_done: true,
            width: 0, height: 0, scale: 1, output_phys_width: 0, maximized: false,
            compositor: None, wm_base: None, viewporter: None,
            surface: None, xdg_surface: None, toplevel: None, seat: None,
            cursor_x: 0.0, cursor_y: 0.0, pointer_in_surface: false,
            left_pressed: false, left_released: false, right_pressed: false,
            scroll_delta: 0.0, pointer_serial: 0, enter_serial: 0,
            cursor_shape_mgr: None, cursor_shape_device: None,
            current_cursor_shape: None, pointer: None,
            key_pressed: None,
            decoration_mgr: None,
            popup_backend: None,
            popup_closed: false,
            pointer_surface: None,
        }
    }

    fn fractional_scale(&self) -> f64 {
        if self.output_phys_width > 0 && self.width > 0 {
            self.output_phys_width as f64 / self.width as f64
        } else {
            self.scale.max(1) as f64
        }
    }

    fn phys_width(&self) -> u32 { (self.width as f64 * self.fractional_scale()).round() as u32 }
    fn phys_height(&self) -> u32 { (self.height as f64 * self.fractional_scale()).round() as u32 }
}

// ── Edge resize helper ──────────────────────────────────────────────────────

/// `controls_x` is the left edge of the window controls zone — if the cursor
/// is in the top-right corner (x > controls_x AND y < border), skip resize so
/// clicks reach the close/max/min buttons instead.
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
    if state.height == 0 { state.height = 640; }

    let surface = compositor.create_surface(&qh, ());
    let xdg_surface = wm_base.get_xdg_surface(&surface, &qh, ());
    let toplevel = xdg_surface.get_toplevel(&qh, ());
    toplevel.set_title("Lantern".into());
    toplevel.set_app_id("lntrn-app-template".into());
    toplevel.set_min_size(640, 480);

    // Request client-side decorations so we control the title bar
    if let Some(mgr) = &state.decoration_mgr {
        let deco = mgr.get_toplevel_decoration(&toplevel, &qh, ());
        deco.set_mode(zxdg_toplevel_decoration_v1::Mode::ClientSide);
    }

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
    let fox = FoxPalette::night_sky();
    let mut menu_bar = MenuBar::new(&fox);
    let ctx_style = lntrn_ui::gpu::ContextMenuStyle {
        palette: fox.clone(),
        bg: Color::from_rgba8(18, 12, 40, 240),
        bg_hover: Color::from_rgba8(50, 120, 200, 50),
        text: Color::from_rgb8(210, 200, 230),
        text_muted: Color::from_rgb8(120, 110, 150),
        text_disabled: Color::from_rgb8(80, 70, 100),
        separator: Color::from_rgba8(100, 70, 160, 40),
        border: Color::from_rgba8(80, 55, 140, 50),
        accent: Color::from_rgb8(225, 175, 35),
        corner_radius: 12.0,
        padding: 5.0,
        item_height: 38.0,
        font_size: 22.0,
        min_width: 200.0,
        border_width: 1.0,
        scale: 1.0,
        no_shadow: false,
    };
    let mut right_click_menu = lntrn_ui::gpu::ContextMenu::new(ctx_style);

    // Initialize popup backend (clone xdg_surface to avoid borrow conflict)
    {
        let xdg_surf = state.xdg_surface.as_ref().unwrap().clone();
        let vp = state.viewporter.as_ref();
        let scale = state.fractional_scale() as f32;
        state.popup_backend = Some(WaylandPopupBackend::new(
            &conn, &compositor, &wm_base, &xdg_surf, vp, &gpu, scale, &qh,
        ));
    }

    let menus: Vec<(&str, Vec<MenuItem>)> = vec![
        ("File", vec![
            MenuItem::action(1, "New"),
            MenuItem::action_with(2, "Open", "Ctrl+O"),
            MenuItem::action_with(3, "Save", "Ctrl+S"),
            MenuItem::separator(),
            MenuItem::action_with(4, "Quit", "Ctrl+Q"),
        ]),
        ("Edit", vec![
            MenuItem::action_with(10, "Undo", "Ctrl+Z"),
            MenuItem::action_with(11, "Redo", "Ctrl+Shift+Z"),
            MenuItem::separator(),
            MenuItem::action_with(12, "Cut", "Ctrl+X"),
            MenuItem::action_with(13, "Copy", "Ctrl+C"),
            MenuItem::action_with(14, "Paste", "Ctrl+V"),
        ]),
        ("View", vec![
            MenuItem::toggle(20, "Dark Mode", true),
            MenuItem::toggle(21, "Show Sidebar", true),
            MenuItem::separator(),
            MenuItem::action(22, "Zoom In"),
            MenuItem::action(23, "Zoom Out"),
        ]),
        ("Help", vec![
            MenuItem::action(30, "About"),
        ]),
    ];

    let mut gallery = crate::gallery::GalleryState::new();
    let mut active_tab: usize = 0;
    let tab_names = ["Shell", "Widgets"];

    while state.running {
        if let Err(e) = event_queue.blocking_dispatch(&mut state) {
            eprintln!("[ui-test] dispatch error: {e}");
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

        // Determine if pointer is on main surface or popup
        let pointer_on_popup = state.pointer_surface.as_ref().and_then(|ps| {
            state.popup_backend.as_ref()?.find_popup_id_by_wl_surface(ps)
        });

        // Cursor — route to main or popup InteractionContext
        let cx = (state.cursor_x as f32) * s;
        let cy = (state.cursor_y as f32) * s;
        if pointer_on_popup.is_some() {
            // Pointer is on a popup — don't send to main ix
            ix.on_cursor_left();
        } else if state.pointer_in_surface {
            ix.on_cursor_moved(cx, cy);
        } else {
            ix.on_cursor_left();
        }
        // Route cursor to the active popup, clear it from all others
        if let Some(backend) = &mut state.popup_backend {
            let active = if state.pointer_in_surface { pointer_on_popup } else { None };
            backend.route_cursor(active, cx, cy);
        }

        // Tell the right-click menu which popup depth has the pointer
        {
            let depth = pointer_on_popup.and_then(|pid| {
                (0..right_click_menu.popup_count())
                    .find(|&d| right_click_menu.popup_id_at_depth(d) == Some(pid))
            });
            right_click_menu.set_pointer_depth(depth);
        }

        // Keyboard
        if let Some(key) = state.key_pressed.take() {
            if key == KEY_ESC { state.running = false; }
        }

        // Left press
        let title_h = TITLE_BAR_H * s;
        if state.left_pressed {
            state.left_pressed = false;
            // Route click to popup if pointer is on a popup
            if let Some(pid) = pointer_on_popup {
                if let Some(backend) = &mut state.popup_backend {
                    if let Some(ctx) = backend.popup_render(pid) {
                        ctx.interaction.on_left_pressed();
                    }
                }
            } else {
            // Close right-click popup menu on any left click outside
            if right_click_menu.is_open() {
                if let Some(backend) = &mut state.popup_backend {
                    right_click_menu.close_popups(backend);
                }
            }
            let border = 10.0 * s;
            let controls_x = wf - 120.0 * s;
            if let Some(edge) = edge_resize(cx, cy, wf, hf, border, controls_x) {
                if let Some(seat) = &state.seat {
                    toplevel.resize(seat, state.pointer_serial, edge);
                }
            } else if cy < title_h {
                // CSD title bar — check window control buttons (right side)
                let hit_r = 20.0 * s; // larger hitbox than visual
                let btn_y = title_h * 0.5;
                let close_cx = wf - 28.0 * s;
                let max_cx = wf - 66.0 * s;
                let min_cx = wf - 104.0 * s;
                let dist_close = ((cx - close_cx).powi(2) + (cy - btn_y).powi(2)).sqrt();
                let dist_max = ((cx - max_cx).powi(2) + (cy - btn_y).powi(2)).sqrt();
                let dist_min = ((cx - min_cx).powi(2) + (cy - btn_y).powi(2)).sqrt();
                if dist_close < hit_r {
                    state.running = false;
                } else if dist_max < hit_r {
                    if state.maximized {
                        toplevel.unset_maximized();
                    } else {
                        toplevel.set_maximized();
                    }
                } else if dist_min < hit_r {
                    toplevel.set_minimized();
                } else {
                    // Drag to move
                    if let Some(seat) = &state.seat {
                        toplevel._move(seat, state.pointer_serial);
                    }
                }
            } else if menu_bar.on_click(&mut ix, &menus, s) {
                // Menu bar consumed the click
            } else {
                // Tab bar hit-test
                let tab_y = title_h + 4.0 * s;
                let tab_h = 36.0 * s;
                let mut tab_hit = false;
                let mut tx = 32.0 * s;
                for i in 0..tab_names.len() {
                    let tw = tab_names[i].len() as f32 * 11.0 * s + 28.0 * s;
                    if cx >= tx && cx <= tx + tw && cy >= tab_y && cy <= tab_y + tab_h {
                        active_tab = i;
                        tab_hit = true;
                        break;
                    }
                    tx += tw + 8.0 * s;
                }
                if tab_hit {
                    // Tab consumed
                } else if active_tab == 1 && crate::gallery::handle_click(cx, cy, s, title_h + 44.0 * s, wf, hf, &mut gallery) {
                    // Gallery consumed
                } else {
                    ix.on_left_pressed();
                }
            }
            } // end else (not on popup)
        }

        // Slider drag
        crate::gallery::handle_drag(cx, cy, s, title_h, hf, &mut gallery);

        // Left release
        if state.left_released {
            state.left_released = false;
            crate::gallery::handle_release(&mut gallery);
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

        // Right press — open context menu via popup surface
        if state.right_pressed {
            state.right_pressed = false;
            menu_bar.close();
            // Close any existing popup menu
            if right_click_menu.is_open() {
                if let Some(backend) = &mut state.popup_backend {
                    right_click_menu.close_popups(backend);
                }
            }
            right_click_menu.set_scale(s);
            let items = vec![
                MenuItem::action_with(50, "Cut", "Ctrl+X"),
                MenuItem::action_with(51, "Copy", "Ctrl+C"),
                MenuItem::action_with(52, "Paste", "Ctrl+V"),
                MenuItem::separator(),
                MenuItem::action(53, "Select All"),
                MenuItem::separator(),
                MenuItem::submenu(60, "Transform", vec![
                    MenuItem::action(61, "Uppercase"),
                    MenuItem::action(62, "Lowercase"),
                    MenuItem::action(63, "Title Case"),
                    MenuItem::separator(),
                    MenuItem::action(64, "Sort Lines"),
                    MenuItem::action(65, "Reverse Lines"),
                ]),
                MenuItem::separator(),
                MenuItem::toggle(54, "Word Wrap", true),
                MenuItem::checkbox(55, "Show Line Numbers", false),
                MenuItem::separator(),
                MenuItem::action(56, "Inspect Element"),
            ];
            // Use logical coordinates for popup positioning
            let lx = state.cursor_x as i32;
            let ly = state.cursor_y as i32;
            if let Some(backend) = &mut state.popup_backend {
                right_click_menu.open_popup(lx as f32, ly as f32, items, backend);
            }
        }

        // Handle popup_done (compositor dismissed popup)
        if state.popup_closed {
            state.popup_closed = false;
            if let Some(backend) = &mut state.popup_backend {
                right_click_menu.close_popups(backend);
            }
        }

        // Reset scroll
        state.scroll_delta = 0.0;

        // ── Cursor shape ────────────────────────────────────────────────
        if state.pointer_in_surface {
            let border = 10.0 * s;
            let controls_x = wf - 120.0 * s;
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

        // ── Window chrome ────────────────────────────────────────────────
        crate::chrome::draw_background(&mut painter, wf, hf, r);
        crate::chrome::draw_title(&mut text, "Lantern", s, wf, title_h, sw, sh);
        crate::chrome::draw_controls(&mut painter, cx, cy, s, wf, title_h);

        // ── Tab bar ──────────────────────────────────────────────────────
        let tab_bar_y = title_h + 4.0 * s;
        let tab_h = 36.0 * s;
        let tab_pad = 32.0 * s;
        let accent_color = Color::from_rgb8(225, 175, 35);
        let tab_surface = Color::rgba(0.08, 0.04, 0.16, 0.40);
        let tab_border = Color::rgba(0.30, 0.20, 0.50, 0.12);
        {
            let mut tx = tab_pad;
            for i in 0..tab_names.len() {
                let tw = tab_names[i].len() as f32 * 11.0 * s + 28.0 * s;
                let is_active = active_tab == i;
                let hov = cx >= tx && cx <= tx + tw && cy >= tab_bar_y && cy <= tab_bar_y + tab_h;
                if is_active {
                    painter.rect_filled(Rect::new(tx, tab_bar_y, tw, tab_h), 8.0 * s, tab_surface);
                } else if hov {
                    painter.rect_filled(Rect::new(tx, tab_bar_y, tw, tab_h), 8.0 * s,
                        Color::rgba(0.06, 0.03, 0.12, 0.25));
                }
                let tc = if is_active { accent_color } else { crate::chrome::TEXT_SECONDARY };
                text.queue(tab_names[i], 18.0 * s, tx + 14.0 * s, tab_bar_y + 7.0 * s, tc, wf, sw, sh);
                tx += tw + 8.0 * s;
            }
            // Bottom line
            painter.rect_filled(
                Rect::new(tab_pad, tab_bar_y + tab_h, wf - tab_pad * 2.0, 1.0 * s), 0.0, tab_border,
            );
        }

        let content_top = tab_bar_y + tab_h + 4.0 * s;

        // ── Tab content ─────────────────────────────────────────────────
        if active_tab == 1 {
            crate::gallery::draw(
                &mut painter, &mut text, cx, cy, s, content_top,
                &gallery, wf, hf, sw, sh,
            );
        }
        // Tab 0 ("Shell") is intentionally empty

        if !state.maximized { crate::chrome::draw_border(&mut painter, wf, hf, r); }

        // Context menus (drawn into painter on top of other shapes)
        menu_bar.context_menu.update(0.016);
        if let Some(evt) = menu_bar.context_menu.draw(
            &mut painter, &mut text, &mut ix, sw, sh,
        ) {
            use lntrn_ui::gpu::MenuEvent;
            if matches!(evt, MenuEvent::Action(_)) {
                menu_bar.close();
            }
        }

        // Right-click menu draws into popup surfaces
        right_click_menu.update(0.016);
        if let Some(backend) = &mut state.popup_backend {
            backend.begin_frame_all();
        }
        if let Some(backend) = &mut state.popup_backend {
            if let Some(evt) = right_click_menu.draw_popups(backend) {
                use lntrn_ui::gpu::MenuEvent;
                if matches!(evt, MenuEvent::Action(_)) {
                    right_click_menu.close_popups(backend);
                }
            }
        }

        // Render pass — main window
        if let Ok(mut frame) = gpu.begin_frame("app-template") {
            let view = frame.view().clone();
            painter.render_pass(&gpu, frame.encoder_mut(), &view, Color::TRANSPARENT);
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

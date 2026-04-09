use std::ffi::c_void;
use std::ptr::NonNull;

use anyhow::{anyhow, Result};
use lntrn_render::{Color, GpuContext, Painter, TextRenderer};
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

use crate::chrome::TITLE_BAR_H;
use crate::playback::Playback;
use crate::preview::PreviewMonitor;

pub const BTN_LEFT: u32 = 0x110;
pub const BTN_RIGHT: u32 = 0x111;
const KEY_ESC: u32 = 1;
const KEY_SPACE: u32 = 57;

// ── WaylandHandle for wgpu ─────────────────────────────────────────────────

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

// ── Wayland state ──────────────────────────────────────────────────────────

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

// ── Edge resize helper ─────────────────────────────────────────────────────

fn edge_resize(
    cx: f32, cy: f32, w: f32, h: f32, border: f32, controls_x: f32,
) -> Option<xdg_toplevel::ResizeEdge> {
    let left = cx < border;
    let right = cx > w - border;
    let top = cy < border;
    let bottom = cy > h - border;
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

fn resize_edge_to_cursor(edge: xdg_toplevel::ResizeEdge) -> wp_cursor_shape_device_v1::Shape {
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

// ── Entry point ────────────────────────────────────────────────────────────

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

    if state.width == 0 { state.width = 1280; }
    if state.height == 0 { state.height = 800; }

    let surface = compositor.create_surface(&qh, ());
    let xdg_surface = wm_base.get_xdg_surface(&surface, &qh, ());
    let toplevel = xdg_surface.get_toplevel(&qh, ());
    toplevel.set_title("Lantern Edit".into());
    toplevel.set_app_id("lntrn-video-editor".into());
    toplevel.set_min_size(960, 600);

    if let Some(mgr) = &state.decoration_mgr {
        let deco = mgr.get_toplevel_decoration(&toplevel, &qh, ());
        deco.set_mode(zxdg_toplevel_decoration_v1::Mode::ClientSide);
    }

    surface.commit();
    state.surface = Some(surface.clone());
    state.xdg_surface = Some(xdg_surface);
    state.toplevel = Some(toplevel.clone());

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

    // GPU init
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

    // Playback + preview
    let mut playback = Playback::new();
    let mut preview = PreviewMonitor::new(&gpu);

    // Open video from CLI arg if provided
    if let Some(path) = std::env::args().nth(1) {
        if let Err(e) = playback.open_file(std::path::Path::new(&path)) {
            eprintln!("[video-editor] failed to open {path}: {e}");
        }
    }

    // Popup backend
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
            MenuItem::action(1, "New Project"),
            MenuItem::action_with(2, "Open", "Ctrl+O"),
            MenuItem::action_with(3, "Save", "Ctrl+S"),
            MenuItem::separator(),
            MenuItem::action(4, "Import Media"),
            MenuItem::action(5, "Export"),
            MenuItem::separator(),
            MenuItem::action_with(6, "Quit", "Ctrl+Q"),
        ]),
        ("Edit", vec![
            MenuItem::action_with(10, "Undo", "Ctrl+Z"),
            MenuItem::action_with(11, "Redo", "Ctrl+Shift+Z"),
            MenuItem::separator(),
            MenuItem::action_with(12, "Cut", "Ctrl+X"),
            MenuItem::action_with(13, "Copy", "Ctrl+C"),
            MenuItem::action_with(14, "Paste", "Ctrl+V"),
            MenuItem::action_with(15, "Delete", "Del"),
            MenuItem::separator(),
            MenuItem::action(16, "Select All"),
        ]),
        ("View", vec![
            MenuItem::toggle(20, "Media Browser", true),
            MenuItem::toggle(21, "Properties", true),
            MenuItem::separator(),
            MenuItem::action(22, "Zoom In"),
            MenuItem::action(23, "Zoom Out"),
            MenuItem::action(24, "Fit Timeline"),
        ]),
        ("Clip", vec![
            MenuItem::action(30, "Split at Playhead"),
            MenuItem::action(31, "Trim Start"),
            MenuItem::action(32, "Trim End"),
            MenuItem::separator(),
            MenuItem::action(33, "Speed / Duration"),
            MenuItem::action(34, "Unlink Audio"),
        ]),
    ];

    // ── Main loop ──────────────────────────────────────────────────────────
    while state.running {
        if let Err(e) = event_queue.blocking_dispatch(&mut state) {
            eprintln!("[video-editor] dispatch error: {e}");
            break;
        }
        if !state.frame_done { continue; }
        state.frame_done = false;

        let s = state.fractional_scale() as f32;

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

        // Keyboard
        if let Some(key) = state.key_pressed.take() {
            match key {
                KEY_ESC => state.running = false,
                KEY_SPACE => playback.toggle(),
                _ => {}
            }
        }

        // Poll for new decoded frames
        if playback.poll_frame() {
            if let Some(frame) = &playback.current_frame {
                preview.upload_frame(&gpu, frame);
            }
        }

        // Left press
        let title_h = TITLE_BAR_H * s;
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
                let controls_x = wf - 120.0 * s;
                if let Some(edge) = edge_resize(cx, cy, wf, hf, border, controls_x) {
                    if let Some(seat) = &state.seat {
                        toplevel.resize(seat, state.pointer_serial, edge);
                    }
                } else if cy < title_h {
                    let hit_r = 20.0 * s;
                    let btn_y = title_h * 0.5;
                    let close_cx = wf - 28.0 * s;
                    let max_cx = wf - 66.0 * s;
                    let min_cx = wf - 104.0 * s;
                    let dist = |bx: f32| ((cx - bx).powi(2) + (cy - btn_y).powi(2)).sqrt();
                    if dist(close_cx) < hit_r {
                        state.running = false;
                    } else if dist(max_cx) < hit_r {
                        if state.maximized { toplevel.unset_maximized(); }
                        else { toplevel.set_maximized(); }
                    } else if dist(min_cx) < hit_r {
                        toplevel.set_minimized();
                    } else if !menu_bar.on_click(&mut ix, &menus, s) {
                        if let Some(seat) = &state.seat {
                            toplevel._move(seat, state.pointer_serial);
                        }
                    }
                } else if !menu_bar.on_click(&mut ix, &menus, s) {
                    ix.on_left_pressed();
                }
            }
        }

        // Left release
        if state.left_released {
            state.left_released = false;
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

        // Right press (close menus)
        if state.right_pressed {
            state.right_pressed = false;
            menu_bar.close();
        }

        if state.popup_closed {
            state.popup_closed = false;
        }

        state.scroll_delta = 0.0;

        // Cursor shape
        if state.pointer_in_surface {
            let border = 10.0 * s;
            let controls_x = wf - 120.0 * s;
            let desired = match edge_resize(cx, cy, wf, hf, border, controls_x) {
                Some(edge) => resize_edge_to_cursor(edge),
                None => wp_cursor_shape_device_v1::Shape::Default,
            };
            if state.current_cursor_shape != Some(desired) {
                if let Some(dev) = &state.cursor_shape_device {
                    dev.set_shape(state.enter_serial, desired);
                }
                state.current_cursor_shape = Some(desired);
            }
        }

        // ── Render ─────────────────────────────────────────────────────────
        ix.begin_frame();
        painter.clear();

        let sw = gpu.width();
        let sh = gpu.height();
        // Background + chrome
        crate::chrome::draw_background(&mut painter, wf, hf);
        crate::chrome::draw_title_bar(&mut painter, &mut text, s, wf, sw, sh);
        crate::chrome::draw_menu_labels(&mut text, s, title_h, sw, sh, wf);
        crate::chrome::draw_controls(&mut painter, cx, cy, s, wf, title_h);

        // NLE panels
        let layout = crate::layout::Layout::compute(wf, hf, title_h, s);
        crate::render::draw_panels(&mut painter, &mut text, &layout, s, sw, sh);

        // Preview monitor (draws over the preview panel area)
        preview.draw(&mut painter, &mut text, &layout.preview, &playback, s, sw, sh);

        if !state.maximized { crate::chrome::draw_border(&mut painter, wf, hf); }

        // Menu bar overlay
        menu_bar.context_menu.update(0.016);
        if let Some(evt) = menu_bar.context_menu.draw(
            &mut painter, &mut text, &mut ix, sw, sh,
        ) {
            use lntrn_ui::gpu::MenuEvent;
            if matches!(evt, MenuEvent::Action(_)) {
                menu_bar.close();
            }
        }

        // Popup surfaces
        if let Some(backend) = &mut state.popup_backend {
            backend.begin_frame_all();
        }
        if let Some(backend) = &mut state.popup_backend {
            backend.render_all();
        }

        // Submit frame: painter → video texture → text
        if let Ok(mut frame) = gpu.begin_frame("video-editor") {
            let view = frame.view().clone();
            painter.render_pass(&gpu, frame.encoder_mut(), &view, Color::TRANSPARENT);
            preview.render_pass(&gpu, frame.encoder_mut(), &view, &layout.preview, &playback, s);
            text.render_queued(&gpu, frame.encoder_mut(), &view);
            frame.submit(&gpu.queue);
        }

        ix.clear_scroll();
        surface.frame(&qh, ());
        surface.commit();
    }

    Ok(())
}

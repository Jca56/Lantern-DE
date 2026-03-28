use std::ffi::c_void;
use std::ptr::NonNull;

use anyhow::{anyhow, Result};
use lntrn_render::{Color, GpuContext, Painter, Rect, SurfaceError, TextRenderer, TexturePass};
use lntrn_ui::gpu::{
    ContextMenu, ContextMenuStyle, FoxPalette, InteractionContext, MenuEvent, MenuItem,
};
use raw_window_handle::{
    DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, RawDisplayHandle,
    RawWindowHandle, WaylandDisplayHandle, WaylandWindowHandle, WindowHandle,
};
use wayland_client::{
    protocol::{
        wl_callback, wl_compositor, wl_output, wl_pointer, wl_region, wl_registry, wl_seat,
        wl_surface,
    },
    Connection, Dispatch, EventQueue, Proxy, QueueHandle,
};
use wayland_protocols::wp::viewporter::client::{wp_viewport, wp_viewporter};
use wayland_protocols_wlr::layer_shell::v1::client::{zwlr_layer_shell_v1, zwlr_layer_surface_v1};

/// Tray bar height in logical pixels.
const TRAY_HEIGHT: u32 = 48;
/// Gap between context menu and tray bar.
const TRAY_GAP: f32 = 8.0;
/// Corner radius for the tray bar.
const TRAY_RADIUS: f32 = 12.0;

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

struct State {
    running: bool,
    configured: bool,
    frame_done: bool,
    width: u32,
    height: u32,
    scale: i32,
    output_phys_width: u32,
    compositor: Option<wl_compositor::WlCompositor>,
    layer_shell: Option<zwlr_layer_shell_v1::ZwlrLayerShellV1>,
    viewporter: Option<wp_viewporter::WpViewporter>,
    surface: Option<wl_surface::WlSurface>,
    layer_surface: Option<zwlr_layer_surface_v1::ZwlrLayerSurfaceV1>,
    seat: Option<wl_seat::WlSeat>,
    cursor_x: f64,
    cursor_y: f64,
    pointer_in_surface: bool,
    right_clicked: bool,
    left_pressed: bool,
    left_released: bool,
}

impl State {
    fn new() -> Self {
        Self {
            running: true, configured: false, frame_done: true,
            width: 0, height: 0,
            scale: 1, output_phys_width: 0,
            compositor: None, layer_shell: None, viewporter: None,
            surface: None, layer_surface: None, seat: None,
            cursor_x: 0.0, cursor_y: 0.0, pointer_in_surface: false,
            right_clicked: false, left_pressed: false, left_released: false,
        }
    }

    fn fractional_scale(&self) -> f64 {
        if self.output_phys_width > 0 && self.width > 0 {
            self.output_phys_width as f64 / self.width as f64
        } else {
            self.scale.max(1) as f64
        }
    }

    fn phys_width(&self) -> u32 {
        (self.width as f64 * self.fractional_scale()).round() as u32
    }

    fn phys_height(&self) -> u32 {
        (self.height as f64 * self.fractional_scale()).round() as u32
    }
}

// ── Dispatch impls ───────────────────────────────────────────────────────────

impl Dispatch<wl_registry::WlRegistry, ()> for State {
    fn event(
        state: &mut Self, registry: &wl_registry::WlRegistry,
        event: wl_registry::Event, _: &(), _: &Connection, qh: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global { name, interface, version } = event {
            match interface.as_str() {
                "wl_compositor" => {
                    state.compositor = Some(registry.bind(name, version.min(6), qh, ()));
                }
                "zwlr_layer_shell_v1" => {
                    state.layer_shell = Some(registry.bind(name, version.min(4), qh, ()));
                }
                "wp_viewporter" => {
                    state.viewporter = Some(registry.bind(name, version.min(1), qh, ()));
                }
                "wl_output" => {
                    let _: wl_output::WlOutput = registry.bind(name, version.min(4), qh, ());
                }
                "wl_seat" => {
                    state.seat = Some(registry.bind(name, version.min(9), qh, ()));
                }
                _ => {}
            }
        }
    }
}

impl Dispatch<wl_compositor::WlCompositor, ()> for State {
    fn event(_: &mut Self, _: &wl_compositor::WlCompositor, _: wl_compositor::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}
impl Dispatch<wl_surface::WlSurface, ()> for State {
    fn event(_: &mut Self, _: &wl_surface::WlSurface, _: wl_surface::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}
impl Dispatch<wl_region::WlRegion, ()> for State {
    fn event(_: &mut Self, _: &wl_region::WlRegion, _: wl_region::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}
impl Dispatch<wp_viewporter::WpViewporter, ()> for State {
    fn event(_: &mut Self, _: &wp_viewporter::WpViewporter, _: wp_viewporter::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}
impl Dispatch<wp_viewport::WpViewport, ()> for State {
    fn event(_: &mut Self, _: &wp_viewport::WpViewport, _: wp_viewport::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}
impl Dispatch<zwlr_layer_shell_v1::ZwlrLayerShellV1, ()> for State {
    fn event(_: &mut Self, _: &zwlr_layer_shell_v1::ZwlrLayerShellV1, _: zwlr_layer_shell_v1::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<wl_output::WlOutput, ()> for State {
    fn event(
        state: &mut Self, _: &wl_output::WlOutput, event: wl_output::Event,
        _: &(), _: &Connection, _: &QueueHandle<Self>,
    ) {
        match event {
            wl_output::Event::Scale { factor } => state.scale = factor,
            wl_output::Event::Mode { width, .. } => state.output_phys_width = width as u32,
            _ => {}
        }
    }
}

impl Dispatch<wl_callback::WlCallback, ()> for State {
    fn event(state: &mut Self, _: &wl_callback::WlCallback, _: wl_callback::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        state.frame_done = true;
    }
}

impl Dispatch<zwlr_layer_surface_v1::ZwlrLayerSurfaceV1, ()> for State {
    fn event(
        state: &mut Self, layer_surface: &zwlr_layer_surface_v1::ZwlrLayerSurfaceV1,
        event: zwlr_layer_surface_v1::Event, _: &(), _: &Connection, _: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_layer_surface_v1::Event::Configure { serial, width, height } => {
                layer_surface.ack_configure(serial);
                if width > 0 { state.width = width; }
                if height > 0 { state.height = height; }
                state.configured = true;
                state.frame_done = true;
            }
            zwlr_layer_surface_v1::Event::Closed => state.running = false,
            _ => {}
        }
    }
}

impl Dispatch<wl_seat::WlSeat, ()> for State {
    fn event(
        _: &mut Self, seat: &wl_seat::WlSeat, event: wl_seat::Event,
        _: &(), _: &Connection, qh: &QueueHandle<Self>,
    ) {
        if let wl_seat::Event::Capabilities { capabilities: caps, .. } = event {
            if let wayland_client::WEnum::Value(caps) = caps {
                if caps.contains(wl_seat::Capability::Pointer) {
                    seat.get_pointer(qh, ());
                }
            }
        }
    }
}

const BTN_LEFT: u32 = 0x110;
const BTN_RIGHT: u32 = 0x111;

impl Dispatch<wl_pointer::WlPointer, ()> for State {
    fn event(
        state: &mut Self, _: &wl_pointer::WlPointer, event: wl_pointer::Event,
        _: &(), _: &Connection, _: &QueueHandle<Self>,
    ) {
        match event {
            wl_pointer::Event::Enter { surface_x, surface_y, .. } => {
                state.pointer_in_surface = true;
                state.cursor_x = surface_x;
                state.cursor_y = surface_y;
            }
            wl_pointer::Event::Leave { .. } => state.pointer_in_surface = false,
            wl_pointer::Event::Motion { surface_x, surface_y, .. } => {
                state.cursor_x = surface_x;
                state.cursor_y = surface_y;
            }
            wl_pointer::Event::Button { button, state: btn_state, .. } => {
                use wayland_client::WEnum;
                let pressed = WEnum::Value(wl_pointer::ButtonState::Pressed);
                let released = WEnum::Value(wl_pointer::ButtonState::Released);
                if button == BTN_RIGHT && btn_state == pressed { state.right_clicked = true; }
                if button == BTN_LEFT && btn_state == pressed { state.left_pressed = true; }
                if button == BTN_LEFT && btn_state == released { state.left_released = true; }
            }
            _ => {}
        }
        state.frame_done = true;
    }
}

// ── Entry point ──────────────────────────────────────────────────────────────

pub fn run(open_x: f64, open_y: f64) -> Result<()> {
    let conn = Connection::connect_to_env()?;
    let display = conn.display();
    let mut event_queue: EventQueue<State> = conn.new_event_queue();
    let qh = event_queue.handle();
    let mut state = State::new();

    display.get_registry(&qh, ());
    event_queue.roundtrip(&mut state)?;

    let compositor = state.compositor.as_ref()
        .ok_or_else(|| anyhow!("wl_compositor not available"))?;
    let layer_shell = state.layer_shell.as_ref()
        .ok_or_else(|| anyhow!("zwlr_layer_shell_v1 not available"))?;

    let surface = compositor.create_surface(&qh, ());
    let input_region = compositor.create_region(&qh, ());

    // Full-screen transparent overlay on the Overlay layer
    let layer_surface = layer_shell.get_layer_surface(
        &surface, None, zwlr_layer_shell_v1::Layer::Overlay,
        "lntrn-menu".to_string(), &qh, (),
    );
    {
        use zwlr_layer_surface_v1::Anchor;
        layer_surface.set_anchor(Anchor::Top | Anchor::Bottom | Anchor::Left | Anchor::Right);
        layer_surface.set_size(0, 0); // fill screen
        layer_surface.set_exclusive_zone(-1); // don't push other surfaces
        layer_surface.set_keyboard_interactivity(
            zwlr_layer_surface_v1::KeyboardInteractivity::OnDemand,
        );
    }
    surface.set_input_region(Some(&input_region));
    surface.commit();

    state.surface = Some(surface.clone());
    state.layer_surface = Some(layer_surface.clone());

    while !state.configured {
        event_queue.blocking_dispatch(&mut state)?;
    }
    if state.width == 0 {
        return Err(anyhow!("compositor sent zero-width configure"));
    }
    event_queue.roundtrip(&mut state)?;

    tracing::info!(w = state.width, h = state.height, "menu overlay configured");

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

    let palette = FoxPalette::dark();
    let mut menu_style = ContextMenuStyle::from_palette(&palette);
    menu_style.font_size = 24.0;
    menu_style.item_height = 40.0;
    menu_style.min_width = 220.0;
    let mut context_menu = ContextMenu::new(menu_style);
    context_menu.set_scale(state.fractional_scale() as f32);

    let tex_pass = TexturePass::new(&gpu);
    let mut system_tray = crate::tray::SystemTray::start();

    let mut ix = InteractionContext::new();
    let mut menu_was_open = false;

    // Open menu immediately at the cursor position passed from compositor
    {
        let scale_f = state.fractional_scale() as f32;
        let phys_x = open_x as f32 * scale_f;
        let phys_y = open_y as f32 * scale_f;
        let phys_w_f = state.phys_width() as f32;
        let phys_h_f = state.phys_height() as f32;
        context_menu.open(
            phys_x, phys_y,
            vec![
                MenuItem::header("Desktop"),
                MenuItem::action(1, "Terminal"),
                MenuItem::action(2, "File Manager"),
                MenuItem::action(3, "Screenshot"),
                MenuItem::separator(),
                MenuItem::action(4, "Settings"),
            ],
        );
        context_menu.clamp_to_screen(phys_w_f, phys_h_f);
        surface.set_input_region(None);
        menu_was_open = true;
    }

    tracing::info!("menu ready, entering render loop");

    while state.running {
        event_queue.blocking_dispatch(&mut state)?;
        if !state.frame_done { continue; }
        state.frame_done = false;

        let scale_f = state.fractional_scale() as f32;

        if state.configured {
            state.configured = false;
            gpu.resize(state.phys_width().max(1), state.phys_height().max(1));
            surface.set_buffer_scale(1);
            if let Some(vp) = &viewport {
                vp.set_destination(state.width as i32, state.height as i32);
            }
            context_menu.set_scale(scale_f);
        }

        let phys_w = state.phys_width().max(1);
        let phys_h = state.phys_height().max(1);
        let phys_w_f = phys_w as f32;
        let phys_h_f = phys_h as f32;

        let phys_cx = (state.cursor_x as f32) * scale_f;
        let phys_cy = (state.cursor_y as f32) * scale_f;

        // ── Input handling ───────────────────────────────────────────
        if state.left_pressed {
            state.left_pressed = false;
            ix.on_left_pressed();
            if let Some(tray) = &system_tray {
                tray.handle_click(&ix, phys_cx, phys_cy);
            }
        }
        if state.left_released {
            state.left_released = false;
            if context_menu.is_open() && !context_menu.contains(phys_cx, phys_cy) {
                context_menu.close();
            }
            ix.on_left_released();
        }

        ix.begin_frame();
        if state.pointer_in_surface {
            ix.on_cursor_moved(phys_cx, phys_cy);
        } else {
            ix.on_cursor_left();
        }

        // Exit when menu finishes closing
        let menu_open = context_menu.is_open();
        if !menu_open && menu_was_open {
            break;
        }
        menu_was_open = menu_open;

        // ── Draw ─────────────────────────────────────────────────────
        painter.clear();

        // Tray bar: small bar positioned below the context menu
        let tray_phys_h = TRAY_HEIGHT as f32 * scale_f;
        let tray_w = 300.0 * scale_f;

        let mut tray_tex_draws = Vec::new();
        if let Some(tray) = &mut system_tray {
            tray.poll(&tex_pass, &gpu);

            // Position tray bar below the context menu
            let (tray_x, tray_y) = if let Some(menu_bounds) = context_menu.bounds() {
                let tx = menu_bounds.x;
                let ty = menu_bounds.y + menu_bounds.h + TRAY_GAP * scale_f;
                (tx, ty)
            } else {
                // Centered at bottom when no menu
                let tx = (phys_w_f - tray_w) / 2.0;
                let ty = phys_h_f - tray_phys_h - 20.0 * scale_f;
                (tx, ty)
            };

            // Draw tray background
            let actual_tray_w = tray_w.max(context_menu.bounds().map(|b| b.w).unwrap_or(tray_w));
            painter.rect_filled(
                Rect::new(tray_x, tray_y, actual_tray_w, tray_phys_h),
                TRAY_RADIUS * scale_f,
                palette.surface,
            );

            let (_, draws) = tray.draw(
                &mut painter, &mut text, &mut ix, &palette,
                tray_x, tray_y, actual_tray_w, tray_phys_h,
                0.0, // no clock offset
                scale_f, phys_w, phys_h,
            );
            tray_tex_draws = draws;
        }

        // Context menu (drawn on top)
        context_menu.update(1.0 / 60.0);
        if let Some(event) = context_menu.draw(&mut painter, &mut text, &mut ix, phys_w, phys_h) {
            match event {
                MenuEvent::Action(id) => {
                    tracing::info!(id, "menu action");
                    context_menu.close();
                }
                _ => {}
            }
        }

        // ── Render ───────────────────────────────────────────────────
        match gpu.begin_frame("Menu") {
            Ok(mut frame) => {
                let view = frame.view().clone();
                painter.render_pass(&gpu, frame.encoder_mut(), &view, Color::TRANSPARENT);
                if !tray_tex_draws.is_empty() {
                    tex_pass.render_pass(&gpu, frame.encoder_mut(), &view, &tray_tex_draws, None);
                }
                text.render_queued(&gpu, frame.encoder_mut(), &view);
                frame.submit(&gpu.queue);
            }
            Err(SurfaceError::Outdated | SurfaceError::Lost) => {
                gpu.resize(state.phys_width().max(1), state.phys_height().max(1));
            }
            Err(SurfaceError::OutOfMemory) => {
                tracing::error!("GPU OOM, exiting");
                break;
            }
            Err(_) => {}
        }

        surface.frame(&qh, ());
        surface.commit();
    }

    Ok(())
}

use std::ffi::c_void;
use std::ptr::NonNull;

use anyhow::{anyhow, Result};
use lntrn_render::{GpuContext, Painter, TextRenderer, TexturePass};
use lntrn_ui::gpu::{
    ContextMenu, ContextMenuStyle, FoxPalette, InteractionContext,
};
use raw_window_handle::{
    DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, RawDisplayHandle,
    RawWindowHandle, WaylandDisplayHandle, WaylandWindowHandle, WindowHandle,
};
use wayland_client::{
    protocol::{
        wl_compositor, wl_seat, wl_surface,
    },
    Connection, EventQueue, Proxy,
};
use wayland_protocols::wp::viewporter::client::wp_viewporter;
use wayland_protocols_wlr::layer_shell::v1::client::{zwlr_layer_shell_v1, zwlr_layer_surface_v1};

use crate::app::App;
use crate::desktop::DesktopApp;
use crate::icons::IconCache;
use crate::Gpu;
use crate::settings::Settings;

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
    // Wayland objects
    pub(crate) compositor: Option<wl_compositor::WlCompositor>,
    pub(crate) viewporter: Option<wp_viewporter::WpViewporter>,
    pub(crate) surface: Option<wl_surface::WlSurface>,
    pub(crate) seat: Option<wl_seat::WlSeat>,
    // Layer shell
    pub(crate) layer_shell: Option<zwlr_layer_shell_v1::ZwlrLayerShellV1>,
    pub(crate) layer_surface: Option<zwlr_layer_surface_v1::ZwlrLayerSurfaceV1>,
    // Input
    pub(crate) cursor_x: f64,
    pub(crate) cursor_y: f64,
    pub(crate) pointer_in_surface: bool,
    pub(crate) left_pressed: bool,
    pub(crate) left_released: bool,
    pub(crate) right_clicked: bool,
    pub(crate) scroll_delta: f32,
    pub(crate) pointer_serial: u32,
    pub(crate) pointer_surface: Option<wl_surface::WlSurface>,
    // Keyboard
    pub(crate) ctrl: bool,
    pub(crate) shift: bool,
    pub(crate) alt: bool,
    pub(crate) key_pressed: Option<u32>,
    // Key repeat
    pub(crate) held_key: Option<u32>,
    pub(crate) repeat_deadline: std::time::Instant,
    pub(crate) repeat_started: bool,
}

impl State {
    fn new() -> Self {
        Self {
            running: true, configured: false, frame_done: true,
            width: 0, height: 0, scale: 1, output_phys_width: 0,
            compositor: None, viewporter: None,
            surface: None, seat: None,
            layer_shell: None, layer_surface: None,
            cursor_x: 0.0, cursor_y: 0.0, pointer_in_surface: false,
            left_pressed: false, left_released: false, right_clicked: false,
            scroll_delta: 0.0, pointer_serial: 0, pointer_surface: None,
            ctrl: false, shift: false, alt: false, key_pressed: None,
            held_key: None, repeat_deadline: std::time::Instant::now(), repeat_started: false,
        }
    }

    pub(crate) fn fractional_scale(&self) -> f64 {
        if self.output_phys_width > 0 && self.width > 0 {
            self.output_phys_width as f64 / self.width as f64
        } else {
            self.scale.max(1) as f64
        }
    }

    pub(crate) fn phys_width(&self) -> u32 { (self.width as f64 * self.fractional_scale()).round() as u32 }
    pub(crate) fn phys_height(&self) -> u32 { (self.height as f64 * self.fractional_scale()).round() as u32 }
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

    let settings = Settings::load();
    let surface = compositor.create_surface(&qh, ());

    // ── Layer shell desktop mode ─────────────────────────────────
    let layer_shell = state.layer_shell.clone()
        .ok_or_else(|| anyhow!("zwlr_layer_shell_v1 not available"))?;

    let layer_surface = layer_shell.get_layer_surface(
        &surface, None,
        zwlr_layer_shell_v1::Layer::Bottom,
        "lntrn-desktop".to_string(),
        &qh, (),
    );
    // Anchor all edges + size 0 = fill available space (bar's exclusive zone respected)
    layer_surface.set_anchor(
        zwlr_layer_surface_v1::Anchor::Top
        | zwlr_layer_surface_v1::Anchor::Bottom
        | zwlr_layer_surface_v1::Anchor::Left
        | zwlr_layer_surface_v1::Anchor::Right,
    );
    layer_surface.set_size(0, 0);
    layer_surface.set_exclusive_zone(0);
    layer_surface.set_keyboard_interactivity(
        zwlr_layer_surface_v1::KeyboardInteractivity::OnDemand,
    );
    surface.commit();

    state.surface = Some(surface.clone());
    state.layer_surface = Some(layer_surface);

    // Wait for initial configure
    while !state.configured {
        event_queue.blocking_dispatch(&mut state)?;
    }
    state.configured = false;

    let scale_f = state.fractional_scale() as f32;
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
    let gpu_ctx = GpuContext::from_window(&wl_handle, phys_w, phys_h)
        .map_err(|e| anyhow!("GPU init failed: {e}"))?;
    let mut gpu = Gpu {
        painter: Painter::new(&gpu_ctx),
        text: TextRenderer::new(&gpu_ctx),
        mono_text: TextRenderer::new_monospace(&gpu_ctx),
        tex_pass: TexturePass::new(&gpu_ctx),
        ctx: gpu_ctx,
    };

    let palette = FoxPalette::dark();
    let mut view_menu = ContextMenu::new(ContextMenuStyle::from_palette(&palette));
    let mut context_menu = ContextMenu::new(ContextMenuStyle::from_palette(&palette));
    view_menu.set_scale(scale_f);
    context_menu.set_scale(scale_f);
    let mut open_with_apps: Vec<DesktopApp> = Vec::new();

    let mut app = App::new();
    app.icon_zoom = settings.icon_zoom;
    app.show_hidden = settings.show_hidden;
    app.sort_by = settings.sort_by_enum();
    app.navigate_to_home();
    // Restore pinned tabs from settings (preserving saved order)
    let mut pinned: Vec<crate::app::DirectoryTab> = Vec::new();
    for pinned_path in &settings.pinned_tabs {
        let path = std::path::PathBuf::from(pinned_path);
        if path.is_dir() {
            let mut tab = crate::app::DirectoryTab::new(path.clone());
            tab.pinned = true;
            tab.pinned_path = Some(path.clone());
            tab.entries = crate::fs::list_directory(&path, app.show_hidden, app.sort_by);
            pinned.push(tab);
        }
    }
    if !pinned.is_empty() {
        // Prepend pinned tabs before the home tab
        pinned.append(&mut app.tabs);
        app.tabs = pinned;
        app.current_tab = 0;
        app.switch_tab(0);
    }

    let mut input = InteractionContext::new();
    let mut icon_cache = IconCache::new();
    let mut file_info = crate::file_info::FileInfoCache::new();
    let mut settings = settings;

    crate::wayland_loop::run_loop(
        &conn, &mut event_queue, &mut state, &qh,
        &surface, &viewport,
        &mut gpu, &palette, &mut view_menu, &mut context_menu,
        &mut open_with_apps, &mut app, &mut input, &mut icon_cache,
        &mut file_info, &mut settings,
    )?;

    eprintln!("[desktop] exited main loop");

    // Save settings on exit
    settings.icon_zoom = app.icon_zoom;
    settings.show_hidden = app.show_hidden;
    settings.set_sort_by(app.sort_by);
    settings.pinned_tabs = app.tabs.iter()
        .filter(|t| t.pinned)
        .map(|t| {
            t.pinned_path.as_ref().unwrap_or(&t.path)
                .to_string_lossy().to_string()
        })
        .collect();
    settings.save();

    Ok(())
}

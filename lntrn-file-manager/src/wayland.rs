use std::ffi::c_void;
use std::ptr::NonNull;

use anyhow::{anyhow, Result};
use lntrn_render::{GpuContext, Painter, TextRenderer, TexturePass};
use lntrn_ui::gpu::{
    ContextMenu, ContextMenuStyle, FoxPalette, InteractionContext,
    WaylandPopupBackend,
};
use raw_window_handle::{
    DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, RawDisplayHandle,
    RawWindowHandle, WaylandDisplayHandle, WaylandWindowHandle, WindowHandle,
};
use wayland_client::{
    protocol::{
        wl_compositor, wl_data_device, wl_data_device_manager,
        wl_seat, wl_surface,
    },
    Connection, EventQueue, Proxy,
};
use wayland_protocols::wp::viewporter::client::wp_viewporter;
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};
use wayland_protocols_wlr::layer_shell::v1::client::{zwlr_layer_shell_v1, zwlr_layer_surface_v1};

use crate::app::App;
use crate::desktop::DesktopApp;
use crate::icons::IconCache;
use crate::{Gpu, PickConfig, PickResult};
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
    pub(crate) output_logical_width: u32,
    pub(crate) maximized: bool,
    pub(crate) desktop_mode: bool,
    // Wayland objects
    pub(crate) compositor: Option<wl_compositor::WlCompositor>,
    pub(crate) wm_base: Option<xdg_wm_base::XdgWmBase>,
    pub(crate) viewporter: Option<wp_viewporter::WpViewporter>,
    pub(crate) surface: Option<wl_surface::WlSurface>,
    pub(crate) xdg_surface: Option<xdg_surface::XdgSurface>,
    pub(crate) toplevel: Option<xdg_toplevel::XdgToplevel>,
    pub(crate) seat: Option<wl_seat::WlSeat>,
    // Layer shell (desktop widget mode)
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
    pub(crate) key_pressed: Option<u32>,
    // Key repeat
    pub(crate) held_key: Option<u32>,
    pub(crate) repeat_deadline: std::time::Instant,
    pub(crate) repeat_started: bool,
    // Popups
    pub(crate) popup_backend: Option<WaylandPopupBackend<State>>,
    pub(crate) popup_closed: bool,
    // DnD
    pub(crate) data_device_manager: Option<wl_data_device_manager::WlDataDeviceManager>,
    pub(crate) data_device: Option<wl_data_device::WlDataDevice>,
    pub(crate) dnd_active: bool,
    pub(crate) dnd_paths: Vec<std::path::PathBuf>,
    pub(crate) dnd_serial: u32,
    pub(crate) dnd_over_self: bool,
    pub(crate) dnd_drop_on_self: bool,
    pub(crate) dnd_cursor_x: f64,
    pub(crate) dnd_cursor_y: f64,
}

impl State {
    fn new() -> Self {
        Self {
            running: true, configured: false, frame_done: true,
            width: 0, height: 0, scale: 1, output_phys_width: 0, output_logical_width: 0, maximized: false,
            desktop_mode: false,
            compositor: None, wm_base: None, viewporter: None,
            surface: None, xdg_surface: None, toplevel: None, seat: None,
            layer_shell: None, layer_surface: None,
            cursor_x: 0.0, cursor_y: 0.0, pointer_in_surface: false,
            left_pressed: false, left_released: false, right_clicked: false,
            scroll_delta: 0.0, pointer_serial: 0, pointer_surface: None,
            ctrl: false, shift: false, key_pressed: None,
            held_key: None, repeat_deadline: std::time::Instant::now(), repeat_started: false,
            popup_backend: None, popup_closed: false,
            data_device_manager: None, data_device: None,
            dnd_active: false, dnd_paths: Vec::new(), dnd_serial: 0,
            dnd_over_self: false, dnd_drop_on_self: false,
            dnd_cursor_x: 0.0, dnd_cursor_y: 0.0,
        }
    }

    pub(crate) fn fractional_scale(&self) -> f64 {
        if self.output_phys_width > 0 && self.output_logical_width > 0 {
            self.output_phys_width as f64 / self.output_logical_width as f64
        } else {
            self.scale.max(1) as f64
        }
    }

    pub(crate) fn phys_width(&self) -> u32 { (self.width as f64 * self.fractional_scale()).round() as u32 }
    pub(crate) fn phys_height(&self) -> u32 { (self.height as f64 * self.fractional_scale()).round() as u32 }
}

// ── Entry point ─────────────────────────────────────────────────────────────

pub fn run(pick: Option<PickConfig>, desktop: bool) -> Result<()> {
    if desktop {
        crate::layout::DESKTOP_MODE.store(true, std::sync::atomic::Ordering::Relaxed);
    }
    let conn = Connection::connect_to_env()?;
    let display = conn.display();
    let mut event_queue: EventQueue<State> = conn.new_event_queue();
    let qh = event_queue.handle();
    let mut state = State::new();
    state.desktop_mode = desktop;

    display.get_registry(&qh, ());
    event_queue.roundtrip(&mut state)?;

    let compositor = state.compositor.clone()
        .ok_or_else(|| anyhow!("wl_compositor not available"))?;

    // Create data device for DnD (if available)
    if let (Some(mgr), Some(seat)) = (&state.data_device_manager, &state.seat) {
        state.data_device = Some(mgr.get_data_device(seat, &qh, ()));
    }

    let settings = Settings::load();
    let surface = compositor.create_surface(&qh, ());

    // Dummy toplevel ref — only used in window mode
    let mut toplevel_holder: Option<xdg_toplevel::XdgToplevel> = None;

    if desktop {
        // ── Layer shell desktop widget mode ─────────────────────────
        let layer_shell = state.layer_shell.clone()
            .ok_or_else(|| anyhow!("zwlr_layer_shell_v1 not available"))?;

        let layer_surface = layer_shell.get_layer_surface(
            &surface, None,
            zwlr_layer_shell_v1::Layer::Bottom,
            "lntrn-file-manager-desktop".to_string(),
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
    } else {
        // ── Normal xdg_toplevel window mode ─────────────────────────
        let wm_base = state.wm_base.clone()
            .ok_or_else(|| anyhow!("xdg_wm_base not available"))?;

        if state.width == 0 { state.width = settings.window_width as u32; }
        if state.height == 0 { state.height = settings.window_height as u32; }

        let xdg_surface = wm_base.get_xdg_surface(&surface, &qh, ());
        let toplevel = xdg_surface.get_toplevel(&qh, ());
        let title = pick.as_ref()
            .and_then(|p| p.title.as_deref())
            .or(pick.as_ref().map(|p| p.default_title()))
            .unwrap_or("Lantern File Manager");
        toplevel.set_title(title.into());
        toplevel.set_app_id("lntrn-file-manager".into());
        toplevel.set_min_size(400, 300);
        surface.commit();

        toplevel_holder = Some(toplevel.clone());
        state.surface = Some(surface.clone());
        state.xdg_surface = Some(xdg_surface);
        state.toplevel = Some(toplevel);
    }

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
        tex_pass: TexturePass::new(&gpu_ctx),
        ctx: gpu_ctx,
    };

    // Popup backend (window mode only — desktop uses inline menus)
    if !desktop {
        let xdg_surf = state.xdg_surface.as_ref().unwrap().clone();
        let wm_base = state.wm_base.as_ref().unwrap();
        let vp = state.viewporter.as_ref();
        state.popup_backend = Some(WaylandPopupBackend::new(
            &conn, &compositor, wm_base, &xdg_surf, vp, &gpu.ctx, scale_f, &qh,
        ));
    }

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
    if let Some(ref p) = pick {
        if let Some(ref dir) = p.start_dir {
            app.navigate_to(dir.clone());
        } else {
            app.navigate_to_home();
        }
        app.pick = Some(p.clone());
        if let Some(ref name) = p.save_name {
            app.save_name_buf = name.clone();
        }
    } else {
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
    }

    let mut input = InteractionContext::new();
    let mut icon_cache = IconCache::new();
    let mut file_info = crate::file_info::FileInfoCache::new();
    let mut settings = settings;

    crate::wayland_loop::run_loop(
        &conn, &mut event_queue, &mut state, &qh,
        &surface, &toplevel_holder, &viewport,
        &mut gpu, &palette, &mut view_menu, &mut context_menu,
        &mut open_with_apps, &mut app, &mut input, &mut icon_cache,
        &mut file_info, &mut settings,
    )?;

    eprintln!("[fox] exited main loop");

    // Pick mode: output results
    if pick.is_some() {
        match app.pick_result.take() {
            Some(PickResult::Selected(paths)) => {
                for p in &paths {
                    println!("{}", p.display());
                }
                return Ok(());
            }
            _ => {
                // Cancelled — exit with code 1
                std::process::exit(1);
            }
        }
    }

    // Save settings on exit (normal mode only)
    settings.icon_zoom = app.icon_zoom;
    settings.show_hidden = app.show_hidden;
    settings.set_sort_by(app.sort_by);
    settings.window_width = state.width as f32;
    settings.window_height = state.height as f32;
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

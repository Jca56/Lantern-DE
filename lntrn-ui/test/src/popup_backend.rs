use std::collections::HashMap;
use std::ffi::c_void;
use std::ptr::NonNull;
use std::sync::Arc;

use lntrn_render::{GpuContext, Painter, TextRenderer};
use lntrn_ui::gpu::{InteractionContext, PopupRenderContext, PopupSurface};
use raw_window_handle::{
    DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, RawDisplayHandle,
    RawWindowHandle, WaylandDisplayHandle, WaylandWindowHandle, WindowHandle,
};
use wayland_client::{
    protocol::{wl_compositor, wl_surface},
    Connection, Dispatch, Proxy, QueueHandle,
};
use wayland_protocols::wp::viewporter::client::{wp_viewport, wp_viewporter};
use wayland_protocols::xdg::shell::client::{xdg_popup, xdg_positioner, xdg_surface, xdg_wm_base};

use super::wayland::State;

struct PopupWaylandHandle {
    display: NonNull<c_void>,
    surface: NonNull<c_void>,
}
impl HasDisplayHandle for PopupWaylandHandle {
    fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
        let raw = RawDisplayHandle::Wayland(WaylandDisplayHandle::new(self.display));
        Ok(unsafe { DisplayHandle::borrow_raw(raw) })
    }
}
impl HasWindowHandle for PopupWaylandHandle {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        let raw = RawWindowHandle::Wayland(WaylandWindowHandle::new(self.surface));
        Ok(unsafe { WindowHandle::borrow_raw(raw) })
    }
}

struct PopupEntry {
    wl_surface: wl_surface::WlSurface,
    xdg_surface: xdg_surface::XdgSurface,
    xdg_popup: xdg_popup::XdgPopup,
    viewport: Option<wp_viewport::WpViewport>,
    render: PopupRenderContext,
    configured: bool,
    logical_w: u32,
    logical_h: u32,
}

pub struct WaylandPopupBackend {
    popups: HashMap<u32, PopupEntry>,
    next_id: u32,
    display_ptr: NonNull<c_void>,
    compositor: wl_compositor::WlCompositor,
    wm_base: xdg_wm_base::XdgWmBase,
    parent_xdg_surface: xdg_surface::XdgSurface,
    parent_instance: Arc<wgpu::Instance>,
    parent_device: Arc<wgpu::Device>,
    parent_queue: Arc<wgpu::Queue>,
    parent_format: wgpu::TextureFormat,
    viewporter: Option<wp_viewporter::WpViewporter>,
    scale: f32,
    qh: QueueHandle<State>,
}

impl WaylandPopupBackend {
    pub fn new(
        conn: &Connection,
        compositor: &wl_compositor::WlCompositor,
        wm_base: &xdg_wm_base::XdgWmBase,
        parent_xdg_surface: &xdg_surface::XdgSurface,
        viewporter: Option<&wp_viewporter::WpViewporter>,
        parent_gpu: &GpuContext,
        scale: f32,
        qh: &QueueHandle<State>,
    ) -> Self {
        let display_ptr = NonNull::new(conn.backend().display_ptr() as *mut c_void)
            .expect("null wl_display");
        Self {
            popups: HashMap::new(),
            next_id: 1,
            display_ptr,
            compositor: compositor.clone(),
            wm_base: wm_base.clone(),
            parent_xdg_surface: parent_xdg_surface.clone(),
            parent_instance: parent_gpu.instance_arc(),
            parent_device: parent_gpu.device_arc(),
            parent_queue: parent_gpu.queue_arc(),
            parent_format: parent_gpu.format,
            viewporter: viewporter.cloned(),
            scale,
            qh: qh.clone(),
        }
    }

    pub fn find_popup_id_by_wl_surface(&self, surface: &wl_surface::WlSurface) -> Option<u32> {
        self.popups.iter()
            .find(|(_, p)| p.wl_surface == *surface)
            .map(|(&id, _)| id)
    }

    pub fn begin_frame_all(&mut self) {
        for entry in self.popups.values_mut() {
            if !entry.configured { continue; }
            entry.render.interaction.begin_frame();
            entry.render.painter.clear();
        }
    }

    pub fn render_all(&mut self) {
        for entry in self.popups.values_mut() {
            if !entry.configured { continue; }
            let ctx = &mut entry.render;
            let gpu = &ctx.gpu;
            if let Ok(mut frame) = gpu.begin_frame("popup") {
                let view = frame.view().clone();
                ctx.painter.render_pass(
                    gpu, frame.encoder_mut(), &view,
                    lntrn_render::Color::TRANSPARENT,
                );
                ctx.text.render_queued(gpu, frame.encoder_mut(), &view);
                frame.submit(&gpu.queue);
            }
            entry.wl_surface.commit();
        }
    }

    pub fn mark_configured(&mut self, id: u32) {
        if let Some(p) = self.popups.get_mut(&id) {
            p.configured = true;
        }
    }

    pub fn configure_size(&mut self, id: u32, width: u32, height: u32) {
        if let Some(p) = self.popups.get_mut(&id) {
            if width > 0 && height > 0 {
                let phys_w = ((width as f32) * self.scale).ceil() as u32;
                let phys_h = ((height as f32) * self.scale).ceil() as u32;
                p.render.gpu.resize(phys_w.max(1), phys_h.max(1));
            }
        }
    }
}

impl PopupSurface for WaylandPopupBackend {
    fn create_popup(&mut self, parent_popup: Option<u32>, parent_x: i32, parent_y: i32, width: u32, height: u32) -> u32 {
        let id = self.next_id;
        self.next_id += 1;

        let wl_surface = self.compositor.create_surface(&self.qh, ());
        let xdg_surface = self.wm_base.get_xdg_surface(&wl_surface, &self.qh, id);

        let positioner = self.wm_base.create_positioner(&self.qh, ());
        positioner.set_size(width as i32, height as i32);
        positioner.set_anchor_rect(parent_x, parent_y, 1, 1);
        positioner.set_anchor(xdg_positioner::Anchor::BottomRight);
        positioner.set_gravity(xdg_positioner::Gravity::BottomRight);
        positioner.set_constraint_adjustment(
            xdg_positioner::ConstraintAdjustment::FlipX
                | xdg_positioner::ConstraintAdjustment::FlipY
                | xdg_positioner::ConstraintAdjustment::SlideX
                | xdg_positioner::ConstraintAdjustment::SlideY,
        );

        // Parent to another popup's xdg_surface, or the main window's
        let parent_xdg = parent_popup
            .and_then(|pid| self.popups.get(&pid).map(|p| p.xdg_surface.clone()))
            .unwrap_or_else(|| self.parent_xdg_surface.clone());

        let xdg_popup = xdg_surface.get_popup(
            Some(&parent_xdg), &positioner, &self.qh, id,
        );
        positioner.destroy();

        // Set up scaling — viewporter for fractional, fallback to buffer_scale
        let viewport = if let Some(vp) = &self.viewporter {
            let viewport = vp.get_viewport(&wl_surface, &self.qh, ());
            viewport.set_destination(width as i32, height as i32);
            wl_surface.set_buffer_scale(1);
            Some(viewport)
        } else {
            wl_surface.set_buffer_scale(self.scale.round() as i32);
            None
        };
        wl_surface.commit();

        // Create GPU context sharing parent device/queue
        let surface_ptr = Proxy::id(&wl_surface).as_ptr() as *mut c_void;
        let wl_handle = PopupWaylandHandle {
            display: self.display_ptr,
            surface: NonNull::new(surface_ptr).expect("null popup wl_surface"),
        };

        // GPU surface needs physical pixels
        let phys_w = ((width as f32) * self.scale).ceil() as u32;
        let phys_h = ((height as f32) * self.scale).ceil() as u32;
        let gpu = GpuContext::from_parent_shared(
            Arc::clone(&self.parent_instance),
            Arc::clone(&self.parent_device),
            Arc::clone(&self.parent_queue),
            self.parent_format,
            &wl_handle,
            phys_w.max(1),
            phys_h.max(1),
        ).expect("popup GPU init failed");

        let painter = Painter::new(&gpu);
        let text = TextRenderer::new(&gpu);

        self.popups.insert(id, PopupEntry {
            wl_surface,
            xdg_surface,
            xdg_popup,
            viewport,
            render: PopupRenderContext {
                gpu,
                painter,
                text,
                interaction: InteractionContext::new(),
            },
            configured: false,
            logical_w: width,
            logical_h: height,
        });

        id
    }

    fn resize_popup(&mut self, id: u32, width: u32, height: u32) {
        if let Some(p) = self.popups.get_mut(&id) {
            let phys_w = ((width as f32) * self.scale).ceil() as u32;
            let phys_h = ((height as f32) * self.scale).ceil() as u32;
            p.render.gpu.resize(phys_w.max(1), phys_h.max(1));
        }
    }

    fn destroy_popup(&mut self, id: u32) {
        if let Some(entry) = self.popups.remove(&id) {
            entry.xdg_popup.destroy();
            entry.xdg_surface.destroy();
            entry.wl_surface.destroy();
        }
    }

    fn popup_render(&mut self, id: u32) -> Option<&mut PopupRenderContext> {
        self.popups.get_mut(&id)
            .filter(|p| p.configured)
            .map(|p| &mut p.render)
    }
}

// ── Dispatch impls for popup protocol objects ─────────────────────────────

impl Dispatch<xdg_positioner::XdgPositioner, ()> for State {
    fn event(
        _: &mut Self, _: &xdg_positioner::XdgPositioner,
        _: xdg_positioner::Event, _: &(), _: &Connection, _: &QueueHandle<Self>,
    ) {}
}

impl Dispatch<xdg_surface::XdgSurface, u32> for State {
    fn event(
        state: &mut Self, xdg_surface: &xdg_surface::XdgSurface,
        event: xdg_surface::Event, popup_id: &u32, _: &Connection, _: &QueueHandle<Self>,
    ) {
        if let xdg_surface::Event::Configure { serial } = event {
            xdg_surface.ack_configure(serial);
            if let Some(backend) = &mut state.popup_backend {
                backend.mark_configured(*popup_id);
            }
            state.frame_done = true;
        }
    }
}

impl Dispatch<xdg_popup::XdgPopup, u32> for State {
    fn event(
        state: &mut Self, _: &xdg_popup::XdgPopup,
        event: xdg_popup::Event, popup_id: &u32, _: &Connection, _: &QueueHandle<Self>,
    ) {
        match event {
            xdg_popup::Event::Configure { width, height, .. } => {
                if let Some(backend) = &mut state.popup_backend {
                    backend.configure_size(*popup_id, width as u32, height as u32);
                }
            }
            xdg_popup::Event::PopupDone => {
                state.popup_closed = true;
            }
            _ => {}
        }
    }
}

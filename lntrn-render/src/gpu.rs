use std::sync::Arc;
use std::ffi::c_void;
use std::ptr::NonNull;

use raw_window_handle::{
    HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle,
    XlibDisplayHandle, XlibWindowHandle,
};

// ── X11 surface wrapper ──────────────────────────────────────────────────────

struct X11Surface {
    display: *mut c_void,
    window: u64,
}

unsafe impl Send for X11Surface {}
unsafe impl Sync for X11Surface {}

impl HasDisplayHandle for X11Surface {
    fn display_handle(
        &self,
    ) -> Result<raw_window_handle::DisplayHandle<'_>, raw_window_handle::HandleError> {
        let handle = XlibDisplayHandle::new(NonNull::new(self.display), 0);
        Ok(unsafe { raw_window_handle::DisplayHandle::borrow_raw(RawDisplayHandle::Xlib(handle)) })
    }
}

impl HasWindowHandle for X11Surface {
    fn window_handle(
        &self,
    ) -> Result<raw_window_handle::WindowHandle<'_>, raw_window_handle::HandleError> {
        let handle = XlibWindowHandle::new(self.window.try_into().unwrap());
        Ok(unsafe { raw_window_handle::WindowHandle::borrow_raw(RawWindowHandle::Xlib(handle)) })
    }
}

// ── GPU Context ──────────────────────────────────────────────────────────────

pub struct GpuContext {
    pub device: Arc<wgpu::Device>,
    pub queue: Arc<wgpu::Queue>,
    pub surface: wgpu::Surface<'static>,
    pub config: wgpu::SurfaceConfiguration,
    pub format: wgpu::TextureFormat,
    xlib_display: *mut c_void,
}

unsafe impl Send for GpuContext {}

impl GpuContext {
    pub fn new(x11_window: u32, width: u32, height: u32) -> Result<Self, Box<dyn std::error::Error>> {
        let xlib_display = unsafe { x11::xlib::XOpenDisplay(std::ptr::null()) };
        if xlib_display.is_null() {
            return Err("Failed to open Xlib display".into());
        }

        let x11_surface = X11Surface {
            display: xlib_display as *mut c_void,
            window: x11_window as u64,
        };

        let (device, queue, surface, config, format) = create_surface_context(&x11_surface, width, height)?;

        Ok(Self {
            device: Arc::new(device),
            queue: Arc::new(queue),
            surface,
            config,
            format,
            xlib_display: xlib_display as *mut c_void,
        })
    }

    pub fn from_window<W>(window: &W, width: u32, height: u32) -> Result<Self, Box<dyn std::error::Error>>
    where
        W: HasDisplayHandle + HasWindowHandle,
    {
        let (device, queue, surface, config, format) = create_surface_context(window, width, height)?;

        Ok(Self {
            device: Arc::new(device),
            queue: Arc::new(queue),
            surface,
            config,
            format,
            xlib_display: std::ptr::null_mut(),
        })
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        if self.config.width == width && self.config.height == height {
            return;
        }
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&self.device, &self.config);
    }

    pub fn width(&self) -> u32 {
        self.config.width
    }

    pub fn height(&self) -> u32 {
        self.config.height
    }
}

impl Drop for GpuContext {
    fn drop(&mut self) {
        if !self.xlib_display.is_null() {
            unsafe { x11::xlib::XCloseDisplay(self.xlib_display as *mut _) };
        }
    }
}

fn create_surface_context<W>(
    window: &W,
    width: u32,
    height: u32,
) -> Result<
    (
        wgpu::Device,
        wgpu::Queue,
        wgpu::Surface<'static>,
        wgpu::SurfaceConfiguration,
        wgpu::TextureFormat,
    ),
    Box<dyn std::error::Error>,
>
where
    W: HasDisplayHandle + HasWindowHandle,
{
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
        backends: wgpu::Backends::PRIMARY | wgpu::Backends::GL,
        ..Default::default()
    });

    let surface = unsafe {
        instance.create_surface_unsafe(wgpu::SurfaceTargetUnsafe::from_window(window)?)
    }?;

    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: Some(&surface),
        force_fallback_adapter: false,
    }))?;

    let (device, queue) = pollster::block_on(adapter.request_device(
        &wgpu::DeviceDescriptor {
            label: Some("Lantern Render"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::default(),
            ..Default::default()
        },
    ))?;

    let caps = surface.get_capabilities(&adapter);
    let format = caps
        .formats
        .iter()
        .find(|f| f.is_srgb())
        .copied()
        .unwrap_or(caps.formats[0]);

    let present_mode = if caps.present_modes.contains(&wgpu::PresentMode::Mailbox) {
        wgpu::PresentMode::Mailbox
    } else {
        caps.present_modes[0]
    };

    let alpha_mode = if caps.alpha_modes.contains(&wgpu::CompositeAlphaMode::PreMultiplied) {
        wgpu::CompositeAlphaMode::PreMultiplied
    } else if caps.alpha_modes.contains(&wgpu::CompositeAlphaMode::PostMultiplied) {
        wgpu::CompositeAlphaMode::PostMultiplied
    } else if caps.alpha_modes.contains(&wgpu::CompositeAlphaMode::Inherit) {
        wgpu::CompositeAlphaMode::Inherit
    } else {
        caps.alpha_modes[0]
    };
    tracing::info!("wgpu alpha_mode: {:?} (available: {:?})", alpha_mode, caps.alpha_modes);

    let config = wgpu::SurfaceConfiguration {
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        format,
        width,
        height,
        present_mode,
        alpha_mode,
        view_formats: vec![],
        desired_maximum_frame_latency: 2,
    };
    surface.configure(&device, &config);

    Ok((device, queue, surface, config, format))
}

use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;
use std::ffi::c_void;
use std::ptr::NonNull;

use raw_window_handle::{
    HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle,
    XlibDisplayHandle, XlibWindowHandle,
};

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

pub struct GpuContext {
    pub device: Arc<wgpu::Device>,
    pub queue: Arc<wgpu::Queue>,
    pub surface: wgpu::Surface<'static>,
    pub config: wgpu::SurfaceConfiguration,
    pub format: wgpu::TextureFormat,
    instance: Arc<wgpu::Instance>,
    timing: Arc<Mutex<FrameTimingSnapshot>>,
    xlib_display: *mut c_void,
}

pub struct Frame {
    output: wgpu::SurfaceTexture,
    view: wgpu::TextureView,
    encoder: wgpu::CommandEncoder,
    label: &'static str,
    started_at: Instant,
    acquire_micros: u128,
    timing: Arc<Mutex<FrameTimingSnapshot>>,
}

#[derive(Clone, Debug, Default)]
pub struct FrameTimingSnapshot {
    pub label: &'static str,
    pub acquire_micros: u128,
    pub encode_micros: u128,
    pub submit_micros: u128,
    pub present_micros: u128,
    pub total_micros: u128,
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

        let (instance, device, queue, surface, config, format) =
            create_surface_context(&x11_surface, width, height)?;

        Ok(Self {
            device: Arc::new(device),
            queue: Arc::new(queue),
            surface,
            config,
            format,
            instance: Arc::new(instance),
            timing: Arc::new(Mutex::new(FrameTimingSnapshot::default())),
            xlib_display: xlib_display as *mut c_void,
        })
    }

    pub fn from_window<W>(window: &W, width: u32, height: u32) -> Result<Self, Box<dyn std::error::Error>>
    where
        W: HasDisplayHandle + HasWindowHandle,
    {
        let (instance, device, queue, surface, config, format) = create_surface_context(window, width, height)?;

        Ok(Self {
            device: Arc::new(device),
            queue: Arc::new(queue),
            surface,
            config,
            format,
            instance: Arc::new(instance),
            timing: Arc::new(Mutex::new(FrameTimingSnapshot::default())),
            xlib_display: std::ptr::null_mut(),
        })
    }

    pub fn from_parent<W>(
        parent: &GpuContext,
        window: &W,
        width: u32,
        height: u32,
    ) -> Result<Self, Box<dyn std::error::Error>>
    where
        W: HasDisplayHandle + HasWindowHandle,
    {
        let surface = unsafe {
            parent.instance.create_surface_unsafe(wgpu::SurfaceTargetUnsafe::from_window(window)?)
        }?;

        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: parent.format,
            width,
            height,
            present_mode: wgpu::PresentMode::Mailbox,
            alpha_mode: wgpu::CompositeAlphaMode::PreMultiplied,
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&parent.device, &config);

        Ok(Self {
            device: Arc::clone(&parent.device),
            queue: Arc::clone(&parent.queue),
            surface,
            config,
            format: parent.format,
            instance: Arc::clone(&parent.instance),
            timing: Arc::new(Mutex::new(FrameTimingSnapshot::default())),
            xlib_display: std::ptr::null_mut(),
        })
    }

    pub fn from_parent_shared<W>(
        instance: Arc<wgpu::Instance>,
        device: Arc<wgpu::Device>,
        queue: Arc<wgpu::Queue>,
        format: wgpu::TextureFormat,
        window: &W,
        width: u32,
        height: u32,
    ) -> Result<Self, Box<dyn std::error::Error>>
    where
        W: HasDisplayHandle + HasWindowHandle,
    {
        let surface = unsafe {
            instance.create_surface_unsafe(wgpu::SurfaceTargetUnsafe::from_window(window)?)
        }?;

        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width,
            height,
            present_mode: wgpu::PresentMode::Mailbox,
            alpha_mode: wgpu::CompositeAlphaMode::PreMultiplied,
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &config);

        Ok(Self {
            device,
            queue,
            surface,
            config,
            format,
            instance,
            timing: Arc::new(Mutex::new(FrameTimingSnapshot::default())),
            xlib_display: std::ptr::null_mut(),
        })
    }

    pub fn instance_arc(&self) -> Arc<wgpu::Instance> { Arc::clone(&self.instance) }
    pub fn device_arc(&self) -> Arc<wgpu::Device> { Arc::clone(&self.device) }
    pub fn queue_arc(&self) -> Arc<wgpu::Queue> { Arc::clone(&self.queue) }

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

    pub fn begin_frame(&self, label: &'static str) -> Result<Frame, wgpu::SurfaceError> {
        let acquire_started = Instant::now();
        let output = self.surface.get_current_texture()?;
        let view = output
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some(label) });
        let acquire_micros = acquire_started.elapsed().as_micros();

        if timing_enabled() {
            eprintln!(
                "[lntrn-gfx] frame={} acquire_us={} size={}x{}",
                label,
                acquire_micros,
                self.width(),
                self.height()
            );
        }

        Ok(Frame {
            output,
            view,
            encoder,
            label,
            started_at: Instant::now(),
            acquire_micros,
            timing: Arc::clone(&self.timing),
        })
    }

    pub fn timing_snapshot(&self) -> FrameTimingSnapshot {
        self.timing.lock().map(|snapshot| snapshot.clone()).unwrap_or_default()
    }

    pub fn width(&self) -> u32 {
        self.config.width
    }

    pub fn height(&self) -> u32 {
        self.config.height
    }
}

impl Frame {
    pub fn view(&self) -> &wgpu::TextureView {
        &self.view
    }

    pub fn encoder_mut(&mut self) -> &mut wgpu::CommandEncoder {
        &mut self.encoder
    }

    pub fn submit(self, queue: &wgpu::Queue) {
        let submit_started = Instant::now();
        queue.submit(std::iter::once(self.encoder.finish()));
        let submit_micros = submit_started.elapsed().as_micros();

        let present_started = Instant::now();
        self.output.present();
        let present_micros = present_started.elapsed().as_micros();
        let encode_micros = self.started_at.elapsed().as_micros();
        let total_micros = encode_micros + self.acquire_micros;

        if let Ok(mut snapshot) = self.timing.lock() {
            *snapshot = FrameTimingSnapshot {
                label: self.label,
                acquire_micros: self.acquire_micros,
                encode_micros,
                submit_micros,
                present_micros,
                total_micros,
            };
        }

        if timing_enabled() {
            eprintln!(
                "[lntrn-gfx] frame={} encode_us={} submit_us={} present_us={} total_us={} acquire_us={}",
                self.label,
                encode_micros,
                submit_micros,
                present_micros,
                total_micros,
                self.acquire_micros,
            );
        }
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
        wgpu::Instance,
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
        backends: wgpu::Backends::VULKAN,
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
            label: Some("Lantern Gfx"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::default(),
            ..Default::default()
        },
    ))?;

    let caps = surface.get_capabilities(&adapter);
    let format = caps
        .formats
        .iter()
        .find(|candidate| candidate.is_srgb())
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
    eprintln!("[lntrn-gfx] alpha_mode={:?} available={:?} format={:?}", alpha_mode, caps.alpha_modes, format);

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

    Ok((instance, device, queue, surface, config, format))
}

fn timing_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| {
        std::env::var("LNTRN_GFX_TIMING")
            .map(|value| value != "0")
            .unwrap_or(false)
    })
}
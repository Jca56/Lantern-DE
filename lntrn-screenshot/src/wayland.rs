//! Layer-shell client for the screenshot UI.
//!
//! Why layer-shell instead of an xdg_toplevel: a regular toplevel would
//! (a) sit *below* the Command Center's overlay layer surface — so CC
//! would steal focus and Ctrl+C/Enter would never reach us — and
//! (b) be subject to the compositor's Super+drag move gesture, which
//! lets the user accidentally drag the screenshot window around the
//! screen revealing the desktop underneath.
//!
//! We open a fullscreen overlay-layer surface and, once the screen has
//! been captured, give it `KeyboardInteractivity::Exclusive`, so we always
//! sit on top of every other surface (including CC and fullscreen windows)
//! and always receive keyboard focus while the screenshot UI is up.

use std::ffi::c_void;
use std::ptr::NonNull;

use anyhow::{anyhow, Result};
use raw_window_handle::{
    DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, RawDisplayHandle,
    RawWindowHandle, WaylandDisplayHandle, WaylandWindowHandle, WindowHandle,
};
use wayland_client::{
    backend::ObjectId,
    protocol::{wl_compositor, wl_output, wl_surface},
    Connection, EventQueue, Proxy, QueueHandle,
};
use wayland_protocols::wp::viewporter::client::{wp_viewport, wp_viewporter};
use wayland_protocols_wlr::layer_shell::v1::client::{zwlr_layer_shell_v1, zwlr_layer_surface_v1};


mod dispatch;
mod keyboard;

pub use keyboard::Typed;

/// Per-frame drained input state.
#[derive(Default)]
pub struct FrameInput {
    pub cursor_moved: bool,
    pub cursor_x: f32,
    pub cursor_y: f32,
    pub left_pressed: bool,
    pub left_released: bool,
    pub esc: bool,
    pub enter: bool,
    pub ctrl_c: bool,
    pub ctrl_s: bool,
    pub ctrl_z: bool,
    /// What was typed, in order, for the text tool.
    pub typed: Vec<Typed>,
}

/// Hands wgpu a `RawDisplayHandle` / `RawWindowHandle` pair pointing at
/// the bare `wl_display` and `wl_surface` pointers.
pub struct WaylandHandle {
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

/// Per-output state the screenshot client tracks so it can target the
/// monitor the layer surface actually entered (rather than blindly using
/// the first output advertised by the registry, which on multi-monitor
/// setups is usually the wrong one).
pub struct TrackedOutput {
    pub proxy: wl_output::WlOutput,
    pub name: Option<String>,
    pub scale: i32,
    pub mode_width: u32,
    pub mode_height: u32,
}

pub struct WlState {
    pub running: bool,
    pub configured: bool,
    pub frame_done: bool,
    /// Logical surface size (compositor units).
    pub width: u32,
    pub height: u32,

    /// Every wl_output advertised by the registry, with its mode/scale/name.
    pub outputs: Vec<TrackedOutput>,
    /// The wl_output id the layer surface entered. Populated by
    /// `wl_surface::Event::Enter` after the first commit, which is the
    /// definitive "which monitor am I on" signal from the compositor.
    pub entered_output_id: Option<ObjectId>,

    compositor: Option<wl_compositor::WlCompositor>,
    layer_shell: Option<zwlr_layer_shell_v1::ZwlrLayerShellV1>,
    viewporter: Option<wp_viewporter::WpViewporter>,

    cursor_x: f64,
    cursor_y: f64,
    cursor_dirty: bool,
    left_pressed_this_frame: bool,
    left_released_this_frame: bool,
    ctrl_held: bool,
    esc_pressed: bool,
    enter_pressed: bool,
    ctrl_c_pressed: bool,
    ctrl_s_pressed: bool,
    ctrl_z_pressed: bool,
    keyboard: keyboard::KeyboardState,
    typed: Vec<Typed>,
}

impl WlState {
    fn new() -> Self {
        Self {
            running: true,
            configured: false,
            frame_done: false,
            width: 0,
            height: 0,
            outputs: Vec::new(),
            entered_output_id: None,
            compositor: None,
            layer_shell: None,
            viewporter: None,
            cursor_x: 0.0,
            cursor_y: 0.0,
            cursor_dirty: false,
            left_pressed_this_frame: false,
            left_released_this_frame: false,
            ctrl_held: false,
            esc_pressed: false,
            enter_pressed: false,
            ctrl_c_pressed: false,
            ctrl_s_pressed: false,
            ctrl_z_pressed: false,
            keyboard: keyboard::KeyboardState::new(),
            typed: Vec::new(),
        }
    }

    /// The output the layer surface entered. None until the compositor
    /// has placed the surface.
    pub fn entered_output(&self) -> Option<&TrackedOutput> {
        let id = self.entered_output_id.as_ref()?;
        self.outputs.iter().find(|o| &o.proxy.id() == id)
    }

    pub fn entered_output_name(&self) -> Option<String> {
        self.entered_output().and_then(|o| o.name.clone())
    }

    pub fn fractional_scale(&self) -> f64 {
        if let Some(out) = self.entered_output() {
            if out.mode_width > 0 && self.width > 0 {
                return out.mode_width as f64 / self.width as f64;
            }
            return out.scale.max(1) as f64;
        }
        1.0
    }

    pub fn phys_width(&self) -> u32 {
        (self.width as f64 * self.fractional_scale()).round() as u32
    }
    pub fn phys_height(&self) -> u32 {
        (self.height as f64 * self.fractional_scale()).round() as u32
    }

    /// Drain accumulated input since the last frame. Coordinates returned
    /// are in *physical* pixels (cursor position multiplied by fractional
    /// scale) so callers can compare directly against the screenshot
    /// texture which is in physical pixels.
    pub fn take_frame_input(&mut self) -> FrameInput {
        let scale = self.fractional_scale() as f32;
        let repeats = self.keyboard.due_repeats();
        self.typed.extend(repeats);
        let out = FrameInput {
            cursor_moved: self.cursor_dirty,
            cursor_x: self.cursor_x as f32 * scale,
            cursor_y: self.cursor_y as f32 * scale,
            left_pressed: self.left_pressed_this_frame,
            left_released: self.left_released_this_frame,
            esc: self.esc_pressed,
            enter: self.enter_pressed,
            ctrl_c: self.ctrl_c_pressed,
            ctrl_s: self.ctrl_s_pressed,
            ctrl_z: self.ctrl_z_pressed,
            typed: std::mem::take(&mut self.typed),
        };
        self.cursor_dirty = false;
        self.left_pressed_this_frame = false;
        self.left_released_this_frame = false;
        self.esc_pressed = false;
        self.enter_pressed = false;
        self.ctrl_c_pressed = false;
        self.ctrl_s_pressed = false;
        self.ctrl_z_pressed = false;
        out
    }
}

/// The handle returned to `main` so it can drive the event loop and
/// hand wgpu the underlying surface pointer.
pub struct LayerWindow {
    pub conn: Connection,
    pub queue: EventQueue<WlState>,
    pub qh: QueueHandle<WlState>,
    pub state: WlState,
    pub surface: wl_surface::WlSurface,
    pub layer_surface: zwlr_layer_surface_v1::ZwlrLayerSurfaceV1,
    pub viewport: Option<wp_viewport::WpViewport>,
    pub handle: WaylandHandle,
}

impl LayerWindow {
    pub fn new() -> Result<Self> {
        let conn = Connection::connect_to_env()?;
        let display = conn.display();
        let mut queue: EventQueue<WlState> = conn.new_event_queue();
        let qh = queue.handle();
        let mut state = WlState::new();

        display.get_registry(&qh, ());
        queue.roundtrip(&mut state)?;

        let compositor = state
            .compositor
            .as_ref()
            .ok_or_else(|| anyhow!("wl_compositor not available"))?
            .clone();
        let layer_shell = state
            .layer_shell
            .as_ref()
            .ok_or_else(|| anyhow!("zwlr_layer_shell_v1 not available"))?
            .clone();

        let surface = compositor.create_surface(&qh, ());

        let layer_surface = layer_shell.get_layer_surface(
            &surface,
            None,
            zwlr_layer_shell_v1::Layer::Overlay,
            "lntrn-screenshot".to_string(),
            &qh,
            (),
        );
        {
            use zwlr_layer_surface_v1::Anchor;
            layer_surface.set_anchor(Anchor::Top | Anchor::Bottom | Anchor::Left | Anchor::Right);
            layer_surface.set_size(0, 0);
            layer_surface.set_exclusive_zone(-1);
            // No keyboard yet: `grab_keyboard` takes it once the screen has
            // been captured.
        }
        surface.commit();

        while !state.configured {
            queue.blocking_dispatch(&mut state)?;
        }
        if state.width == 0 {
            return Err(anyhow!("compositor sent zero-width configure"));
        }
        queue.roundtrip(&mut state)?;
        // Drain a second roundtrip so the wl_surface::enter event
        // (which the compositor emits when arranging the layer surface
        // onto the focused output) is delivered before the caller
        // queries `entered_output_name()`. Without this, capture may
        // race ahead and target the first-enumerated output instead.
        queue.roundtrip(&mut state)?;

        surface.set_buffer_scale(1);
        let viewport = state.viewporter.as_ref().map(|vp| {
            let v = vp.get_viewport(&surface, &qh, ());
            v.set_destination(state.width as i32, state.height as i32);
            v
        });

        let display_ptr = conn.backend().display_ptr() as *mut c_void;
        let surface_ptr = Proxy::id(&surface).as_ptr() as *mut c_void;
        let handle = WaylandHandle {
            display: NonNull::new(display_ptr).ok_or_else(|| anyhow!("null wl_display"))?,
            surface: NonNull::new(surface_ptr).ok_or_else(|| anyhow!("null wl_surface"))?,
        };

        // Mark frame_done so the first iteration draws.
        state.frame_done = true;

        Ok(Self {
            conn,
            queue,
            qh,
            state,
            surface,
            layer_surface,
            viewport,
            handle,
        })
    }

    /// Take exclusive keyboard focus, stealing it from CC / any other layer
    /// surface so Esc/Enter/Ctrl+C always reach the screenshot UI. Only done
    /// once the screen has been captured: the grab takes focus away from
    /// the window underneath, and a game that pauses on focus loss would
    /// otherwise get its pause menu into the screenshot.
    pub fn grab_keyboard(&self) {
        self.layer_surface
            .set_keyboard_interactivity(zwlr_layer_surface_v1::KeyboardInteractivity::Exclusive);
        self.surface.commit();
        let _ = self.conn.flush();
    }

    /// Block for the next batch of events. Returns when at least one
    /// event has been dispatched.
    pub fn dispatch(&mut self) -> Result<()> {
        self.queue.blocking_dispatch(&mut self.state)?;
        Ok(())
    }

    /// Request a frame callback so we know when the compositor is ready
    /// for the next buffer. Sets `frame_done = false` until the callback
    /// fires.
    pub fn request_frame(&mut self) {
        self.state.frame_done = false;
        let _ = self.surface.frame(&self.qh, ());
    }

    /// Name of the output the layer surface was placed on, if known.
    /// Used to target the same output for screencopy capture so the
    /// screenshot UI appears on the monitor the cursor was on.
    pub fn entered_output_name(&self) -> Option<String> {
        self.state.entered_output_name()
    }

    /// Tear down the layer surface so input grabs are released *before*
    /// we start serving the clipboard. Otherwise the compositor would
    /// keep keyboard focus pinned on our (now invisible) surface.
    pub fn destroy(self) {
        self.layer_surface.destroy();
        self.surface.destroy();
        let _ = self.conn.flush();
    }
}

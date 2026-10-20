//! What our Wayland connection hears: the monitors (their names and
//! sizes) and one screencopy frame at a time.

use std::time::Duration;

use wayland_client::globals::GlobalListContents;
use wayland_client::protocol::{wl_buffer, wl_output, wl_registry, wl_shm, wl_shm_pool};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle, WEnum};
use wayland_protocols_wlr::screencopy::v1::client::{zwlr_screencopy_frame_v1 as frame, zwlr_screencopy_manager_v1 as manager};

/// One monitor as the compositor announces it.
pub struct Out {
    pub proxy: wl_output::WlOutput,
    pub name: Option<String>,
    /// Its current mode, in pixels.
    pub size: (u32, u32),
}

/// The buffer a frame asks for: format, width, height, stride.
pub type Layout = (wl_shm::Format, u32, u32, u32);

/// What came of the frame in flight.
#[derive(Default)]
pub struct Frame {
    /// The first buffer it offered that we can fill.
    pub layout: Option<Layout>,
    pub buffer_done: bool,
    /// When the compositor finished the copy, since the epoch.
    pub ready: Option<Duration>,
    pub failed: bool,
    /// The rows came bottom first.
    pub flipped: bool,
}

#[derive(Default)]
pub struct State {
    pub outputs: Vec<Out>,
    pub frame: Frame,
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for State {
    fn event(_: &mut Self, _: &wl_registry::WlRegistry, _: wl_registry::Event, _: &GlobalListContents, _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<wl_output::WlOutput, ()> for State {
    fn event(state: &mut Self, output: &wl_output::WlOutput, event: wl_output::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        let Some(out) = state.outputs.iter_mut().find(|o| o.proxy.id() == output.id()) else { return };
        match event {
            wl_output::Event::Name { name } => out.name = Some(name),
            wl_output::Event::Mode { flags: WEnum::Value(flags), width, height, .. } if flags.contains(wl_output::Mode::Current) => {
                out.size = (width.max(0) as u32, height.max(0) as u32);
            }
            _ => {}
        }
    }
}

impl Dispatch<wl_shm::WlShm, ()> for State {
    fn event(_: &mut Self, _: &wl_shm::WlShm, _: wl_shm::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<wl_shm_pool::WlShmPool, ()> for State {
    fn event(_: &mut Self, _: &wl_shm_pool::WlShmPool, _: wl_shm_pool::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<wl_buffer::WlBuffer, ()> for State {
    fn event(_: &mut Self, _: &wl_buffer::WlBuffer, _: wl_buffer::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<manager::ZwlrScreencopyManagerV1, ()> for State {
    fn event(_: &mut Self, _: &manager::ZwlrScreencopyManagerV1, _: manager::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<frame::ZwlrScreencopyFrameV1, ()> for State {
    fn event(state: &mut Self, _: &frame::ZwlrScreencopyFrameV1, event: frame::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        let f = &mut state.frame;
        match event {
            frame::Event::Buffer { format: WEnum::Value(format), width, height, stride } => {
                if f.layout.is_none() && matches!(format, wl_shm::Format::Argb8888 | wl_shm::Format::Xrgb8888) {
                    f.layout = Some((format, width, height, stride));
                }
            }
            frame::Event::Flags { flags: WEnum::Value(flags) } => f.flipped = flags.contains(frame::Flags::YInvert),
            frame::Event::BufferDone => f.buffer_done = true,
            frame::Event::Ready { tv_sec_hi, tv_sec_lo, tv_nsec } => {
                let secs = (u64::from(tv_sec_hi) << 32) | u64::from(tv_sec_lo);
                f.ready = Some(Duration::new(secs, tv_nsec.min(999_999_999)));
            }
            frame::Event::Failed => f.failed = true,
            _ => {}
        }
    }
}

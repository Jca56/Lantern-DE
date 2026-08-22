//! Minimal 500×500 blank window for testing WM-tab sliders.
//!
//! Invoked via `lntrn-system-settings --test-window`. Renders a single dark
//! grey SHM buffer and runs until the user closes it. Critically it requests
//! ServerSide decorations so the user can see how the compositor's SSD looks
//! with their current `titlebar_height`, `corner_radius`, and `border_width`
//! settings — every regular Lantern app uses CSD, so they're invisible to
//! these sliders.

use std::io::{Seek, SeekFrom, Write};
use std::os::fd::AsFd;

use anyhow::{anyhow, Result};
use wayland_client::protocol::{
    wl_buffer::WlBuffer,
    wl_compositor::WlCompositor,
    wl_registry::{self, WlRegistry},
    wl_shm::{self, Format, WlShm},
    wl_shm_pool::WlShmPool,
    wl_surface::WlSurface,
};
use wayland_client::{Connection, Dispatch, EventQueue, QueueHandle};
use wayland_protocols::xdg::decoration::zv1::client::{
    zxdg_decoration_manager_v1::ZxdgDecorationManagerV1,
    zxdg_toplevel_decoration_v1::{self, Mode as DecorationMode, ZxdgToplevelDecorationV1},
};
use wayland_protocols::xdg::shell::client::{
    xdg_surface::{self, XdgSurface},
    xdg_toplevel::{self, XdgToplevel},
    xdg_wm_base::{self, XdgWmBase},
};

const W: i32 = 500;
const H: i32 = 500;
/// Matches Fox Dark palette `bg` (lntrn-theme::FOX_DARK = rgb(24, 24, 24)).
const BG: [u8; 4] = [0x18, 0x18, 0x18, 0xff];

struct State {
    compositor: Option<WlCompositor>,
    shm: Option<WlShm>,
    wm_base: Option<XdgWmBase>,
    decoration_mgr: Option<ZxdgDecorationManagerV1>,
    configured: bool,
    running: bool,
}

impl State {
    fn new() -> Self {
        Self {
            compositor: None,
            shm: None,
            wm_base: None,
            decoration_mgr: None,
            configured: false,
            running: true,
        }
    }
}

pub fn run() -> Result<()> {
    let conn = Connection::connect_to_env()?;
    let display = conn.display();
    let mut queue: EventQueue<State> = conn.new_event_queue();
    let qh = queue.handle();

    let _registry = display.get_registry(&qh, ());
    let mut state = State::new();
    queue.roundtrip(&mut state)?;

    let compositor = state
        .compositor
        .clone()
        .ok_or_else(|| anyhow!("no wl_compositor"))?;
    let shm = state.shm.clone().ok_or_else(|| anyhow!("no wl_shm"))?;
    let wm_base = state
        .wm_base
        .clone()
        .ok_or_else(|| anyhow!("no xdg_wm_base"))?;

    let surface = compositor.create_surface(&qh, ());
    let xdg_surface = wm_base.get_xdg_surface(&surface, &qh, ());
    let toplevel = xdg_surface.get_toplevel(&qh, ());
    toplevel.set_title("Test Window".into());
    toplevel.set_app_id("lntrn-test-window".into());
    toplevel.set_min_size(W, H);
    toplevel.set_max_size(W, H);

    // Request server-side decorations so Lantern's SSD draws the titlebar.
    let _deco = state.decoration_mgr.as_ref().map(|m| {
        let d = m.get_toplevel_decoration(&toplevel, &qh, ());
        d.set_mode(DecorationMode::ServerSide);
        d
    });

    surface.commit();
    while !state.configured {
        queue.blocking_dispatch(&mut state)?;
    }

    // Build the buffer once and keep it attached. The window never animates.
    let stride = W * 4;
    let size = (stride * H) as usize;
    let pixels = make_dark_pixels(size);
    let buffer = create_shm_buffer(&shm, &qh, &pixels, W, H, stride)?;
    surface.attach(Some(&buffer), 0, 0);
    surface.damage_buffer(0, 0, W, H);
    surface.commit();

    while state.running {
        queue.blocking_dispatch(&mut state)?;
    }
    Ok(())
}

fn make_dark_pixels(size: usize) -> Vec<u8> {
    let mut v = Vec::with_capacity(size);
    let pixel_count = size / 4;
    for _ in 0..pixel_count {
        // ARGB8888 little-endian on disk: B, G, R, A
        v.extend_from_slice(&[BG[0], BG[1], BG[2], BG[3]]);
    }
    v
}

fn create_shm_buffer(
    shm: &WlShm,
    qh: &QueueHandle<State>,
    pixels: &[u8],
    w: i32,
    h: i32,
    stride: i32,
) -> Result<WlBuffer> {
    // /dev/shm gives us a file that the compositor can mmap. Unlinked
    // immediately — the fd keeps it alive until both ends drop it.
    let path = format!("/dev/shm/lantern-test-{}", std::process::id());
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(&path)?;
    file.write_all(pixels)?;
    file.flush()?;
    file.seek(SeekFrom::Start(0))?;
    let _ = std::fs::remove_file(&path);

    let size = pixels.len() as i32;
    let pool = shm.create_pool(file.as_fd(), size, qh, ());
    let buffer = pool.create_buffer(0, w, h, stride, Format::Argb8888, qh, ());
    pool.destroy();
    Ok(buffer)
}

// ── Dispatch impls (mostly empty — we don't care about most events) ────────

impl Dispatch<WlRegistry, ()> for State {
    fn event(
        state: &mut Self,
        registry: &WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
        {
            match interface.as_str() {
                "wl_compositor" => {
                    state.compositor =
                        Some(registry.bind::<WlCompositor, _, _>(name, version.min(6), qh, ()));
                }
                "wl_shm" => {
                    state.shm = Some(registry.bind::<WlShm, _, _>(name, version.min(1), qh, ()));
                }
                "xdg_wm_base" => {
                    state.wm_base =
                        Some(registry.bind::<XdgWmBase, _, _>(name, version.min(6), qh, ()));
                }
                "zxdg_decoration_manager_v1" => {
                    state.decoration_mgr =
                        Some(registry.bind::<ZxdgDecorationManagerV1, _, _>(name, 1, qh, ()));
                }
                _ => {}
            }
        }
    }
}

impl Dispatch<XdgWmBase, ()> for State {
    fn event(
        _: &mut Self,
        wm_base: &XdgWmBase,
        event: xdg_wm_base::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_wm_base::Event::Ping { serial } = event {
            wm_base.pong(serial);
        }
    }
}

impl Dispatch<XdgSurface, ()> for State {
    fn event(
        state: &mut Self,
        xdg_surface: &XdgSurface,
        event: xdg_surface::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_surface::Event::Configure { serial } = event {
            xdg_surface.ack_configure(serial);
            state.configured = true;
        }
    }
}

impl Dispatch<XdgToplevel, ()> for State {
    fn event(
        state: &mut Self,
        _: &XdgToplevel,
        event: xdg_toplevel::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if matches!(event, xdg_toplevel::Event::Close) {
            state.running = false;
        }
    }
}

impl Dispatch<ZxdgToplevelDecorationV1, ()> for State {
    fn event(
        _: &mut Self,
        _: &ZxdgToplevelDecorationV1,
        _: zxdg_toplevel_decoration_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

// These have no events we care about, but Dispatch must be implemented.
impl Dispatch<WlCompositor, ()> for State {
    fn event(
        _: &mut Self,
        _: &WlCompositor,
        _: wayland_client::protocol::wl_compositor::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}
impl Dispatch<WlShm, ()> for State {
    fn event(
        _: &mut Self,
        _: &WlShm,
        _: wl_shm::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}
impl Dispatch<WlShmPool, ()> for State {
    fn event(
        _: &mut Self,
        _: &WlShmPool,
        _: wayland_client::protocol::wl_shm_pool::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}
impl Dispatch<WlBuffer, ()> for State {
    fn event(
        _: &mut Self,
        _: &WlBuffer,
        _: wayland_client::protocol::wl_buffer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}
impl Dispatch<WlSurface, ()> for State {
    fn event(
        _: &mut Self,
        _: &WlSurface,
        _: wayland_client::protocol::wl_surface::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}
impl Dispatch<ZxdgDecorationManagerV1, ()> for State {
    fn event(
        _: &mut Self,
        _: &ZxdgDecorationManagerV1,
        _: wayland_protocols::xdg::decoration::zv1::client::zxdg_decoration_manager_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

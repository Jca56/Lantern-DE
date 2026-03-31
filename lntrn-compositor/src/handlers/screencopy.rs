//! wlr-screencopy protocol implementation.
//!
//! Allows screenshot tools (grim, OBS, etc.) to capture screen contents
//! into client-provided wl_shm buffers.

use smithay::{
    output::Output,
    reexports::wayland_server::{
        backend::{ClientId, GlobalId},
        protocol::wl_buffer::WlBuffer,
        protocol::wl_output::WlOutput,
        protocol::wl_shm,
        Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
    },
};
use wayland_protocols_wlr::screencopy::v1::server::{
    zwlr_screencopy_frame_v1::{self, ZwlrScreencopyFrameV1},
    zwlr_screencopy_manager_v1::{self, ZwlrScreencopyManagerV1},
};

use crate::Lantern;

/// Per-frame state: tracks a single capture request.
pub struct ScreencopyFrameState {
    pub output: Output,
    pub overlay_cursor: bool,
    pub width: u32,
    pub height: u32,
    pub with_damage: bool,
}

/// Global screencopy manager state.
pub struct ScreencopyManagerState {
    _global: GlobalId,
}

impl ScreencopyManagerState {
    pub fn new(dh: &DisplayHandle) -> Self {
        let global = dh.create_global::<Lantern, ZwlrScreencopyManagerV1, _>(3, ());
        Self { _global: global }
    }
}

// ── Pending capture requests that need to be served at next render ───

/// A capture request waiting to be fulfilled during the next render.
pub struct PendingScreencopy {
    pub frame: ZwlrScreencopyFrameV1,
    pub buffer: WlBuffer,
    pub output: Output,
    pub overlay_cursor: bool,
    pub with_damage: bool,
}

// ── GlobalDispatch: Manager ─────────────────────────────────────────

impl GlobalDispatch<ZwlrScreencopyManagerV1, (), Lantern> for Lantern {
    fn bind(
        _state: &mut Lantern,
        _dh: &DisplayHandle,
        _client: &Client,
        resource: New<ZwlrScreencopyManagerV1>,
        _data: &(),
        data_init: &mut DataInit<'_, Lantern>,
    ) {
        data_init.init(resource, ());
    }
}

// ── Dispatch: Manager requests ──────────────────────────────────────

impl Dispatch<ZwlrScreencopyManagerV1, (), Lantern> for Lantern {
    fn request(
        state: &mut Lantern,
        _client: &Client,
        _manager: &ZwlrScreencopyManagerV1,
        request: zwlr_screencopy_manager_v1::Request,
        _data: &(),
        _dh: &DisplayHandle,
        data_init: &mut DataInit<'_, Lantern>,
    ) {
        match request {
            zwlr_screencopy_manager_v1::Request::CaptureOutput {
                frame,
                overlay_cursor,
                output,
            } => {
                handle_capture(state, data_init, frame, overlay_cursor, output, None);
            }
            zwlr_screencopy_manager_v1::Request::CaptureOutputRegion {
                frame,
                overlay_cursor,
                output,
                ..
            } => {
                // Region capture: for now, capture the full output.
                // TODO: support sub-region capture
                handle_capture(state, data_init, frame, overlay_cursor, output, None);
            }
            zwlr_screencopy_manager_v1::Request::Destroy => {}
            _ => {}
        }
    }
}

fn handle_capture(
    _state: &mut Lantern,
    data_init: &mut DataInit<'_, Lantern>,
    frame: New<ZwlrScreencopyFrameV1>,
    overlay_cursor: i32,
    wl_output: WlOutput,
    _region: Option<(i32, i32, i32, i32)>,
) {
    // Find the smithay Output from the wl_output resource
    let output = Output::from_resource(&wl_output);

    let Some(output) = output else {
        // Can't find the output — init the frame then send failed
        let frame_obj = data_init.init(frame, None);
        frame_obj.failed();
        return;
    };

    // Get physical output size
    let mode = output.current_mode().unwrap();
    let (width, height) = (mode.size.w as u32, mode.size.h as u32);

    let frame_state = ScreencopyFrameState {
        output: output.clone(),
        overlay_cursor: overlay_cursor != 0,
        width,
        height,
        with_damage: false,
    };

    let frame_obj = data_init.init(frame, Some(frame_state));

    // Tell the client which buffer format/size to use (ARGB8888, physical size)
    let stride = width * 4;
    frame_obj.buffer(
        wl_shm::Format::Argb8888,
        width,
        height,
        stride,
    );

    // Version 3: signal that all buffer types have been sent
    if frame_obj.version() >= 3 {
        frame_obj.buffer_done();
    }
}

// ── Dispatch: Frame requests ────────────────────────────────────────

impl Dispatch<ZwlrScreencopyFrameV1, Option<ScreencopyFrameState>, Lantern> for Lantern {
    fn request(
        state: &mut Lantern,
        _client: &Client,
        frame: &ZwlrScreencopyFrameV1,
        request: zwlr_screencopy_frame_v1::Request,
        data: &Option<ScreencopyFrameState>,
        _dh: &DisplayHandle,
        _data_init: &mut DataInit<'_, Lantern>,
    ) {
        match request {
            zwlr_screencopy_frame_v1::Request::Copy { buffer } => {
                submit_copy(state, frame, &buffer, data, false);
            }
            zwlr_screencopy_frame_v1::Request::CopyWithDamage { buffer } => {
                submit_copy(state, frame, &buffer, data, true);
            }
            zwlr_screencopy_frame_v1::Request::Destroy => {}
            _ => {}
        }
    }

    fn destroyed(
        state: &mut Lantern,
        _client: ClientId,
        resource: &ZwlrScreencopyFrameV1,
        _data: &Option<ScreencopyFrameState>,
    ) {
        // Remove any pending capture for this frame
        state
            .pending_screencopy
            .retain(|p| p.frame != *resource);
    }
}

fn submit_copy(
    state: &mut Lantern,
    frame: &ZwlrScreencopyFrameV1,
    buffer: &WlBuffer,
    data: &Option<ScreencopyFrameState>,
    with_damage: bool,
) {
    let Some(frame_state) = data else {
        frame.failed();
        return;
    };

    // Queue this capture to be fulfilled at the next render
    state.pending_screencopy.push(PendingScreencopy {
        frame: frame.clone(),
        buffer: buffer.clone(),
        output: frame_state.output.clone(),
        overlay_cursor: frame_state.overlay_cursor,
        with_damage,
    });

    // Trigger a render so we can capture
    state.schedule_render();
}

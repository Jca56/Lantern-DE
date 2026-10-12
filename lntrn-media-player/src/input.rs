//! Keyboard shortcuts, and the window-edge resize bands with their cursor
//! shapes.
//!
//! Shortcut table:
//! - Space: play / pause                - Left / A, Right / D: seek ∓5 s
//! - Up / Down: volume                  - N / P: next / previous track
//! - L: loop mode                       - F / F11: fullscreen, Esc leaves it
//! - Ctrl+Q: quit

use wayland_protocols::wp::cursor_shape::v1::client::wp_cursor_shape_device_v1::Shape;
use wayland_protocols::xdg::shell::client::xdg_toplevel::{ResizeEdge, XdgToplevel};

use crate::app::App;
use crate::keyboard::KeyPress;
use crate::wayland::State;

// Linux keycodes
const KEY_ESC: u32 = 1;
const KEY_Q: u32 = 16;
const KEY_P: u32 = 25;
const KEY_A: u32 = 30;
const KEY_D: u32 = 32;
const KEY_F: u32 = 33;
const KEY_L: u32 = 38;
const KEY_N: u32 = 49;
const KEY_SPACE: u32 = 57;
const KEY_F11: u32 = 87;
const KEY_UP: u32 = 103;
const KEY_LEFT: u32 = 105;
const KEY_RIGHT: u32 = 106;
const KEY_DOWN: u32 = 108;

const SEEK_STEP_NS: i64 = 5_000_000_000;
const VOLUME_STEP: f64 = 0.05;

/// Act on one key press. A held key repeats only for seeking and volume;
/// everything else (play/pause, track changes, toggles) acts once per press.
pub fn handle_key(app: &mut App, toplevel: &XdgToplevel, state: &mut State, press: KeyPress) {
    let key = press.key;
    let steps = matches!(
        key,
        KEY_LEFT | KEY_RIGHT | KEY_A | KEY_D | KEY_UP | KEY_DOWN
    );
    if press.repeat && !steps {
        return;
    }
    // Chords aren't the plain shortcuts: Ctrl+F must not toggle fullscreen.
    if press.ctrl {
        if key == KEY_Q {
            state.running = false;
        }
        return;
    }
    // Super chords belong to the compositor, which hands us F11 on purpose
    // (plain F11 is its own fullscreen toggle).
    if press.alt || (press.logo && key != KEY_F11) {
        return;
    }
    match key {
        KEY_SPACE => app.toggle_play_pause(),
        KEY_LEFT | KEY_A => app.seek_relative(-SEEK_STEP_NS),
        KEY_RIGHT | KEY_D => app.seek_relative(SEEK_STEP_NS),
        KEY_UP => app.adjust_volume(VOLUME_STEP),
        KEY_DOWN => app.adjust_volume(-VOLUME_STEP),
        KEY_N => {
            app.next_track();
        }
        KEY_P => {
            app.prev_track();
        }
        KEY_L => app.cycle_loop_mode(),
        KEY_F11 | KEY_F => {
            if state.fullscreen {
                toplevel.unset_fullscreen();
            } else {
                toplevel.set_fullscreen(None);
            }
        }
        KEY_ESC => {
            if state.fullscreen {
                toplevel.unset_fullscreen();
            }
        }
        _ => {}
    }
}

/// Resize on any edge / corner within `border` of the window bounds. The
/// controls overlay auto-hides, so when `controls_visible` is true the bottom
/// edge is suppressed to keep the seek bar and buttons clickable; the top edge
/// is likewise suppressed when the title bar is showing (it owns those clicks).
pub fn edge_resize(
    cx: f32,
    cy: f32,
    w: f32,
    h: f32,
    border: f32,
    controls_visible: bool,
    title_visible: bool,
) -> Option<ResizeEdge> {
    let left = cx < border;
    let right = cx > w - border;
    let top = cy < border && !title_visible;
    let bottom = cy > h - border && !controls_visible;
    match (left, right, top, bottom) {
        (true, _, true, _) => Some(ResizeEdge::TopLeft),
        (_, true, true, _) => Some(ResizeEdge::TopRight),
        (true, _, _, true) => Some(ResizeEdge::BottomLeft),
        (_, true, _, true) => Some(ResizeEdge::BottomRight),
        (true, _, _, _) => Some(ResizeEdge::Left),
        (_, true, _, _) => Some(ResizeEdge::Right),
        (_, _, true, _) => Some(ResizeEdge::Top),
        (_, _, _, true) => Some(ResizeEdge::Bottom),
        _ => None,
    }
}

pub fn resize_edge_shape(edge: ResizeEdge) -> Shape {
    match edge {
        ResizeEdge::Top => Shape::NResize,
        ResizeEdge::Bottom => Shape::SResize,
        ResizeEdge::Left => Shape::WResize,
        ResizeEdge::Right => Shape::EResize,
        ResizeEdge::TopLeft => Shape::NwResize,
        ResizeEdge::TopRight => Shape::NeResize,
        ResizeEdge::BottomLeft => Shape::SwResize,
        ResizeEdge::BottomRight => Shape::SeResize,
        _ => Shape::Default,
    }
}

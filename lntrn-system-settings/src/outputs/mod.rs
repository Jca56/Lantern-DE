//! The compositor's monitors: what each is doing now, the modes it can
//! run, and a way to ask for another setup. Spoken over
//! wlr-output-management on a connection of our own, beside the window's:
//! Lantern UI keeps the window's to itself, and this is the one protocol
//! the app speaks that a window has no use for.
//!
//! - `wire`: the Wayland wire format.
//! - `engine`: the protocol, as bytes in and bytes out.
//! - `link`: the socket and the thread that reads it.
//!
//! The page talks in names and numbers ([`Head`], [`Change`]); the ids
//! the compositor knows things by stay in here.

mod engine;
mod link;
mod wire;

pub use link::{Link, Outputs, View};

/// One way a monitor can be driven.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Mode {
    pub width: i32,
    pub height: i32,
    /// Millihertz, as the compositor says it.
    pub refresh: i32,
    /// The one the monitor itself would pick.
    pub preferred: bool,
}

/// A monitor as the compositor has it.
#[derive(Clone, Debug, PartialEq)]
pub struct Head {
    /// What the compositor calls it: `DP-1`.
    pub name: String,
    /// Off, it is plugged in but no part of the desktop, and the
    /// compositor says neither where it is nor how it is scaled.
    pub enabled: bool,
    /// Its top-left corner on the desktop, in logical pixels.
    pub position: (i32, i32),
    pub scale: f64,
    /// The panel's size in millimetres; zero when it doesn't say.
    pub physical_mm: (i32, i32),
    pub modes: Vec<Mode>,
    /// Which of `modes` it is in.
    pub current: Option<usize>,
}

impl Default for Head {
    fn default() -> Self {
        Self { name: String::new(), enabled: false, position: (0, 0), scale: 1.0, physical_mm: (0, 0), modes: Vec::new(), current: None }
    }
}

/// What one monitor should be after an apply.
#[derive(Clone, Debug, PartialEq)]
pub struct Change {
    pub name: String,
    pub enabled: bool,
    /// Which of its modes to switch to; `None` leaves the mode alone, so
    /// the screen doesn't blink for nothing.
    pub mode: Option<usize>,
    pub position: (i32, i32),
    /// `None` leaves the scale as it is.
    pub scale: Option<f64>,
}

/// How an apply ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Succeeded,
    /// The compositor tried and couldn't: a mode the monitor refused.
    Failed,
    /// The monitors changed while we were asking.
    Cancelled,
}

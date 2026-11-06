//! Afterglow's root helper, and everything the window shares with it.
//!
//! Changing a graphics card's clocks, its power limit or its fans, or a
//! processor's governor and power limits, takes root. `lntrn-afterglowd`
//! is the only part of Afterglow that runs as root, and the only part
//! that changes anything: the window reads the hardware itself and asks
//! the helper, over a socket only the `wheel` group can open, to set what
//! the user picked.
//!
//! - `hw` — the hardware: NVIDIA's management library, the processor's
//!   frequency driver, and Intel's package power limits. Reading works as
//!   anyone; writing takes root.
//! - `tune` — what can be set ([`tune::Tune`]), how far each knob may go
//!   on this machine ([`tune::Limits`]), and both as documents.
//! - `wire` — the protocol: one JSON object a line each way.
//! - `client` — the window's end of the socket.
//!
//! The helper itself (`main.rs`, `serve/`) is the binary: the socket,
//! who may use it, the keep-or-revert timer and the fan curve.

pub mod client;
pub mod hw;
pub mod tune;
pub mod wire;

/// Where the helper listens.
pub const SOCKET: &str = "/run/lntrn-afterglowd.sock";

/// The group whose members may ask the helper for anything. The socket is
/// theirs to open and no one else's.
pub const GROUP: &str = "wheel";

/// The protocol's version: the window and the helper say theirs in
/// `hello`, and a window refuses a helper that speaks another.
pub const PROTOCOL: i64 = 1;

/// How long a trial lasts before it is undone, unless kept.
pub const TRIAL_SECONDS: u64 = 15;

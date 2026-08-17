//! Security primitives for compositor — protocol gating and IPC peer auth.
//!
//! The core idea: most privileged Wayland protocols (clipboard read,
//! screen capture, window enumeration, layer-shell overlays) should
//! only be available to *our own* Lantern apps, not arbitrary clients
//! that connect to the compositor socket.
//!
//! A client is "trusted" if its `/proc/<pid>/exe` resolves to a binary
//! either inside `~/.lantern/bin/` or listed in
//! `~/.lantern/config/trusted-clients.toml`.
//!
//! ## Caveats
//!
//! - PID-based identification is not a hard security boundary; the
//!   wayland-server docs explicitly call this out. A truly hostile
//!   process can race a fork+exec to spoof its `/proc/<pid>/exe` link
//!   between when we check it and when the client actually does
//!   anything privileged. The real long-term answer is
//!   `security_context_v1`, planned for a later wave.
//! - This is *defense-in-depth*, raising the bar from "any local
//!   Wayland client steals your clipboard" to "an attacker has to
//!   actually exploit a Lantern app first."

pub mod client_trust;

pub use client_trust::{
    compute_trust_at_connect, is_trusted_client, is_trusted_surface, peer_identity,
};

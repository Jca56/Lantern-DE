//! lntrn-gear — Lantern's peripheral control (our G HUB replacement).
//!
//! Layered so a new device or vendor is additive, never a rewrite:
//!
//! ```text
//!   UI / daemon / CLI         drive capabilities, never raw protocol
//!        │
//!   caps.rs                   Device / Lighting / Dpi traits (the contract)
//!        │
//!   devices/                  map a transport's features onto capabilities
//!     logitech.rs             HID++ driver (mouse / keyboard / headset)
//!        │
//!   transport/                the wire protocols
//!     hidpp.rs                HID++ 2.0 over hidraw
//! ```
//!
//! The UI asks a `Device` for its `Lighting` and sets a color — it never
//! learns whether that's HID++ or anything else underneath. Adding Razer
//! (etc.) means a new `transport` + a `devices` driver behind the same
//! traits, with zero UI changes.

pub mod caps;
pub mod devices;
pub mod error;
pub mod transport;

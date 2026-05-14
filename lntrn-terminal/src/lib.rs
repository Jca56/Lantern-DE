//! Library surface for `lntrn-terminal` — re-exports the modules that
//! other Lantern crates (command-center, etc.) need to embed a real
//! terminal: PTY allocation/IO and the VTE-driven grid state.
//!
//! The full standalone terminal binary still lives in `main.rs`; this
//! lib only exposes the embeddable bits.

pub mod clipboard;
pub mod pty;
pub mod terminal;

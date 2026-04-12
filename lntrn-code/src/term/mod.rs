//! Embedded terminal emulator — core grid, PTY, ANSI parsing, and rendering.
//! Adapted from `lntrn-terminal` for use as a panel inside the code editor.

mod charwidth;
pub mod grid;
mod performer;
pub mod pty;
pub mod input;
pub mod render;

pub use grid::{Cell, Color8, TerminalState, Wide};

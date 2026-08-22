//! Embedded terminal emulator — core grid, PTY, ANSI parsing, and rendering.
//! Adapted from `lntrn-terminal` for use as a panel inside the code editor.

mod charwidth;
pub mod grid;
pub mod input;
mod performer;
pub mod pty;
pub mod render;

pub use grid::{Cell, Color8, TerminalState, Wide};

//! The apps installed here and their pictures, for Lantern apps on
//! Lantern UI 2, so a launcher, a dock and a settings page list the same
//! apps the same way:
//!
//! - `entry`: what each `.desktop` file says (its name, what it runs, its
//!   icon and categories), with the ones a launcher hides left out.
//! - `group`: the categories a launcher sorts apps under.
//! - `icons`: the picture an icon name means, found the way the desktop
//!   finds it, rendered off the UI thread and handed to the GPU.
//! - `launch`: running an app so it outlives what started it.
//!
//! The parsing and the icon search order began as System Settings' radial
//! page's (and `lntrn-desktop/src/system_icons.rs`'s, which other crates
//! keep copies of): this crate is where those copies can move.

pub mod entry;
pub mod group;
pub mod icons;
pub mod launch;

pub use entry::{App, installed};
pub use group::Group;
pub use icons::Icons;

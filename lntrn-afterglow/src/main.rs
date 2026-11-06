//! Lantern Afterglow: tune the graphics card and the processor, and
//! watch what it does, on Lantern UI 2 with `lntrn-kit`'s look.
//!
//! The window runs as the user and reads the hardware itself; every
//! change goes through `lntrn-afterglowd`, the root helper, which keeps
//! the keep-or-revert timer and runs the fan curve. `--page <id>` opens
//! it on a page (`overview`, `graphics`, `fans`, `processor`, `profiles`).
//!
//! ## Layout
//!
//! - `app.rs` — the [`lntrn_ui::Host`]: sidebar, page and bar, the menu,
//!   the palette, and what the actions do.
//! - `edit.rs` — the tune being edited, beside the one applied.
//! - `sample.rs`, `worker.rs` — the hardware's readings and the helper's
//!   word, looked at on a thread of their own.
//! - `nav.rs`, `sidebar.rs`, `glyphs.rs` — the pages and their pictures.
//! - `pages/` — one module per page; `curve.rs` the fan curve's editor.
//! - `bar.rs` — the strip along the bottom: Apply, Keep, Revert.
//! - `profiles.rs`, `dialogs.rs` — saved tunes, and naming them.
//! - `format.rs` — clocks, heat and power as people read them.

mod app;
mod bar;
mod curve;
mod dialogs;
mod edit;
mod format;
mod glyphs;
mod nav;
mod pages;
mod profiles;
mod sample;
mod sidebar;
#[cfg(test)]
mod smoke;
mod worker;

use lntrn_app::{AppConfig, run};
use lntrn_ui::Shell;

use crate::app::{APP_ID, App, Editor};
use crate::nav::Page;
use crate::profiles::Profiles;
use crate::worker::Link;

/// The page `--page <id>` (or `--page=<id>`) names, if it names one.
fn page_asked(args: &[String]) -> Option<Page> {
    let at = args.iter().position(|a| a == "--page" || a.starts_with("--page="))?;
    let id = args[at].strip_prefix("--page=").or_else(|| args.get(at + 1).map(String::as_str))?;
    Page::from_id(id)
}

fn main() {
    lntrn_kit::startup::log_panics(APP_ID);
    let mut app = App::new(Link::spawn(lntrn_afterglowd::SOCKET.into()), Profiles::load());
    if let Some(page) = page_asked(&std::env::args().collect::<Vec<_>>()) {
        app.page = page;
    }
    let desktop = app.desktop().clone();
    let config = AppConfig {
        title: "Afterglow".into(),
        app_id: APP_ID.into(),
        size: (1500.0, 1050.0),
        min_size: (1100.0, 750.0),
        maximized: false,
        sans: desktop.font,
        opacity: desktop.opacity,
        transparent: true,
        ..AppConfig::default()
    };
    run(config, app, Shell::new(Editor::Main));
    std::process::exit(0);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_page_is_asked_for_either_way_and_nonsense_is_passed_over() {
        let args = |list: &[&str]| list.iter().map(|a| (*a).to_owned()).collect::<Vec<_>>();
        assert_eq!(page_asked(&args(&["lntrn-afterglow", "--page", "fans"])), Some(Page::Fans));
        assert_eq!(page_asked(&args(&["lntrn-afterglow", "--page=processor"])), Some(Page::Processor));
        assert_eq!(page_asked(&args(&["lntrn-afterglow", "--page"])), None);
        assert_eq!(page_asked(&args(&["lntrn-afterglow", "--page", "nope"])), None);
    }
}

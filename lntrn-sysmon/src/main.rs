//! Lantern System Monitor: what the machine is doing, on Lantern UI 2
//! with `lntrn-kit`'s look. `--page <id>` opens it on a page (`cpu`,
//! `memory`, `gpu`, ...: `nav.rs`), as the Control Center's mini monitors
//! do.
//!
//! ## Layout
//!
//! - `app.rs` — the [`lntrn_ui::Host`]: the sidebar and the chosen page,
//!   the menu, the palette, what the actions do.
//! - `sample/` — looking at the machine: `/proc`, sysfs and the graphics
//!   driver read into a [`sample::Frame`]. All of it runs on
//! - `worker.rs` — the sampler's thread, which keeps only its newest
//!   look for the window to pick up.
//! - `nav.rs`, `sidebar.rs`, `glyphs.rs` — the pages, the groups the
//!   sidebar lists them under, and their pictures.
//! - `pages/` — one module per page, made of the kit's cards and charts.
//! - `rows.rs`, `kill.rs` — what the Processes page lists, and ending
//!   what is picked there.
//! - `settings.rs` — the `[sysmon]` section of `lantern.toml`.
//! - `format.rs`, `ffi.rs` — numbers as people read them, and the libc
//!   calls std has no face for.

mod app;
mod ffi;
mod format;
mod glyphs;
mod kill;
mod nav;
mod pages;
mod rows;
mod sample;
mod settings;
mod sidebar;
#[cfg(test)]
mod smoke;
mod worker;

use lntrn_app::{AppConfig, run};
use lntrn_ui::Shell;

use crate::app::{APP_ID, App, Editor};
use crate::nav::Page;
use crate::settings::Settings;
use crate::worker::Link;

/// The page `--page <id>` (or `--page=<id>`) names, if it names one.
fn page_asked(args: &[String]) -> Option<Page> {
    let at = args.iter().position(|a| a == "--page" || a.starts_with("--page="))?;
    let id = args[at].strip_prefix("--page=").or_else(|| args.get(at + 1).map(String::as_str))?;
    Page::from_id(id)
}

fn main() {
    lntrn_kit::startup::log_panics(APP_ID);
    let settings = Settings::load();
    let mut app = App::new(Link::spawn(settings.interval), settings);
    if let Some(page) = page_asked(&std::env::args().collect::<Vec<_>>()) {
        app.page = page;
    }
    let desktop = app.desktop().clone();
    let config = AppConfig {
        title: "System Monitor".into(),
        app_id: APP_ID.into(),
        size: (1500.0, 1000.0),
        min_size: (1100.0, 700.0),
        maximized: false,
        sans: desktop.font,
        opacity: desktop.opacity,
        transparent: true,
        ..AppConfig::default()
    };
    run(config, app, Shell::new(Editor::Monitor));
    std::process::exit(0);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_page_is_asked_for_either_way_and_nonsense_is_passed_over() {
        let args = |list: &[&str]| list.iter().map(|a| (*a).to_owned()).collect::<Vec<_>>();
        assert_eq!(page_asked(&args(&["lntrn-sysmon", "--page", "gpu"])), Some(Page::Gpu));
        assert_eq!(page_asked(&args(&["lntrn-sysmon", "--page=memory"])), Some(Page::Memory));
        assert_eq!(page_asked(&args(&["lntrn-sysmon", "--page"])), None);
        assert_eq!(page_asked(&args(&["lntrn-sysmon", "--page", "nope"])), None);
        assert_eq!(page_asked(&args(&["lntrn-sysmon"])), None);
    }
}

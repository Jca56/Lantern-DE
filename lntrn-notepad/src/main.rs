//! Lantern Notepad: documents with dressed text, lists and pictures, on
//! Lantern UI 2 with `lntrn-kit`'s look.
//!
//! ## Layout
//!
//! - `doc/` — the document: paragraphs, runs, styles, lists, pictures.
//! - `editor.rs`, `editor_ops.rs` — a document being edited: the caret,
//!   the selection, typing, formatting, cut and paste, undo.
//! - `view/` — the document on screen: set into rows (`layout`), told as
//!   marks (`paint`), drawn on its page (`page`), typed and clicked at
//!   (`input`), with the toolbar and the find bar.
//! - `io/` — files: Notepad's own `.lnote`, text, Markdown, Word.
//! - `export.rs`, `paper.rs` — pages, for a PDF, and the sheet they are.
//! - `app.rs` — the [`lntrn_ui::Host`]: tabs of documents.
//! - `actions.rs`, `menu.rs` — what the menus and keys do, and the menus.
//! - `ops.rs` — after each frame: tabs, drafts, what is open remembered.
//! - `session.rs`, `settings.rs` — what is kept between runs.

mod actions;
mod app;
mod doc;
mod editor;
mod editor_ops;
mod export;
mod io;
mod menu;
mod ops;
mod paper;
mod session;
mod settings;
#[cfg(test)]
mod smoke;
mod view;

use std::path::PathBuf;

use lntrn_app::{AppConfig, run};

/// A test must never reach the real `~/.lantern`: everything that works
/// out a path there checks, while tested, that the folders have been
/// pointed at a scratch tree first.
#[cfg(test)]
pub(crate) fn sandboxed() {
    let home = std::env::var_os("LANTERN_HOME").map(PathBuf::from);
    assert!(home.is_some_and(|h| h.starts_with(std::env::temp_dir())), "a test reached for the real ~/.lantern: it has to set up its sandbox first");
}
use lntrn_ui::Shell;

use crate::app::{APP_ID, App, Kind};

fn main() {
    lntrn_kit::startup::log_panics(APP_ID);
    // Every argument is a file to open, wherever Notepad was started.
    let here = std::env::current_dir().unwrap_or_default();
    let files: Vec<PathBuf> = std::env::args_os().skip(1).map(|arg| here.join(arg)).collect();
    let mut app = App::new();
    let mut shell = Shell::new(Kind::Document);
    app.restore(&mut shell, files);
    let desktop = app.follow.desktop().clone();
    let config = AppConfig {
        title: "Notepad".into(),
        app_id: APP_ID.into(),
        size: (1100.0, 1200.0),
        min_size: (640.0, 480.0),
        maximized: false,
        sans: desktop.font,
        opacity: desktop.opacity,
        transparent: true,
        // What is open is remembered here, with the drafts.
        persist: false,
        ..AppConfig::default()
    };
    run(config, app, shell);
    std::process::exit(0);
}

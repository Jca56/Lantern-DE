//! What a headless test can ask the kit about a layout it can't look at:
//! which labels were cut short, and which rows had to put their control
//! under their text. Nothing is recorded until [`watch`] is called, so an
//! app that never asks pays for one flag check a row.

use std::cell::{Cell, RefCell};

thread_local! {
    static WATCHING: Cell<bool> = const { Cell::new(false) };
    static CLIPPED: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    static STACKED: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

/// Start recording on this thread.
pub fn watch() {
    WATCHING.with(|w| w.set(true));
}

pub(crate) fn clipped(label: &str) {
    if WATCHING.with(Cell::get) {
        CLIPPED.with(|c| c.borrow_mut().push(label.to_owned()));
    }
}

pub(crate) fn stacked(label: &str) {
    if WATCHING.with(Cell::get) {
        STACKED.with(|c| c.borrow_mut().push(label.to_owned()));
    }
}

/// The labels laid out wider than the room their row had for them since
/// the last call.
pub fn take_clipped() -> Vec<String> {
    CLIPPED.with(|c| std::mem::take(&mut *c.borrow_mut()))
}

/// The labels whose row had no room for the control beside them, so it
/// went underneath, since the last call.
pub fn take_stacked() -> Vec<String> {
    STACKED.with(|c| std::mem::take(&mut *c.borrow_mut()))
}

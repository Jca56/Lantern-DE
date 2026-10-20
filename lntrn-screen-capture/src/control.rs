//! What the parts of a recording say to each other across threads: stop
//! (the badge's button, a second Super+Print, Ctrl+C or SIGTERM), and the
//! UI loop to wake when there is something new to show.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use lntrn_app::Waker;

static STOP: AtomicBool = AtomicBool::new(false);
/// The loop running now: the picker's, then the badge's.
static WAKER: Mutex<Option<Waker>> = Mutex::new(None);

/// Ask whatever runs to stop: the picker cancels, a recording finishes
/// its file.
pub fn stop() {
    STOP.store(true, Ordering::SeqCst);
    wake();
}

pub fn stopped() -> bool {
    STOP.load(Ordering::SeqCst)
}

pub fn set_waker(waker: Waker) {
    *WAKER.lock().unwrap_or_else(|e| e.into_inner()) = Some(waker);
}

/// Wake the UI loop, if one runs: every window draws what changed.
pub fn wake() {
    if let Some(waker) = WAKER.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
        waker.wake();
    }
}

/// Ctrl+C and SIGTERM stop too. A handler may only touch the flag; the UI
/// notices it on its next look (both look at least twice a second).
pub fn catch_signals() {
    extern "C" fn on_signal(_: libc::c_int) {
        STOP.store(true, Ordering::SeqCst);
    }
    // SAFETY: a zeroed sigaction with only the handler set is valid, and
    // the handler does nothing but an atomic store.
    unsafe {
        let mut sa: libc::sigaction = std::mem::zeroed();
        sa.sa_sigaction = on_signal as extern "C" fn(libc::c_int) as usize;
        libc::sigaction(libc::SIGINT, &sa, std::ptr::null_mut());
        libc::sigaction(libc::SIGTERM, &sa, std::ptr::null_mut());
    }
}

//! Mouse-reporting support (xterm protocol).
//!
//! TUIs opt into pointer events with DECSET 1000/1002/1003 and pick the SGR
//! encoding with 1006 — Claude Code, htop, and `vim` with `mouse=a` set all
//! four. We forward scroll-wheel ticks (buttons 64 = up, 65 = down) so those
//! apps drive their own viewports; without this the wheel falls through to
//! terminal scrollback, which is empty on the alt screen — a dead wheel.
//!
//! Button/motion reporting is deliberately NOT forwarded yet: clicks keep
//! doing native text selection everywhere. If we ever forward clicks, the
//! standard escape hatch is Shift+click for native selection.

/// Which pointer events the app asked for (DECSET 1000/1002/1003).
/// The wheel reports identically in all three modes — the distinction only
/// matters for click/motion forwarding, kept for when we add it.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum MouseMode {
    Off,
    /// 1000 — button presses/releases
    Buttons,
    /// 1002 — buttons + drag motion
    Drag,
    /// 1003 — buttons + all motion
    Any,
}

/// One wheel tick as a mouse report. `col`/`row` are 1-based cell coords.
/// SGR (mode 1006): `CSI < 64|65 ; col ; row M`. The legacy X10 framing
/// encodes each coordinate as a single byte (32 + coord), capping at 223.
pub fn wheel_report(sgr: bool, up: bool, col: usize, row: usize) -> Vec<u8> {
    let btn: usize = if up { 64 } else { 65 };
    if sgr {
        format!("\x1b[<{};{};{}M", btn, col, row).into_bytes()
    } else {
        vec![
            0x1b,
            b'[',
            b'M',
            (32 + btn) as u8,
            (32 + col.min(223)) as u8,
            (32 + row.min(223)) as u8,
        ]
    }
}

/// Alternate scroll: wheel ticks for alt-screen apps that did NOT request
/// mouse reporting (`less`, `man`, …) become arrow keys, 3 lines per tick —
/// same feel as alacritty/kitty. `app_cursor` picks SS3 vs CSI arrows
/// (DECCKM), which pagers in application-cursor mode expect.
pub fn alternate_scroll(up: bool, app_cursor: bool, ticks: usize) -> Vec<u8> {
    let seq: &[u8] = match (up, app_cursor) {
        (true, true) => b"\x1bOA",
        (true, false) => b"\x1b[A",
        (false, true) => b"\x1bOB",
        (false, false) => b"\x1b[B",
    };
    seq.repeat(3 * ticks)
}

//! Parse one keystroke (or paste) at a time from stdin.

use crate::term;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Key {
    Char(char),
    Enter,
    Tab,
    Esc,
    Backspace,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    Delete,
    CtrlC,
    CtrlD,
    /// A single block of pasted text (bracketed paste). The string already
    /// has its `\x1b[200~` / `\x1b[201~` wrappers stripped.
    Paste(String),
    Unknown,
}

/// Block on stdin until a key (or paste block) is decoded.
pub fn read_key() -> std::io::Result<Key> {
    let b = term::read_byte()?;
    Ok(match b {
        0x03 => Key::CtrlC,
        0x04 => Key::CtrlD,
        b'\r' | b'\n' => Key::Enter,
        b'\t' => Key::Tab,
        0x7f | 0x08 => Key::Backspace,
        0x1b => read_escape()?,
        c if c < 0x20 => Key::Unknown,
        c => {
            if c < 0x80 {
                Key::Char(c as char)
            } else {
                let mut buf = vec![c];
                let extra = if c & 0xE0 == 0xC0 { 1 }
                    else if c & 0xF0 == 0xE0 { 2 }
                    else if c & 0xF8 == 0xF0 { 3 }
                    else { 0 };
                for _ in 0..extra {
                    buf.push(term::read_byte()?);
                }
                std::str::from_utf8(&buf)
                    .ok()
                    .and_then(|s| s.chars().next())
                    .map(Key::Char)
                    .unwrap_or(Key::Unknown)
            }
        }
    })
}

fn read_escape() -> std::io::Result<Key> {
    let b1 = match read_byte_nonblock() {
        Some(b) => b,
        None => return Ok(Key::Esc), // bare ESC
    };
    if b1 != b'[' && b1 != b'O' {
        return Ok(Key::Esc);
    }
    let b2 = term::read_byte()?;
    Ok(match b2 {
        b'A' => Key::Up,
        b'B' => Key::Down,
        b'C' => Key::Right,
        b'D' => Key::Left,
        b'H' => Key::Home,
        b'F' => Key::End,
        b'3' => { let _ = term::read_byte()?; Key::Delete }
        b'5' => { let _ = term::read_byte()?; Key::PageUp }
        b'6' => { let _ = term::read_byte()?; Key::PageDown }
        b'2' => parse_paste_or_other()?,
        _ => Key::Unknown,
    })
}

/// Handles `\x1b[200~…\x1b[201~` (bracketed paste). The `\x1b[2` prefix has
/// already been consumed by the caller — we read from `0` onward.
fn parse_paste_or_other() -> std::io::Result<Key> {
    // We've consumed: ESC [ 2 . The next bytes should be `00~` for paste-start.
    let b3 = term::read_byte()?;
    let b4 = term::read_byte()?;
    let b5 = term::read_byte()?;
    if (b3, b4, b5) != (b'0', b'0', b'~') {
        // Some other ESC[2… sequence we don't recognize.
        return Ok(Key::Unknown);
    }
    // Read until `\x1b[201~` terminator.
    let mut body = Vec::new();
    loop {
        let b = term::read_byte()?;
        if b == 0x1b {
            // Look for `[201~`
            if term::read_byte()? == b'['
                && term::read_byte()? == b'2'
                && term::read_byte()? == b'0'
                && term::read_byte()? == b'1'
                && term::read_byte()? == b'~'
            {
                break;
            }
            // Not a paste terminator — give up cleanly.
            return Ok(Key::Paste(String::from_utf8_lossy(&body).into_owned()));
        }
        body.push(b);
    }
    let s = String::from_utf8_lossy(&body).into_owned();
    Ok(Key::Paste(s))
}

/// Non-blocking single-byte read. Returns None if nothing is available within
/// a few milliseconds — used to distinguish bare ESC from a CSI sequence.
fn read_byte_nonblock() -> Option<u8> {
    use std::os::fd::AsRawFd;
    let fd = std::io::stdin().as_raw_fd();
    let mut pfd = libc::pollfd { fd, events: libc::POLLIN, revents: 0 };
    let r = unsafe { libc::poll(&mut pfd, 1, 5) };
    if r <= 0 { return None; }
    term::read_byte().ok()
}

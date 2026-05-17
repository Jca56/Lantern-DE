//! Non-blocking key reader. Uses poll(2) with a millisecond timeout so the
//! clock tick can drive redraws while still responding instantly to keys.
//!
//! Goes through libc::read on fd 0 (not std::io::stdin) for the same reason
//! lntrn-keys does — stdlib's BufReader hides bytes from poll().

use std::io;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Char(char),
    Up,
    Down,
    Left,
    Right,
    Enter,
    Esc,
    Tab,
    Backspace,
    Tick,
}

/// Wait up to `timeout_ms` for a key. Returns `Key::Tick` if the timeout
/// elapses without input — caller treats that as "redraw the clock."
pub fn read_or_tick(timeout_ms: i32) -> io::Result<Key> {
    let mut pfd = libc::pollfd {
        fd: 0,
        events: libc::POLLIN,
        revents: 0,
    };
    let r = unsafe { libc::poll(&mut pfd, 1, timeout_ms) };
    if r < 0 {
        let err = io::Error::last_os_error();
        if err.kind() == io::ErrorKind::Interrupted {
            return Ok(Key::Tick);
        }
        return Err(err);
    }
    if r == 0 {
        return Ok(Key::Tick);
    }
    let b = read_byte()?;
    decode(b)
}

fn read_byte() -> io::Result<u8> {
    let mut b = [0u8; 1];
    loop {
        let n = unsafe { libc::read(0, b.as_mut_ptr() as *mut _, 1) };
        if n == 1 {
            return Ok(b[0]);
        }
        if n == 0 {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "stdin closed"));
        }
        let err = io::Error::last_os_error();
        if err.kind() == io::ErrorKind::Interrupted {
            continue;
        }
        return Err(err);
    }
}

/// Peek with a 1ms poll — used after seeing ESC to disambiguate a real
/// Esc keypress from the start of a CSI sequence like ESC [ A.
fn peek_byte() -> Option<u8> {
    let mut pfd = libc::pollfd { fd: 0, events: libc::POLLIN, revents: 0 };
    let r = unsafe { libc::poll(&mut pfd, 1, 1) };
    if r <= 0 {
        return None;
    }
    read_byte().ok()
}

fn decode(b: u8) -> io::Result<Key> {
    Ok(match b {
        0x1b => match peek_byte() {
            Some(b'[') => match peek_byte() {
                Some(b'A') => Key::Up,
                Some(b'B') => Key::Down,
                Some(b'C') => Key::Right,
                Some(b'D') => Key::Left,
                _ => Key::Esc,
            },
            _ => Key::Esc,
        },
        b'\r' | b'\n' => Key::Enter,
        b'\t' => Key::Tab,
        0x7f | 0x08 => Key::Backspace,
        c if c.is_ascii_graphic() || c == b' ' => Key::Char(c as char),
        _ => Key::Char('?'),
    })
}

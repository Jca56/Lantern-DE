//! Raw-mode terminal handling — no curses dep.
//!
//! - Toggles termios canonical/echo on stdin
//! - Switches to the alternate screen buffer
//! - Hides the cursor while we own the screen
//! - Restores everything in `Drop`

use std::io::{self, Write};
use std::mem::MaybeUninit;
use std::os::fd::AsRawFd;

/// RAII handle that holds the terminal in raw + altscreen mode.
pub struct Term {
    original: libc::termios,
    fd: i32,
    stdout: io::Stdout,
}

impl Term {
    pub fn new() -> io::Result<Self> {
        let stdin = io::stdin();
        let fd = stdin.as_raw_fd();
        let mut original: libc::termios = unsafe { MaybeUninit::zeroed().assume_init() };
        if unsafe { libc::tcgetattr(fd, &mut original) } != 0 {
            return Err(io::Error::last_os_error());
        }
        let mut raw = original;
        raw.c_lflag &= !(libc::ICANON | libc::ECHO | libc::IEXTEN | libc::ISIG);
        raw.c_iflag &= !(libc::IXON | libc::ICRNL | libc::BRKINT | libc::INPCK | libc::ISTRIP);
        raw.c_cflag |= libc::CS8;
        raw.c_cc[libc::VMIN] = 1;
        raw.c_cc[libc::VTIME] = 0;
        if unsafe { libc::tcsetattr(fd, libc::TCSANOW, &raw) } != 0 {
            return Err(io::Error::last_os_error());
        }

        let mut t = Self { original, fd, stdout: io::stdout() };
        t.write_all(b"\x1b[?1049h"); // alt screen
        t.write_all(b"\x1b[?25l");   // hide cursor
        t.write_all(b"\x1b[?2004h"); // bracketed paste mode
        t.flush();
        Ok(t)
    }

    pub fn write_all(&mut self, bytes: &[u8]) {
        let _ = self.stdout.write_all(bytes);
    }

    pub fn flush(&mut self) {
        let _ = self.stdout.flush();
    }

    /// Terminal size in (cols, rows). Falls back to 80x24 if the ioctl fails.
    pub fn size(&self) -> (u16, u16) {
        let mut ws: libc::winsize = unsafe { MaybeUninit::zeroed().assume_init() };
        let r = unsafe { libc::ioctl(self.fd, libc::TIOCGWINSZ, &mut ws) };
        if r != 0 || ws.ws_col == 0 || ws.ws_row == 0 {
            (80, 24)
        } else {
            (ws.ws_col, ws.ws_row)
        }
    }
}

impl Drop for Term {
    fn drop(&mut self) {
        self.write_all(b"\x1b[?2004l"); // disable bracketed paste
        self.write_all(b"\x1b[?25h");   // show cursor
        self.write_all(b"\x1b[?1049l"); // exit alt screen
        self.flush();
        unsafe { libc::tcsetattr(self.fd, libc::TCSANOW, &self.original) };
    }
}

// ── ANSI escape helpers ─────────────────────────────────────────────────────

pub fn move_to(buf: &mut Vec<u8>, col: u16, row: u16) {
    use std::fmt::Write as _;
    let mut s = String::new();
    let _ = write!(s, "\x1b[{};{}H", row + 1, col + 1);
    buf.extend_from_slice(s.as_bytes());
}

pub fn clear_all(buf: &mut Vec<u8>) {
    buf.extend_from_slice(b"\x1b[2J");
}

pub fn clear_eol(buf: &mut Vec<u8>) {
    buf.extend_from_slice(b"\x1b[K");
}

pub fn reset(buf: &mut Vec<u8>) {
    buf.extend_from_slice(b"\x1b[0m");
}

pub fn fg(buf: &mut Vec<u8>, code: u8) {
    use std::fmt::Write as _;
    let mut s = String::new();
    let _ = write!(s, "\x1b[38;5;{}m", code);
    buf.extend_from_slice(s.as_bytes());
}

pub fn bg(buf: &mut Vec<u8>, code: u8) {
    use std::fmt::Write as _;
    let mut s = String::new();
    let _ = write!(s, "\x1b[48;5;{}m", code);
    buf.extend_from_slice(s.as_bytes());
}

pub fn bold(buf: &mut Vec<u8>) { buf.extend_from_slice(b"\x1b[1m"); }

/// Read one byte from stdin (blocking).
///
/// Uses `libc::read` directly on fd 0 instead of `std::io::stdin()` — the
/// stdlib wraps stdin in a `BufReader`, which slurps multi-byte sequences
/// (like a bracketed-paste `\x1b[200~…\x1b[201~`) into its internal buffer
/// in one syscall. After that, `poll()` on fd 0 reports "no data" even
/// though bytes are queued in user-space, so we misread escape sequences
/// as bare Esc. Going straight to the fd keeps `poll()` honest.
pub fn read_byte() -> io::Result<u8> {
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

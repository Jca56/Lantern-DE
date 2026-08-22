//! Raw-mode terminal handle — same pattern as lntrn-keys/src/term.rs.
//!
//! Enters alt-screen + hides cursor + sets raw mode in `new()`, restores
//! everything on Drop so panics still leave a usable terminal.

use std::io::{self, Write};
use std::mem::MaybeUninit;
use std::os::fd::AsRawFd;

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

        let mut t = Self {
            original,
            fd,
            stdout: io::stdout(),
        };
        t.write_all(b"\x1b[?1049h"); // alt screen
        t.write_all(b"\x1b[?25l"); // hide cursor
        t.flush();
        Ok(t)
    }

    pub fn write_all(&mut self, bytes: &[u8]) {
        let _ = self.stdout.write_all(bytes);
    }

    pub fn flush(&mut self) {
        let _ = self.stdout.flush();
    }

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
        self.write_all(b"\x1b[0m");
        self.write_all(b"\x1b[?25h");
        self.write_all(b"\x1b[?1049l");
        self.flush();
        unsafe { libc::tcsetattr(self.fd, libc::TCSANOW, &self.original) };
    }
}

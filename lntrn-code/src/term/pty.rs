//! A pseudo-terminal with a shell in it. The master side is read by one
//! thread and written by another, so the UI never blocks on the child;
//! output arrives over a bounded channel, which stops a chatty child when
//! nobody is looking (a hidden terminal) instead of filling memory.

use std::ffi::{CStr, c_char, c_int, c_ulong};
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Child, Command};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::thread;

use lntrn_app::Waker;

unsafe extern "C" {
    fn posix_openpt(flags: c_int) -> c_int;
    fn grantpt(fd: c_int) -> c_int;
    fn unlockpt(fd: c_int) -> c_int;
    fn ptsname_r(fd: c_int, buf: *mut c_char, len: usize) -> c_int;
    fn ioctl(fd: c_int, request: c_ulong, ...) -> c_int;
    fn setsid() -> c_int;
}

const O_RDWR: c_int = 2;
const O_NOCTTY: c_int = 0o400;
const TIOCSCTTY: c_ulong = 0x540E;
const TIOCSWINSZ: c_ulong = 0x5414;

#[repr(C)]
struct Winsize {
    ws_row: u16,
    ws_col: u16,
    ws_xpixel: u16,
    ws_ypixel: u16,
}

/// Output chunks queued for the UI before the reader blocks.
const QUEUE_CHUNKS: usize = 256;

pub struct Pty {
    master: File,
    child: Child,
    rx: Receiver<Vec<u8>>,
    tx_in: SyncSender<Vec<u8>>,
    /// The reader saw the end of the output: the shell is gone.
    eof: Arc<AtomicBool>,
    exited: Option<i32>,
}

fn check(r: c_int, what: &str) -> io::Result<()> {
    if r < 0 { Err(io::Error::other(format!("{what}: {}", io::Error::last_os_error()))) } else { Ok(()) }
}

fn set_size(fd: c_int, cols: u16, rows: u16) {
    let ws = Winsize { ws_row: rows.max(1), ws_col: cols.max(1), ws_xpixel: 0, ws_ypixel: 0 };
    // SAFETY: TIOCSWINSZ reads a winsize struct; the fd is ours.
    unsafe {
        ioctl(fd, TIOCSWINSZ, &ws as *const Winsize);
    }
}

impl Pty {
    /// Open a pty and start the user's shell in it, `cols`×`rows`, in
    /// `cwd`. Output wakes the loop through `waker` when there is one.
    pub fn spawn(cwd: Option<&Path>, cols: u16, rows: u16, waker: Option<Waker>, env: &[(String, String)]) -> io::Result<Self> {
        // SAFETY: plain libc calls on a fresh descriptor; ptsname_r writes
        // at most `len` bytes into our buffer.
        let (master_fd, slave_path) = unsafe {
            let fd = posix_openpt(O_RDWR | O_NOCTTY);
            check(fd, "posix_openpt")?;
            check(grantpt(fd), "grantpt")?;
            check(unlockpt(fd), "unlockpt")?;
            let mut buf = [0 as c_char; 256];
            check(ptsname_r(fd, buf.as_mut_ptr(), buf.len()), "ptsname_r")?;
            let path = CStr::from_ptr(buf.as_ptr()).to_string_lossy().into_owned();
            (fd, path)
        };
        // SAFETY: we own the descriptor from here on.
        let master = unsafe { File::from_raw_fd(master_fd) };
        set_size(master_fd, cols, rows);
        // O_NOCTTY matters: an app launched detached (the desktop's
        // launcher runs it in a session of its own with no terminal) is a
        // session leader, and a session leader that opens a tty without
        // it adopts the tty as its controlling terminal. The shell's
        // setsid + TIOCSCTTY then fails, the parent hangs up on itself
        // when the fds close, and the whole app dies of SIGHUP.
        let slave = OpenOptions::new().read(true).write(true).custom_flags(O_NOCTTY).open(&slave_path)?;
        let shell = std::env::var("SHELL").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| "/bin/sh".to_owned());
        let mut cmd = Command::new(&shell);
        cmd.env("TERM", "xterm-256color").env("COLORTERM", "truecolor").env("TERM_PROGRAM", "lntrn-code");
        cmd.env_remove("LINES").env_remove("COLUMNS");
        for (k, v) in env {
            cmd.env(k, v);
        }
        if let Some(d) = cwd.filter(|d| d.is_dir()) {
            cmd.current_dir(d);
        }
        cmd.stdin(slave.try_clone()?).stdout(slave.try_clone()?).stderr(slave);
        // SAFETY: only async-signal-safe calls between fork and exec.
        unsafe {
            cmd.pre_exec(|| {
                if setsid() < 0 {
                    return Err(io::Error::last_os_error());
                }
                if ioctl(0, TIOCSCTTY, 0) < 0 {
                    return Err(io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let child = cmd.spawn()?;
        let mut reader = master.try_clone()?;
        let (tx, rx) = sync_channel::<Vec<u8>>(QUEUE_CHUNKS);
        let eof = Arc::new(AtomicBool::new(false));
        let eof_flag = Arc::clone(&eof);
        thread::Builder::new().name("pty-read".into()).spawn(move || {
            let mut buf = vec![0u8; 16 * 1024];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        if tx.send(buf[..n].to_vec()).is_err() {
                            break;
                        }
                        if let Some(w) = &waker {
                            w.wake();
                        }
                    }
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                    Err(_) => break,
                }
            }
            eof_flag.store(true, Ordering::Release);
            if let Some(w) = &waker {
                w.wake();
            }
        })?;
        let mut writer = master.try_clone()?;
        let (tx_in, rx_in) = sync_channel::<Vec<u8>>(QUEUE_CHUNKS);
        thread::Builder::new().name("pty-write".into()).spawn(move || {
            while let Ok(bytes) = rx_in.recv() {
                if writer.write_all(&bytes).is_err() {
                    break;
                }
            }
        })?;
        Ok(Self { master, child, rx, tx_in, eof, exited: None })
    }

    /// Move everything the child wrote since the last call into `out`.
    pub fn drain(&mut self, out: &mut Vec<u8>) -> bool {
        let mut any = false;
        while let Ok(chunk) = self.rx.try_recv() {
            out.extend_from_slice(&chunk);
            any = true;
        }
        any
    }

    /// The shell's process id.
    pub fn pid(&self) -> u32 {
        self.child.id()
    }

    /// Send keystrokes (or a paste) to the child.
    pub fn write(&self, bytes: &[u8]) {
        if !bytes.is_empty() {
            let _ = self.tx_in.try_send(bytes.to_vec());
        }
    }

    pub fn resize(&self, cols: u16, rows: u16) {
        set_size(self.master.as_raw_fd(), cols, rows);
    }

    /// The exit code once the shell is gone. After the output ended the
    /// wait is a real one, so the code is known the moment it shows.
    pub fn poll_exit(&mut self) -> Option<i32> {
        if self.exited.is_none() {
            let status = if self.eof.load(Ordering::Acquire) { self.child.wait().ok() } else { self.child.try_wait().ok().flatten() };
            if let Some(status) = status {
                self.exited = Some(status.code().unwrap_or(-1));
            }
        }
        self.exited
    }

    pub fn kill(&mut self) {
        if self.exited.is_none() {
            let _ = self.child.kill();
            let _ = self.child.wait();
            self.exited = Some(-1);
        }
    }
}

impl Drop for Pty {
    fn drop(&mut self) {
        self.kill();
    }
}

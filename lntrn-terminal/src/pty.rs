use std::io::Read;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::sync::mpsc::{self, Receiver, TryRecvError};

use nix::pty::openpty;
use nix::sys::termios::{self, SetArg};
use nix::unistd::{close, dup, dup2, execvp, fork, setsid, ForkResult};

// ── PTY handle ──────────────────────────────────────────────────────────────

pub struct Pty {
    master: OwnedFd,
    child_pid: nix::unistd::Pid,
    pub alive: bool,
    rx: Receiver<Vec<u8>>,
    buffered: Vec<u8>,
}

impl Pty {
    /// Spawn a new PTY running the given shell command.
    /// `repaint` is called from the reader thread when new output arrives.
    pub fn spawn(
        shell: &str,
        cwd: Option<&str>,
        repaint: Box<dyn Fn() + Send + 'static>,
    ) -> Result<Self, String> {
        let pty_pair = openpty(None, None).map_err(|e| format!("openpty: {e}"))?;

        if let Ok(mut termios) = termios::tcgetattr(&pty_pair.slave) {
            termios::cfsetspeed(&mut termios, termios::BaudRate::B38400)
                .map_err(|e| format!("cfsetspeed: {e}"))?;
            termios.local_flags |= termios::LocalFlags::ECHO
                | termios::LocalFlags::ICANON
                | termios::LocalFlags::ISIG
                | termios::LocalFlags::IEXTEN;
            termios.input_flags |= termios::InputFlags::ICRNL;
            termios.output_flags |= termios::OutputFlags::OPOST | termios::OutputFlags::ONLCR;
            termios::tcsetattr(&pty_pair.slave, SetArg::TCSANOW, &termios)
                .map_err(|e| format!("tcsetattr: {e}"))?;
        }

        match unsafe { fork() }.map_err(|e| format!("fork: {e}"))? {
            ForkResult::Child => {
                setsid().ok();

                let slave_fd = pty_pair.slave.as_raw_fd();
                let master_fd = pty_pair.master.as_raw_fd();

                close(master_fd).ok();

                dup2(slave_fd, 0).ok();
                dup2(slave_fd, 1).ok();
                dup2(slave_fd, 2).ok();

                if slave_fd > 2 {
                    close(slave_fd).ok();
                }

                // Set controlling terminal — required for sudo, ssh, etc.
                unsafe { libc::ioctl(0, libc::TIOCSCTTY, 0) };

                if let Some(dir) = cwd {
                    std::env::set_current_dir(dir).ok();
                }

                std::env::set_var("TERM", "xterm-256color");
                std::env::set_var("TERM_PROGRAM", "Lantern");
                std::env::set_var("TERM_PROGRAM_VERSION", "0.1.0");

                // Clean up inherited env vars that confuse child tools
                // (e.g. Claude Code thinks it's already in a session)
                std::env::remove_var("CLAUDECODE");
                std::env::remove_var("CLAUDE_CODE_ENTRYPOINT");

                let shell_cstr = std::ffi::CString::new(shell).expect("Invalid shell path");
                execvp(&shell_cstr, &[&shell_cstr]).ok();

                std::process::exit(1);
            }
            ForkResult::Parent { child } => {
                drop(pty_pair.slave);

                let reader_fd =
                    dup(pty_pair.master.as_raw_fd()).map_err(|e| format!("dup: {e}"))?;
                let (tx, rx) = mpsc::channel();
                std::thread::spawn(move || {
                    let mut read_buf = vec![0u8; 4096];

                    loop {
                        let mut poll_fd = libc::pollfd {
                            fd: reader_fd.as_raw_fd(),
                            events: libc::POLLIN | libc::POLLHUP | libc::POLLERR,
                            revents: 0,
                        };

                        let poll_result = unsafe { libc::poll(&mut poll_fd, 1, -1) };
                        if poll_result <= 0 {
                            break;
                        }

                        if poll_fd.revents & (libc::POLLHUP | libc::POLLERR) != 0 {
                            break;
                        }

                        let mut file = unsafe { std::fs::File::from_raw_fd(reader_fd.as_raw_fd()) };
                        let result = file.read(&mut read_buf);
                        std::mem::forget(file);

                        match result {
                            Ok(0) => break,
                            Ok(n) => {
                                if tx.send(read_buf[..n].to_vec()).is_err() {
                                    break;
                                }
                                repaint();
                            }
                            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => continue,
                            Err(_) => break,
                        }
                    }

                    close(reader_fd).ok();
                });

                let master_fd = pty_pair.master.as_raw_fd();
                let flags = nix::fcntl::fcntl(master_fd, nix::fcntl::FcntlArg::F_GETFL)
                    .map_err(|e| format!("fcntl F_GETFL: {e}"))?;

                let mut oflags = nix::fcntl::OFlag::from_bits_truncate(flags);
                oflags.insert(nix::fcntl::OFlag::O_NONBLOCK);
                nix::fcntl::fcntl(master_fd, nix::fcntl::FcntlArg::F_SETFL(oflags))
                    .map_err(|e| format!("fcntl F_SETFL: {e}"))?;

                Ok(Self {
                    master: pty_pair.master,
                    child_pid: child,
                    alive: true,
                    rx,
                    buffered: Vec::new(),
                })
            }
        }
    }

    pub fn read(&mut self, max_bytes: usize) -> Option<(Vec<u8>, bool)> {
        take_buffered_chunk(&mut self.buffered, &self.rx, max_bytes, &mut self.alive)
    }

    pub fn write(&self, data: &[u8]) {
        use std::io::Write;
        let fd = self.master.as_raw_fd();
        let mut file = unsafe { std::fs::File::from_raw_fd(fd) };
        file.write_all(data).ok();
        std::mem::forget(file);
    }

    pub fn cleanup(&mut self) {
        if self.alive {
            self.alive = false;
            self.graceful_kill();
        }
    }

    /// Send SIGHUP first so the shell can flush history, then SIGKILL as fallback.
    fn graceful_kill(&self) {
        let pid = self.child_pid;
        let _ = nix::sys::signal::kill(pid, nix::sys::signal::Signal::SIGHUP);
        match nix::sys::wait::waitpid(pid, Some(nix::sys::wait::WaitPidFlag::WNOHANG)) {
            Ok(nix::sys::wait::WaitStatus::StillAlive) | Err(_) => {
                std::thread::sleep(std::time::Duration::from_millis(100));
                if matches!(
                    nix::sys::wait::waitpid(pid, Some(nix::sys::wait::WaitPidFlag::WNOHANG)),
                    Ok(nix::sys::wait::WaitStatus::StillAlive) | Err(_)
                ) {
                    let _ = nix::sys::signal::kill(pid, nix::sys::signal::Signal::SIGKILL);
                    let _ = nix::sys::wait::waitpid(pid, None);
                }
            }
            Ok(_) => {} // Already exited
        }
    }

    /// Get the current working directory of the child process via /proc.
    pub fn cwd(&self) -> Option<String> {
        let path = format!("/proc/{}/cwd", self.child_pid);
        std::fs::read_link(path).ok().map(|p| p.to_string_lossy().into_owned())
    }

    pub fn resize(&self, cols: u16, rows: u16) {
        let ws = nix::pty::Winsize {
            ws_row: rows,
            ws_col: cols,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        unsafe {
            libc::ioctl(
                self.master.as_raw_fd(),
                libc::TIOCSWINSZ,
                &ws as *const nix::pty::Winsize,
            );
        }
    }
}

fn take_buffered_chunk(
    buffered: &mut Vec<u8>,
    rx: &Receiver<Vec<u8>>,
    max_bytes: usize,
    alive: &mut bool,
) -> Option<(Vec<u8>, bool)> {
    if max_bytes == 0 {
        return None;
    }

    while buffered.len() < max_bytes {
        match rx.try_recv() {
            Ok(chunk) => buffered.extend_from_slice(&chunk),
            Err(TryRecvError::Empty) => break,
            Err(TryRecvError::Disconnected) => {
                *alive = false;
                break;
            }
        }
    }

    // Probe one more queued chunk when we exactly hit the budget so callers
    // can tell there is more output ready to drain next frame.
    if buffered.len() == max_bytes {
        match rx.try_recv() {
            Ok(chunk) => buffered.extend_from_slice(&chunk),
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => {
                *alive = false;
            }
        }
    }

    if buffered.is_empty() && !*alive {
        return None;
    }

    if buffered.is_empty() {
        return None;
    }

    let data = if buffered.len() <= max_bytes {
        std::mem::take(buffered)
    } else {
        let remaining = buffered.split_off(max_bytes);
        std::mem::replace(buffered, remaining)
    };

    let has_more = !buffered.is_empty();
    Some((data, has_more))
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;

    use super::take_buffered_chunk;

    #[test]
    fn read_preserves_unconsumed_output_between_calls() {
        let (tx, rx) = mpsc::channel();
        tx.send(vec![b'a'; 4]).unwrap();
        tx.send(vec![b'b'; 4]).unwrap();
        drop(tx);

        let mut buffered = Vec::new();
        let mut alive = true;

        let (first, has_more) = take_buffered_chunk(&mut buffered, &rx, 4, &mut alive).unwrap();
        assert_eq!(first, vec![b'a'; 4]);
        assert!(has_more);
        assert_eq!(buffered, vec![b'b'; 4]);

        let (second, has_more) = take_buffered_chunk(&mut buffered, &rx, 4, &mut alive).unwrap();
        assert_eq!(second, vec![b'b'; 4]);
        assert!(!has_more);
        assert!(!alive);
    }

    #[test]
    fn read_marks_more_output_when_budget_boundary_is_exact() {
        let (tx, rx) = mpsc::channel();
        tx.send(vec![1; 4]).unwrap();
        tx.send(vec![2; 4]).unwrap();

        let mut buffered = Vec::new();
        let mut alive = true;

        let (first, has_more) = take_buffered_chunk(&mut buffered, &rx, 4, &mut alive).unwrap();
        assert_eq!(first, vec![1; 4]);
        assert!(has_more);
        assert_eq!(buffered, vec![2; 4]);
        assert!(alive);
    }
}

impl Drop for Pty {
    fn drop(&mut self) {
        self.graceful_kill();
    }
}

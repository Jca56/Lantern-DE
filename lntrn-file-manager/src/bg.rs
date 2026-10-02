//! Work that must not run on the render thread: anything that waits on a
//! device, a child process or the network.
//!
//! `Task::spawn` runs a closure on a thread of its own and hands its result
//! back through `poll`, which never blocks. When the result is ready the
//! worker pokes one process-wide eventfd; the main loop has that fd in its
//! poll set, so an idle window wakes at once, collects the result (see
//! `App::idle_tick`) and draws. Frames keep coming while a task is pending
//! because nothing on the main thread waits for it.
//!
//! A worker that dies without a result (a panic, a thread that could not be
//! started) is reported as `Lost`, so a "working…" state never hangs.

use std::os::fd::RawFd;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::OnceLock;

/// What `Task::poll` found.
pub enum Polled<T> {
    /// Still running.
    Pending,
    /// The result, handed over exactly once.
    Ready(T),
    /// The worker ended without one. Also what every poll after `Ready`
    /// returns: a finished task has nothing more to give.
    Lost,
}

/// A piece of blocking work running off the main thread.
pub struct Task<T> {
    rx: Receiver<T>,
}

impl<T: Send + 'static> Task<T> {
    /// Run `work` on a new thread named `name`.
    pub fn spawn(name: &str, work: impl FnOnce() -> T + Send + 'static) -> Self {
        let (tx, rx) = mpsc::channel();
        let spawned = std::thread::Builder::new()
            .name(name.to_string())
            .spawn(move || {
                // Declared before `tx` so it drops after it: by the time the
                // loop wakes, the channel already holds the result, or is
                // closed if `work` panicked.
                let _wake = WakeOnDrop;
                let tx = tx;
                let _ = tx.send(work());
            });
        if spawned.is_err() {
            // The closure (and the sender in it) is gone: the first poll
            // reports `Lost`. Make sure that poll happens.
            wake();
        }
        Self { rx }
    }

    /// Never blocks.
    pub fn poll(&mut self) -> Polled<T> {
        match self.rx.try_recv() {
            Ok(value) => Polled::Ready(value),
            Err(TryRecvError::Empty) => Polled::Pending,
            Err(TryRecvError::Disconnected) => Polled::Lost,
        }
    }
}

struct WakeOnDrop;

impl Drop for WakeOnDrop {
    fn drop(&mut self) {
        wake();
    }
}

static WAKE_FD: OnceLock<RawFd> = OnceLock::new();

/// The eventfd the main loop polls next to the Wayland socket. `None` when
/// the kernel refused one; results are then picked up at the loop's idle
/// timeout instead of at once.
pub fn wake_fd() -> Option<RawFd> {
    let fd = *WAKE_FD
        .get_or_init(|| unsafe { libc::eventfd(0, libc::EFD_NONBLOCK | libc::EFD_CLOEXEC) });
    (fd >= 0).then_some(fd)
}

/// Wake the main loop. Callable from any thread.
pub fn wake() {
    if let Some(fd) = wake_fd() {
        let one: u64 = 1;
        unsafe {
            libc::write(fd, &one as *const u64 as *const libc::c_void, 8);
        }
    }
}

/// Clear the eventfd after poll() reported it readable.
pub fn drain_wake() {
    if let Some(fd) = wake_fd() {
        let mut buf = 0u64;
        unsafe {
            libc::read(fd, &mut buf as *mut u64 as *mut libc::c_void, 8);
        }
    }
}

/// One to three dots that advance over time, for a "Formatting…" that
/// visibly lives. (`App::idle_tick` redraws while such a state is shown.)
pub fn dots() -> &'static str {
    static START: OnceLock<std::time::Instant> = OnceLock::new();
    let ms = START
        .get_or_init(std::time::Instant::now)
        .elapsed()
        .as_millis();
    [".", "..", "..."][(ms / 400 % 3) as usize]
}


/// Run `cmd` and collect what it prints, but not for ever: a child that has
/// not ended after `limit` is killed, with whatever it started. `None` when
/// it could not be started or had to be given up on.
///
/// For helpers that read files Fox did not choose (git on a repository,
/// ffmpeg on a video): a FIFO among those files makes them wait for a
/// writer that never comes, and the worker that waits for them with it.
pub fn output_with_deadline(
    cmd: &mut std::process::Command,
    limit: std::time::Duration,
) -> Option<(std::process::ExitStatus, Vec<u8>)> {
    use std::io::Read;
    use std::os::unix::process::CommandExt;
    // A group of its own, so that giving up takes its children too.
    let mut child = cmd
        .stdout(std::process::Stdio::piped())
        .process_group(0)
        .spawn()
        .ok()?;
    let mut stdout = child.stdout.take()?;
    // Read while it runs: a large output would fill the pipe and stall it.
    let reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = stdout.read_to_end(&mut bytes);
        bytes
    });
    let deadline = std::time::Instant::now() + limit;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if std::time::Instant::now() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            _ => {
                unsafe {
                    libc::kill(-(child.id() as libc::pid_t), libc::SIGKILL);
                }
                let _ = child.wait();
                break None;
            }
        }
    };
    let bytes = reader.join().ok()?;
    Some((status?, bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn wait<T: Send + 'static>(task: &mut Task<T>) -> Polled<T> {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            match task.poll() {
                Polled::Pending if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(2));
                }
                other => return other,
            }
        }
    }

    #[test]
    fn a_task_hands_its_result_over_once() {
        let mut task = Task::spawn("fox-test", || 6 * 7);
        assert!(matches!(wait(&mut task), Polled::Ready(42)));
        assert!(matches!(task.poll(), Polled::Lost));
    }

    #[test]
    fn a_pending_task_does_not_block_the_caller() {
        let (release, gate) = mpsc::channel::<()>();
        let mut task = Task::spawn("fox-test", move || gate.recv().is_ok());
        assert!(matches!(task.poll(), Polled::Pending));
        release.send(()).unwrap();
        assert!(matches!(wait(&mut task), Polled::Ready(true)));
    }

    #[test]
    fn a_worker_that_panics_is_reported_lost() {
        let mut task: Task<u32> = Task::spawn("fox-test", || panic!("worker died"));
        assert!(matches!(wait(&mut task), Polled::Lost));
    }

    #[test]
    fn a_finished_task_wakes_the_loop() {
        let fd = wake_fd().expect("eventfd");
        let mut task = Task::spawn("fox-test", || ());
        let mut pfd = libc::pollfd {
            fd,
            events: libc::POLLIN,
            revents: 0,
        };
        let ready = unsafe { libc::poll(&mut pfd, 1, 10_000) };
        assert_eq!(ready, 1, "the wake fd never became readable");
        assert!(matches!(wait(&mut task), Polled::Ready(())));
    }
}

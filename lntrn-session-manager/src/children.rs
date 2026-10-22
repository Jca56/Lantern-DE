//! Everything the session starts and has to answer for: the compositor, the
//! shell daemons it keeps running, and the autostart entries.
//!
//! Only pids are kept, and [`Children::reap`] is the one place that waits on
//! them. A child that has not been reaped there still holds its pid, so a
//! signal sent to it can never land on a stranger; once reaped, the pid is
//! forgotten.

use std::fs::OpenOptions;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use nix::sys::signal::{kill, Signal};
use nix::sys::wait::{waitpid, WaitPidFlag, WaitStatus};
use nix::unistd::Pid;

use crate::backoff::Backoff;
use crate::exit::Exit;
use crate::log::log;

struct Daemon {
    name: &'static str,
    args: &'static [&'static str],
    /// `None` while it is not running.
    pid: Option<i32>,
    backoff: Backoff,
    /// When to start it again, and what it died of.
    restart: Option<(Instant, Exit)>,
    /// Its log has been started for this session; later starts add to it.
    log_started: bool,
    /// Being stopped on purpose (the other panel was chosen): its end is
    /// not a death, and it is not started again.
    retiring: bool,
}

pub struct Children {
    log_dir: PathBuf,
    compositor: Option<i32>,
    compositor_exit: Option<Exit>,
    daemons: Vec<Daemon>,
    /// Autostart entries still running, to put a name to them when they end.
    autostart: Vec<(i32, String)>,
    /// The session is ending: children are expected to go, and stay gone.
    stopping: bool,
}

impl Children {
    /// `log_dir` is where each child's output goes, as `<name>.log`.
    pub fn new(log_dir: PathBuf) -> Self {
        Children {
            log_dir,
            compositor: None,
            compositor_exit: None,
            daemons: Vec::new(),
            autostart: Vec::new(),
            stopping: false,
        }
    }

    /// Start the compositor on the DRM backend. Returns its pid.
    pub fn start_compositor(&mut self, program: &Path) -> std::io::Result<i32> {
        let (out, err) = child_log(&self.log_dir, "compositor", true);
        let child = Command::new(program)
            .arg("--udev")
            .env("RUST_BACKTRACE", "1")
            .env_remove("DISPLAY")
            .env_remove("WAYLAND_DISPLAY")
            .stdout(out)
            .stderr(err)
            .spawn()?;
        let pid = child.id() as i32;
        self.compositor = Some(pid);
        Ok(pid)
    }

    pub fn compositor_running(&self) -> bool {
        self.compositor.is_some()
    }

    /// How the compositor ended, once it has.
    pub fn compositor_exit(&self) -> Option<Exit> {
        self.compositor_exit
    }

    /// Start a shell daemon and keep it running for the rest of the session.
    /// It has to stay in the foreground: the process started here is the one
    /// that is watched.
    pub fn start_daemon(&mut self, name: &'static str, args: &'static [&'static str]) {
        self.daemons.push(Daemon {
            name,
            args,
            pid: None,
            backoff: Backoff::default(),
            restart: None,
            log_started: false,
            retiring: false,
        });
        self.spawn_daemon(self.daemons.len() - 1);
    }

    /// Stop the daemon started as `name` for good: ask it to quit, kill it
    /// if it hasn't within `grace`, and forget it.
    pub fn stop_daemon(&mut self, name: &str, grace: Duration) {
        let mut pids = Vec::new();
        for daemon in self.daemons.iter_mut().filter(|d| d.name == name) {
            daemon.retiring = true;
            daemon.restart = None;
            pids.extend(daemon.pid);
        }
        self.stop(&pids, grace);
        self.daemons.retain(|d| !(d.name == name && d.retiring));
    }

    fn spawn_daemon(&mut self, index: usize) {
        let daemon = &mut self.daemons[index];
        daemon.restart = None;
        let (out, err) = child_log(&self.log_dir, daemon.name, !daemon.log_started);
        daemon.log_started = true;
        match Command::new(daemon.name)
            .args(daemon.args)
            .stdin(Stdio::null())
            .stdout(out)
            .stderr(err)
            .spawn()
        {
            Ok(child) => {
                daemon.pid = Some(child.id() as i32);
                log(&format!("🏮 Started {} (pid {})", daemon.name, child.id()));
            }
            Err(e) => log(&format!("🏮 WARNING: could not start {}: {e}", daemon.name)),
        }
    }

    /// Note an autostart entry that was just started.
    pub fn adopt_autostart(&mut self, pid: i32, name: String) {
        self.autostart.push((pid, name));
    }

    /// Take note of every child that has ended since the last call.
    ///
    /// This is done by hand on purpose. Do NOT set SA_NOCLDWAIT (or ignore
    /// SIGCHLD) to have the kernel do it: that is inherited across exec() by
    /// everything the session starts, where it makes Rust's
    /// `Command::output()` / `status()` fail with ECHILD, and every nmcli /
    /// wpctl / bluetoothctl call from the shell then reports "unavailable".
    pub fn reap(&mut self) {
        loop {
            let (pid, exit) = match waitpid(Pid::from_raw(-1), Some(WaitPidFlag::WNOHANG)) {
                Ok(WaitStatus::Exited(pid, code)) => (pid, Exit::Code(code)),
                Ok(WaitStatus::Signaled(pid, signal, core)) => (pid, Exit::Signal(signal, core)),
                // Nothing has ended, or (ECHILD) there are no children left.
                Ok(WaitStatus::StillAlive) | Err(_) => break,
                Ok(_) => continue,
            };
            self.ended(pid.as_raw(), exit, Instant::now());
        }
    }

    fn ended(&mut self, pid: i32, exit: Exit, now: Instant) {
        if self.compositor == Some(pid) {
            self.compositor = None;
            self.compositor_exit = Some(exit);
        } else if let Some(daemon) = self.daemons.iter_mut().find(|d| d.pid == Some(pid)) {
            daemon.pid = None;
            daemon.restart = None;
            if self.stopping {
                return;
            }
            if daemon.retiring {
                log(&format!("🏮 Stopped {} (pid {pid})", daemon.name));
                return;
            }
            // Said plainly, with no verdict yet: at logout the daemons lose
            // the compositor and go a moment before it is seen to be gone.
            // Whether this was a crash shows in what follows.
            log(&format!("🏮 {} (pid {pid}) ended: {exit}", daemon.name));
            // A clean exit is its own decision and is left alone.
            if exit.is_clean() {
                return;
            }
            match daemon.backoff.next(now) {
                Some(delay) => daemon.restart = Some((now + delay, exit)),
                None => log(&format!(
                    "🏮 WARNING: {} has died {} times within a minute and stays down until the next login; see {}.log",
                    daemon.name,
                    Backoff::MAX_DEATHS,
                    daemon.name
                )),
            }
        } else if let Some(index) = self.autostart.iter().position(|(p, _)| *p == pid) {
            let (_, name) = self.autostart.swap_remove(index);
            if !self.stopping {
                log(&format!("🏮 Autostart {name} (pid {pid}) ended: {exit}"));
            }
        }
    }

    /// Start again the daemons that died and have waited out their delay.
    pub fn restart_due(&mut self, now: Instant) {
        for index in 0..self.daemons.len() {
            let daemon = &self.daemons[index];
            let Some((at, exit)) = daemon.restart else {
                continue;
            };
            if now < at {
                continue;
            }
            log(&format!(
                "🏮 WARNING: {} died ({exit}), starting it again; see {}.log",
                daemon.name, daemon.name
            ));
            self.spawn_daemon(index);
        }
    }

    /// Ask the shell daemons to quit, last started first, and kill whichever
    /// have not within `grace`.
    pub fn stop_daemons(&mut self, grace: Duration) {
        self.stopping = true;
        let pids: Vec<i32> = self.daemons.iter().rev().filter_map(|d| d.pid).collect();
        self.stop(&pids, grace);
    }

    /// The same for the compositor.
    pub fn stop_compositor(&mut self, grace: Duration) {
        self.stopping = true;
        let pids: Vec<i32> = self.compositor.into_iter().collect();
        self.stop(&pids, grace);
    }

    fn stop(&mut self, pids: &[i32], grace: Duration) {
        for &pid in pids {
            let _ = kill(Pid::from_raw(pid), Signal::SIGTERM);
        }
        if self.wait_gone(pids, grace) {
            return;
        }
        for &pid in pids {
            if let Some(name) = self.running(pid) {
                log(&format!(
                    "🏮 WARNING: {name} (pid {pid}) was still up {:?} after SIGTERM, killing it",
                    grace
                ));
                let _ = kill(Pid::from_raw(pid), Signal::SIGKILL);
            }
        }
        self.wait_gone(pids, Duration::from_secs(1));
    }

    /// Whether all of `pids` ended within `limit`.
    fn wait_gone(&mut self, pids: &[i32], limit: Duration) -> bool {
        let deadline = Instant::now() + limit;
        loop {
            self.reap();
            if pids.iter().all(|&pid| self.running(pid).is_none()) {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// The name of the compositor or daemon running as `pid`, if one is.
    fn running(&self, pid: i32) -> Option<&'static str> {
        if self.compositor == Some(pid) {
            return Some("lntrn-compositor");
        }
        self.daemons.iter().find(|d| d.pid == Some(pid)).map(|d| d.name)
    }
}

/// Where a child's stdout and stderr go: `<name>.log` in `dir`.
///
/// `fresh` starts a new log and keeps the one before it as `<name>.log.1`: a
/// problem that ends in a reboot is otherwise wiped by the very restart that
/// follows it. Without `fresh` the log is added to, so what a daemon said
/// before it died is still there after its restart.
fn child_log(dir: &Path, name: &str, fresh: bool) -> (Stdio, Stdio) {
    let path = dir.join(format!("{name}.log"));
    if fresh {
        let _ = std::fs::rename(&path, dir.join(format!("{name}.log.1")));
    }
    let file = OpenOptions::new().create(true).append(true).open(&path);
    match file.and_then(|file| Ok((file.try_clone()?, file))) {
        Ok((out, err)) => (Stdio::from(out), Stdio::from(err)),
        Err(_) => (Stdio::null(), Stdio::null()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testdir::TestDir;

    const SEGV: Exit = Exit::Signal(Signal::SIGSEGV, true);

    /// Children with one daemon that counts as running under `pid`, without
    /// anything being started.
    fn with_daemon(dir: &TestDir, pid: i32) -> Children {
        let mut children = Children::new(dir.path().to_owned());
        children.daemons.push(Daemon {
            name: "lntrn-test-daemon",
            args: &[],
            pid: Some(pid),
            backoff: Backoff::default(),
            restart: None,
            log_started: false,
            retiring: false,
        });
        children
    }

    #[test]
    fn a_crashed_daemon_is_due_again_after_its_delay() {
        let dir = TestDir::new("crash");
        let mut children = with_daemon(&dir, 4242);
        let now = Instant::now();

        children.ended(4242, SEGV, now);
        assert_eq!(children.daemons[0].pid, None);
        assert_eq!(children.daemons[0].restart, Some((now + Duration::from_secs(1), SEGV)));
        // The pid is forgotten with it: nothing is left to signal.
        assert_eq!(children.running(4242), None);
    }

    #[test]
    fn a_daemon_that_exits_cleanly_is_left_alone() {
        let dir = TestDir::new("clean");
        let mut children = with_daemon(&dir, 4242);
        children.ended(4242, Exit::Code(0), Instant::now());
        assert_eq!(children.daemons[0].pid, None);
        assert_eq!(children.daemons[0].restart, None);
    }

    #[test]
    fn a_daemon_that_keeps_dying_is_given_up_on() {
        let dir = TestDir::new("loop");
        let mut children = with_daemon(&dir, 100);
        let start = Instant::now();
        for i in 0..5 {
            children.daemons[0].pid = Some(100 + i);
            children.ended(100 + i, Exit::Code(1), start + Duration::from_secs(i as u64));
        }
        assert_eq!(children.daemons[0].restart, None);
    }

    #[test]
    fn nothing_is_restarted_while_the_session_is_stopping() {
        let dir = TestDir::new("stopping");
        let mut children = with_daemon(&dir, 4242);
        children.stopping = true;
        children.ended(4242, SEGV, Instant::now());
        assert_eq!(children.daemons[0].restart, None);
    }

    #[test]
    fn a_daemon_stopped_on_purpose_is_not_a_death() {
        let dir = TestDir::new("retire");
        let mut children = with_daemon(&dir, 4242);
        children.daemons[0].retiring = true;
        children.ended(4242, Exit::Signal(Signal::SIGTERM, false), Instant::now());
        assert_eq!((children.daemons[0].pid, children.daemons[0].restart), (None, None));
    }

    #[test]
    fn the_compositors_end_is_kept() {
        let dir = TestDir::new("compositor");
        let mut children = with_daemon(&dir, 4242);
        children.compositor = Some(7);
        assert!(children.compositor_running());

        // A pid that is nobody's changes nothing.
        children.ended(9999, SEGV, Instant::now());
        assert!(children.compositor_running());

        children.ended(7, SEGV, Instant::now());
        assert!(!children.compositor_running());
        assert_eq!(children.compositor_exit(), Some(SEGV));
    }

    #[test]
    fn a_fresh_log_keeps_the_one_before_it() {
        let dir = TestDir::new("log");
        let read = |name: &str| std::fs::read_to_string(dir.path().join(name)).unwrap_or_default();
        std::fs::write(dir.path().join("thing.log"), "last session\n").unwrap();

        drop(child_log(dir.path(), "thing", true));
        assert_eq!(read("thing.log.1"), "last session\n");
        assert_eq!(read("thing.log"), "");

        // A restart inside the session adds to the log it already has.
        std::fs::write(dir.path().join("thing.log"), "before the crash\n").unwrap();
        drop(child_log(dir.path(), "thing", false));
        assert_eq!(read("thing.log"), "before the crash\n");
        assert_eq!(read("thing.log.1"), "last session\n");
    }

    /// The one test that runs real processes. `reap` waits on any child of
    /// this process, so a second test doing the same alongside it would have
    /// its children collected here: keep them all in this function.
    #[test]
    fn real_children_are_started_reaped_restarted_and_stopped() {
        let dir = TestDir::new("real");
        let mut children = Children::new(dir.path().to_owned());
        let wait_until = |children: &mut Children, done: &dyn Fn(&Children) -> bool| {
            let deadline = Instant::now() + Duration::from_secs(10);
            while !done(children) {
                assert!(Instant::now() < deadline, "timed out");
                std::thread::sleep(Duration::from_millis(10));
                children.reap();
            }
        };

        // Dies at once with an error: reaped, its output kept, due again.
        children.start_daemon("sh", &["-c", "echo it broke >&2; exit 3"]);
        let first = children.daemons[0].pid.expect("sh starts");
        wait_until(&mut children, &|c| c.daemons[0].pid.is_none());
        let (at, exit) = children.daemons[0].restart.expect("a restart is due");
        assert_eq!(exit, Exit::Code(3));

        children.restart_due(at - Duration::from_millis(1));
        assert_eq!(children.daemons[0].pid, None, "not before its delay");
        children.restart_due(at);
        assert_ne!(children.daemons[0].pid.expect("started again"), first);
        wait_until(&mut children, &|c| c.daemons[0].pid.is_none());
        let log = std::fs::read_to_string(dir.path().join("sh.log")).unwrap();
        assert_eq!(log, "it broke\nit broke\n", "the restart adds to the same log");
        children.daemons.clear();

        // Quits when asked.
        children.start_daemon("sleep", &["30"]);
        // Has to be killed: it ignores SIGTERM.
        children.start_daemon("sh", &["-c", "trap '' TERM; while :; do sleep 0.05; done"]);
        assert!(children.daemons.iter().all(|d| d.pid.is_some()));
        std::thread::sleep(Duration::from_millis(200));

        // One of them stopped on its own, for good: the other runs on.
        children.start_daemon("sleep", &["31"]);
        children.stop_daemon("sleep", Duration::from_millis(300));
        assert_eq!(children.daemons.len(), 1, "both sleeps are gone, the sh is not");
        assert!(children.daemons[0].pid.is_some());
        children.restart_due(Instant::now() + Duration::from_secs(60));
        assert_eq!(children.daemons.len(), 1, "nothing came back");

        let began = Instant::now();
        children.stop_daemons(Duration::from_millis(300));
        assert!(children.daemons.iter().all(|d| d.pid.is_none()));
        assert!(children.daemons.iter().all(|d| d.restart.is_none()));
        assert!(began.elapsed() < Duration::from_secs(5));
    }
}

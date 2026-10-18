//! The bar's line to the compositor: its workspace socket, read on a
//! thread of its own and written from the UI. In come whether a maximized
//! or fullscreen window shows on our monitor (`covered:<output>:<0|1>`),
//! so the bar docks, every open window (`win:` / `win-gone:`), for the
//! dock, and each monitor's workspaces (`state:<output>:<active>:<ids>`),
//! for the tray's number, and a Super tap's `launcher:toggle`, to open or
//! close the launcher (or, the bar away with auto hide, to peek). Out go the dock's commands: activate, minimize
//! or close a window, the tray's turn to the next workspace,
//! where its icons are (minimizing flies a window into its app's icon),
//! and which windows its previews want live thumbnails of. The thread
//! wakes the loop when something it read changed.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use lntrn_app::Waker;
use lntrn_core::{log_info, log_warn};

unsafe extern "C" {
    fn getuid() -> u32;
}

/// How long before trying the socket again when the compositor isn't
/// listening (yet).
const RETRY: Duration = Duration::from_secs(2);
/// How long a write may take before the line is given up on: the UI
/// thread writes, and must never hang on a compositor that has.
const WRITE_TIMEOUT: Duration = Duration::from_millis(100);

/// Where the compositor listens: `/run/user/<uid>/lntrn-workspaces.sock`.
fn socket_path() -> PathBuf {
    // SAFETY: a plain syscall that can't fail.
    PathBuf::from(format!("/run/user/{}/lntrn-workspaces.sock", unsafe { getuid() }))
}

/// An open window, as the compositor tells it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Window {
    /// The compositor's, stable for the window's life; later is newer.
    pub id: u64,
    /// What the window calls itself: its Wayland app_id or X11 class.
    pub app_id: String,
    pub title: String,
    pub minimized: bool,
    /// It has the focus.
    pub active: bool,
}

/// A monitor's workspaces, as the compositor tells them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Workspaces {
    /// The monitor's name (`DP-1`): what commands about it say.
    pub output: String,
    /// The one showing.
    pub active: u32,
    /// The ones with windows, and the one showing, ascending.
    pub ids: Vec<u32>,
}

/// The line to the compositor, shared by the UI and its reading thread.
#[derive(Clone)]
pub struct Link {
    shared: Arc<Shared>,
}

struct Shared {
    covered: AtomicBool,
    /// A Super tap asked for the launcher, and it hasn't been seen to.
    launcher: AtomicBool,
    /// Our monitor's workspaces (any one's, while we don't know ours).
    workspaces: Mutex<Option<Workspaces>>,
    /// Oldest first.
    windows: Mutex<Vec<Window>>,
    /// Bumped whenever `windows` changes.
    revision: AtomicU64,
    /// The connection to write on; `None` while there is none.
    writer: Mutex<Option<UnixStream>>,
    /// Bumped on each new connection: whatever the compositor was told on
    /// the one before is to be told again.
    connection: AtomicU64,
    /// Every line sent, for the tests to read back.
    #[cfg(test)]
    sent: Mutex<Vec<String>>,
}

impl Link {
    pub fn new() -> Link {
        let shared = Shared {
            covered: AtomicBool::new(false),
            launcher: AtomicBool::new(false),
            workspaces: Mutex::new(None),
            windows: Mutex::new(Vec::new()),
            revision: AtomicU64::new(0),
            writer: Mutex::new(None),
            connection: AtomicU64::new(0),
            #[cfg(test)]
            sent: Mutex::new(Vec::new()),
        };
        Link { shared: Arc::new(shared) }
    }

    /// Read the compositor's word from a thread of its own, keeping to
    /// whether `output` is covered (any monitor, when the compositor
    /// picked ours and we can't know which) and waking the loop on a change.
    pub fn start(&self, output: Option<String>, waker: Waker) {
        let shared = self.shared.clone();
        let spawned = std::thread::Builder::new().name("compositor".into()).spawn(move || {
            let mut said = false;
            loop {
                if let Ok(stream) = UnixStream::connect(socket_path()) {
                    log_info!("compositor: connected");
                    shared.connected(&stream);
                    waker.wake();
                    shared.listen(stream, output.as_deref(), &waker);
                    shared.hung_up();
                    waker.wake();
                    log_info!("compositor: hung up");
                } else if !said {
                    said = true;
                    log_warn!("compositor: no socket at {}; floating, with no windows on the dock, until there is", socket_path().display());
                }
                std::thread::sleep(RETRY);
            }
        });
        if let Err(e) = spawned {
            log_warn!("compositor: no thread: {e}; the bar stays floating and the dock empty");
        }
    }

    /// Whether a maximized or fullscreen window shows on our monitor.
    pub fn covered(&self) -> bool {
        self.shared.covered.load(Ordering::Acquire)
    }

    /// Whether a Super tap asked for the launcher since this was last
    /// asked.
    pub fn take_launcher(&self) -> bool {
        self.shared.launcher.swap(false, Ordering::AcqRel)
    }

    /// Our monitor's workspaces, once the compositor has said.
    pub fn workspaces(&self) -> Option<Workspaces> {
        self.shared.workspaces.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    /// Bumped whenever the windows change.
    pub fn revision(&self) -> u64 {
        self.shared.revision.load(Ordering::Acquire)
    }

    /// The open windows, oldest first.
    pub fn windows(&self) -> Vec<Window> {
        self.shared.windows.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    /// Bumped on each new connection to the compositor.
    pub fn connection(&self) -> u64 {
        self.shared.connection.load(Ordering::Acquire)
    }

    /// Send the compositor one line; dropped when it isn't there.
    pub fn send(&self, line: &str) {
        #[cfg(test)]
        self.shared.sent.lock().unwrap_or_else(PoisonError::into_inner).push(line.to_owned());
        let mut writer = self.shared.writer.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(stream) = writer.as_mut() else { return };
        if let Err(e) = stream.write_all(format!("{line}\n").as_bytes()) {
            log_warn!("compositor: can't send {line:?}: {e}");
        }
    }
}

/// Lines told the compositor, by what each is about: one goes again only
/// when it changes, or the connection does (a compositor started over
/// knows nothing).
#[derive(Default)]
pub struct Told(std::collections::HashMap<&'static str, (u64, String)>);

impl Told {
    pub fn tell(&mut self, link: &Link, about: &'static str, line: String) {
        let told = (link.connection(), line);
        if self.0.get(about) != Some(&told) {
            link.send(&told.1);
            self.0.insert(about, told);
        }
    }
}

#[cfg(test)]
impl Link {
    /// Take in a line as though the compositor said it.
    pub fn feed(&self, line: &str) {
        if line.trim() == "launcher:toggle" {
            self.shared.launcher.store(true, Ordering::Release);
            return;
        }
        if apply(&mut self.shared.windows.lock().unwrap(), line) {
            self.shared.revision.fetch_add(1, Ordering::AcqRel);
        }
    }

    /// Every line sent so far, and forget them.
    pub fn take_sent(&self) -> Vec<String> {
        std::mem::take(&mut self.shared.sent.lock().unwrap())
    }
}

impl Shared {
    fn connected(&self, stream: &UnixStream) {
        let writer = stream.try_clone().and_then(|w| w.set_write_timeout(Some(WRITE_TIMEOUT)).map(|()| w));
        match writer {
            Ok(w) => *self.writer.lock().unwrap_or_else(PoisonError::into_inner) = Some(w),
            Err(e) => log_warn!("compositor: can't write to it: {e}"),
        }
        self.connection.fetch_add(1, Ordering::AcqRel);
    }

    /// The compositor went: no windows, nothing to write to.
    fn hung_up(&self) {
        *self.writer.lock().unwrap_or_else(PoisonError::into_inner) = None;
        *self.workspaces.lock().unwrap_or_else(PoisonError::into_inner) = None;
        self.windows.lock().unwrap_or_else(PoisonError::into_inner).clear();
        self.revision.fetch_add(1, Ordering::AcqRel);
    }

    /// Take in lines until the connection ends.
    fn listen(&self, stream: UnixStream, output: Option<&str>, waker: &Waker) {
        let mut outputs: HashMap<String, bool> = HashMap::new();
        for line in BufReader::new(stream).lines() {
            let Ok(line) = line else { break };
            if line.trim() == "launcher:toggle" {
                self.launcher.store(true, Ordering::Release);
                waker.wake();
                continue;
            }
            if let Some((name, on)) = parse_covered(&line) {
                outputs.insert(name.to_owned(), on);
                let now = ours(&outputs, output);
                if self.covered.swap(now, Ordering::AcqRel) != now {
                    waker.wake();
                }
                continue;
            }
            if let Some(ws) = parse_state(&line) {
                // Ours, or the first heard of while we don't know ours.
                let mut shown = self.workspaces.lock().unwrap_or_else(PoisonError::into_inner);
                let take = match output {
                    Some(name) => ws.output == name,
                    None => shown.as_ref().is_none_or(|s| s.output == ws.output),
                };
                if take && shown.as_ref() != Some(&ws) {
                    *shown = Some(ws);
                    waker.wake();
                }
                continue;
            }
            let mut windows = self.windows.lock().unwrap_or_else(PoisonError::into_inner);
            if !apply(&mut windows, &line) {
                continue;
            }
            drop(windows);
            self.revision.fetch_add(1, Ordering::AcqRel);
            waker.wake();
        }
    }
}

/// Take a window line into `windows`, kept oldest first. Returns whether
/// it was one.
fn apply(windows: &mut Vec<Window>, line: &str) -> bool {
    if let Some(window) = parse_window(line) {
        match windows.binary_search_by_key(&window.id, |w| w.id) {
            Ok(at) => windows[at] = window,
            Err(at) => windows.insert(at, window),
        }
        return true;
    }
    if let Some(id) = line.strip_prefix("win-gone:").and_then(|id| id.trim().parse::<u64>().ok()) {
        windows.retain(|w| w.id != id);
        return true;
    }
    false
}

/// A `covered:<output>:<0|1>` line; `None` for any other.
fn parse_covered(line: &str) -> Option<(&str, bool)> {
    let (name, flag) = line.strip_prefix("covered:")?.rsplit_once(':')?;
    let on = match flag.trim() {
        "1" => true,
        "0" => false,
        _ => return None,
    };
    (!name.is_empty()).then_some((name, on))
}

/// A `state:<output>:<active>:<id>,<id>…` line; `None` for any other.
fn parse_state(line: &str) -> Option<Workspaces> {
    let (output, rest) = line.strip_prefix("state:")?.split_once(':')?;
    let (active, ids) = rest.split_once(':')?;
    let ids = ids.split(',').filter(|id| !id.is_empty()).map(|id| id.trim().parse().ok()).collect::<Option<Vec<u32>>>()?;
    let active = active.trim().parse().ok()?;
    (!output.is_empty()).then(|| Workspaces { output: output.to_owned(), active, ids })
}

/// A `win:<id>\t<flags>\t<app_id>\t<title>` line; `None` for any other.
fn parse_window(line: &str) -> Option<Window> {
    let mut fields = line.strip_prefix("win:")?.splitn(4, '\t');
    let id = fields.next()?.parse().ok()?;
    let flags = fields.next()?;
    let app_id = fields.next()?.to_owned();
    let title = fields.next().unwrap_or_default().to_owned();
    Some(Window { id, app_id, title, minimized: flags.contains('m'), active: flags.contains('a') })
}

/// Whether our monitor is covered: the one we're on, or any of them when
/// we don't know which that is.
fn ours(outputs: &HashMap<String, bool>, output: Option<&str>) -> bool {
    match output {
        Some(name) => outputs.get(name).copied().unwrap_or(false),
        None => outputs.values().any(|on| *on),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_cover_lines_are_read_as_covers() {
        assert_eq!(parse_covered("covered:DP-1:1"), Some(("DP-1", true)));
        assert_eq!(parse_covered("covered:HDMI-A-1:0"), Some(("HDMI-A-1", false)));
        // The workspace lines, and anything malformed.
        assert_eq!(parse_covered("state:DP-1:1:1,2"), None);
        assert_eq!(parse_covered("covered:DP-1:yes"), None);
        assert_eq!(parse_covered("covered::1"), None);
        assert_eq!(parse_covered("covered:DP-1"), None);
    }

    #[test]
    fn workspace_lines_read_as_workspaces() {
        assert_eq!(parse_state("state:DP-1:3:1,3,5"), Some(Workspaces { output: "DP-1".into(), active: 3, ids: vec![1, 3, 5] }));
        assert_eq!(parse_state("state:HDMI-A-1:1:"), Some(Workspaces { output: "HDMI-A-1".into(), active: 1, ids: vec![] }));
        assert_eq!(parse_state("state:DP-1:x:1"), None);
        assert_eq!(parse_state("state:DP-1:1:1,y"), None);
        assert_eq!(parse_state("covered:DP-1:1"), None);
        assert_eq!(parse_state("state::1:1"), None);
    }

    #[test]
    fn our_monitor_alone_counts_unless_we_dont_know_it() {
        let outputs = HashMap::from([("DP-1".to_owned(), false), ("HDMI-A-1".to_owned(), true)]);
        assert!(!ours(&outputs, Some("DP-1")), "a maximized window on the other monitor");
        assert!(ours(&outputs, Some("HDMI-A-1")));
        assert!(!ours(&outputs, Some("eDP-1")), "not heard of: floating");
        assert!(ours(&outputs, None));
        assert!(!ours(&HashMap::new(), None));
    }

    #[test]
    fn the_window_feed_keeps_them_oldest_first() {
        let mut w = Vec::new();
        assert!(apply(&mut w, "win:5\t-\tfirefox\tGitHub"));
        assert!(apply(&mut w, "win:2\tam\tlntrn-terminal\t~/Projects: a:b"));
        assert_eq!(w.iter().map(|w| w.id).collect::<Vec<_>>(), vec![2, 5]);
        assert_eq!(w[0], Window { id: 2, app_id: "lntrn-terminal".into(), title: "~/Projects: a:b".into(), minimized: true, active: true });
        // A change in place, then one gone.
        assert!(apply(&mut w, "win:5\ta\tfirefox\tYouTube"));
        assert_eq!((w[1].title.as_str(), w[1].active), ("YouTube", true));
        assert!(apply(&mut w, "win-gone:2"));
        assert_eq!(w.len(), 1);
        assert!(!apply(&mut w, "covered:DP-1:1"));
        assert!(!apply(&mut w, "win:x\t-\ta\tb"));
    }
}

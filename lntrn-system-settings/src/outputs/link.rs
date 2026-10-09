//! The socket to the compositor and the thread that reads it. The thread
//! sleeps in a read until the compositor has something to say, keeps
//! what the page shows up to date, and wakes the window when that
//! changed. A new setup is written by whoever asks for it, at once when
//! the compositor has settled and otherwise as soon as it has.
//!
//! A connection is not kept past anything that could leave it wrong:
//! after an apply was answered, and when a monitor went away, it is
//! dropped and made again, so what the page shows is what the compositor
//! says on a fresh look, and the ids we hold are the ones it means. How
//! an apply ended is only told once that fresh look is in, so whoever
//! hears it sees the monitors as it left them.

use std::io::{Read, Write};
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};

use lntrn_app::Waker;
use lntrn_core::{log_error, log_info};

use super::engine::{Engine, Note};
use super::wire::Reader;
use super::{Change, Head, Outcome};

/// Connections in a row that ended before the compositor had said how
/// things are, after which we stop trying.
const MAX_FALSE_STARTS: u32 = 3;
/// Times a setup is sent again when the monitors changed under it.
const MAX_TRIES: u32 = 3;

/// How things stand with the compositor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Link {
    /// Not heard from yet.
    Connecting,
    Ready,
    /// The compositor has no output manager: not Lantern, or an old one.
    Missing,
    /// The connection is gone, and why.
    Lost(String),
}

/// What the page shows: the monitors as of `generation`, which goes up
/// each time the compositor has said how things are, changed or not.
#[derive(Clone, Debug, PartialEq)]
pub struct View {
    pub link: Link,
    pub heads: Vec<Head>,
    pub generation: u64,
}

/// A setup on its way to the compositor.
struct Job {
    changes: Vec<Change>,
    tries: u32,
    /// Send it once more after it took: a monitor that was off only comes
    /// on the first time, and takes its place, mode and scale the second.
    again: bool,
}

struct Shared {
    view: View,
    /// The connection: the protocol's state and the socket to write on.
    live: Option<(Engine, UnixStream)>,
    /// Waiting for the compositor to settle.
    queued: Option<Job>,
    /// Sent, not answered yet.
    flying: Option<Job>,
    /// Answered; told once the compositor has been looked at afresh.
    held: Option<Outcome>,
    outcome: Option<Outcome>,
    /// Nobody is listening any more: the thread stops.
    closed: bool,
}

impl Shared {
    fn set_link(&mut self, link: Link) {
        if self.view.link != link {
            self.view.link = link;
            self.view.generation += 1;
        }
    }

    /// Send the queued setup if there is one and the compositor will
    /// take it now. One it refuses outright is over.
    fn send_queued(&mut self) {
        let Some((engine, stream)) = self.live.as_mut() else { return };
        if !engine.ready() {
            return;
        }
        let Some(job) = self.queued.take() else { return };
        let mut out = Vec::new();
        match engine.configure(&job.changes, &mut out).and_then(|()| stream.write_all(&out).map_err(|e| e.to_string())) {
            Ok(()) => self.flying = Some(job),
            Err(why) => {
                log_error!("monitors: not applied: {why}");
                self.outcome = Some(Outcome::Failed);
            }
        }
    }

    /// The connection ended with a setup sent and not answered: it goes
    /// again on the next one, as long as that is worth trying. Asking
    /// twice for the same setup is asking once.
    fn requeue_flying(&mut self) {
        match self.flying.take() {
            Some(job) if job.tries + 1 < MAX_TRIES => self.queued = Some(Job { tries: job.tries + 1, ..job }),
            Some(_) => self.outcome = Some(Outcome::Failed),
            None => {}
        }
    }

    /// The connection ended for good: whatever was on its way didn't
    /// happen.
    fn give_up(&mut self, link: Link) {
        self.live = None;
        if self.queued.take().is_some() | self.flying.take().is_some() {
            self.outcome = Some(Outcome::Failed);
        } else if let Some(held) = self.held.take() {
            // It was answered; only the look afterwards is missing.
            self.outcome = Some(held);
        }
        self.set_link(link);
    }
}

/// Why a connection ended.
enum End {
    /// On purpose: connect again for a fresh look.
    Again,
    Missing,
    Lost(String),
}

/// The monitors, live. Dropping it closes the connection.
pub struct Outputs {
    shared: Arc<Mutex<Shared>>,
}

fn lock(shared: &Mutex<Shared>) -> MutexGuard<'_, Shared> {
    shared.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// The compositor's socket: `WAYLAND_DISPLAY`, in `XDG_RUNTIME_DIR`
/// unless it is a whole path.
fn socket_path() -> Option<PathBuf> {
    let name = PathBuf::from(std::env::var_os("WAYLAND_DISPLAY").unwrap_or_else(|| "wayland-0".into()));
    if name.is_absolute() {
        return Some(name);
    }
    Some(PathBuf::from(std::env::var_os("XDG_RUNTIME_DIR")?).join(name))
}

impl Outputs {
    /// Connect and start listening. `waker` is told whenever there is
    /// something new to show.
    pub fn start(waker: Waker) -> Self {
        let shared = Arc::new(Mutex::new(Shared { view: View { link: Link::Connecting, heads: Vec::new(), generation: 1 }, live: None, queued: None, flying: None, held: None, outcome: None, closed: false }));
        let theirs = Arc::clone(&shared);
        let spawned = std::thread::Builder::new().name("monitors".to_owned()).spawn(move || run(&theirs, &waker));
        if let Err(e) = spawned {
            lock(&shared).give_up(Link::Lost(format!("no thread to listen on: {e}")));
        }
        Self { shared }
    }

    /// What there is to show, when it is newer than `seen`.
    pub fn newer_than(&self, seen: u64) -> Option<View> {
        let s = lock(&self.shared);
        (s.view.generation != seen).then(|| s.view.clone())
    }


    /// Ask for the monitors to be as `changes` says; [`Self::take_outcome`]
    /// says how it went. Refused at once when nothing could come of it.
    pub fn apply(&self, changes: Vec<Change>) -> Result<(), String> {
        let mut s = lock(&self.shared);
        match &s.view.link {
            Link::Missing => return Err("the compositor has no output manager".to_owned()),
            Link::Lost(why) => return Err(why.clone()),
            Link::Connecting | Link::Ready => {}
        }
        if s.queued.is_some() || s.flying.is_some() || s.held.is_some() {
            return Err("the last change is still being made".to_owned());
        }
        let again = changes.iter().any(|c| c.enabled && s.view.heads.iter().any(|h| h.name == c.name && !h.enabled));
        log_info!("monitors: applying {changes:?}");
        s.outcome = None;
        s.queued = Some(Job { changes, tries: 0, again });
        s.send_queued();
        Ok(())
    }

    /// How the last [`Self::apply`] ended, once.
    pub fn take_outcome(&self) -> Option<Outcome> {
        lock(&self.shared).outcome.take()
    }
}

impl Drop for Outputs {
    fn drop(&mut self) {
        let mut s = lock(&self.shared);
        s.closed = true;
        // The thread's read ends with this.
        if let Some((_, stream)) = &s.live {
            let _ = stream.shutdown(Shutdown::Both);
        }
    }
}

/// The thread: one connection after another until one ends for good.
fn run(shared: &Mutex<Shared>, waker: &Waker) {
    let mut false_starts = 0;
    loop {
        let mut settled = false;
        let end = session(shared, waker, &mut settled);
        let mut s = lock(shared);
        s.live = None;
        false_starts = if settled { 0 } else { false_starts + 1 };
        let link = match end {
            _ if s.closed => return,
            End::Again if false_starts < MAX_FALSE_STARTS => {
                s.requeue_flying();
                continue;
            }
            End::Again => Link::Lost("the compositor kept changing its monitors".to_owned()),
            End::Missing => Link::Missing,
            End::Lost(why) => Link::Lost(why),
        };
        match &link {
            Link::Lost(why) => log_error!("monitors: {why}"),
            _ => log_info!("monitors: the compositor has no output manager"),
        }
        s.give_up(link);
        drop(s);
        waker.wake();
        return;
    }
}

/// One connection, from hello to its end. `settled` is set once the
/// compositor has said how things are.
fn session(shared: &Mutex<Shared>, waker: &Waker, settled: &mut bool) -> End {
    let Some(path) = socket_path() else { return End::Lost("XDG_RUNTIME_DIR is not set".to_owned()) };
    let mut stream = match UnixStream::connect(&path) {
        Ok(s) => s,
        Err(e) => return End::Lost(format!("{}: {e}", path.display())),
    };
    let mut hello = Vec::new();
    let engine = Engine::new(&mut hello);
    let writer = match stream.try_clone() {
        Ok(w) => w,
        Err(e) => return End::Lost(e.to_string()),
    };
    if let Err(e) = stream.write_all(&hello) {
        return End::Lost(e.to_string());
    }
    lock(shared).live = Some((engine, writer));

    let mut reader = Reader::default();
    let mut buf = [0u8; 8192];
    loop {
        let n = match stream.read(&mut buf) {
            Ok(0) => return End::Lost("the compositor hung up".to_owned()),
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return End::Lost(e.to_string()),
        };
        reader.push(&buf[..n]);
        let mut guard = lock(shared);
        let s = &mut *guard;
        if s.closed {
            return End::Again;
        }
        let (mut out, mut end, mut wake) = (Vec::new(), None, false);
        while end.is_none() {
            let message = match reader.next() {
                Ok(Some(m)) => m,
                Ok(None) => break,
                Err(why) => return End::Lost(why),
            };
            let Some((engine, _)) = s.live.as_mut() else { return End::Again };
            match engine.event(&message, &mut out) {
                Note::Nothing => {}
                Note::Settled => {
                    *settled = true;
                    s.view.heads = engine.heads();
                    s.view.link = Link::Ready;
                    s.view.generation += 1;
                    // Unless the setup goes round again, this is the look
                    // its answer was waiting for.
                    if s.queued.is_none()
                        && let Some(held) = s.held.take()
                    {
                        s.outcome = Some(held);
                    }
                    wake = true;
                }
                Note::Outcome(outcome) => {
                    let job = s.flying.take();
                    log_info!("monitors: {outcome:?}");
                    match (outcome, job) {
                        (Outcome::Succeeded, Some(job)) if job.again => s.queued = Some(Job { again: false, tries: 0, ..job }),
                        (Outcome::Cancelled, Some(job)) if job.tries + 1 < MAX_TRIES => s.queued = Some(Job { tries: job.tries + 1, ..job }),
                        _ => s.held = Some(outcome),
                    }
                    end = Some(End::Again);
                }
                Note::Stale => end = Some(End::Again),
                Note::Missing => end = Some(End::Missing),
                Note::Broken(why) => end = Some(End::Lost(why)),
            }
        }
        if let Some(end) = end {
            drop(guard);
            if wake {
                waker.wake();
            }
            return end;
        }
        if !out.is_empty()
            && let Err(e) = stream.write_all(&out)
        {
            return End::Lost(e.to_string());
        }
        s.send_queued();
        wake |= s.outcome.is_some();
        drop(guard);
        if wake {
            waker.wake();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Talk to the compositor this runs under until `stop` says a note
    /// is the one waited for.
    fn listen(stream: &mut UnixStream, engine: &mut Engine, reader: &mut Reader, stop: impl Fn(&Note) -> bool) -> Note {
        let mut buf = [0u8; 8192];
        loop {
            let mut out = Vec::new();
            while let Some(m) = reader.next().unwrap() {
                match engine.event(&m, &mut out) {
                    Note::Nothing => {}
                    note if stop(&note) => {
                        stream.write_all(&out).unwrap();
                        return note;
                    }
                    Note::Settled => {}
                    other => panic!("{other:?}"),
                }
            }
            stream.write_all(&out).unwrap();
            let n = stream.read(&mut buf).unwrap();
            assert!(n > 0, "the compositor hung up");
            reader.push(&buf[..n]);
        }
    }

    /// Ask the compositor this runs under what monitors it has, show
    /// them, and rehearse a whole setup with it: every monitor named
    /// with the mode, place and scale it already has, sent as a `test`,
    /// which the compositor answers and does nothing about. Nothing on
    /// screen changes. Run with `--ignored --nocapture` inside a Lantern
    /// session.
    #[test]
    #[ignore]
    fn the_compositor_here_says_what_monitors_it_has() {
        let mut stream = UnixStream::connect(socket_path().expect("a runtime dir")).expect("a compositor");
        let mut out = Vec::new();
        let mut engine = Engine::new(&mut out);
        stream.write_all(&out).unwrap();
        let mut reader = Reader::default();
        listen(&mut stream, &mut engine, &mut reader, |n| *n == Note::Settled);
        let heads = engine.heads();
        for h in &heads {
            let mode = h.current.map(|i| h.modes[i]);
            println!("{} on={} at {:?} scale {} {:?} mm, {} modes, now {mode:?}", h.name, h.enabled, h.position, h.scale, h.physical_mm, h.modes.len());
        }
        assert!(!heads.is_empty() && engine.ready());

        let same: Vec<Change> = heads.iter().map(|h| Change { name: h.name.clone(), enabled: h.enabled, mode: h.current, position: h.position, scale: Some(h.scale) }).collect();
        let mut out = Vec::new();
        engine.rehearse(&same, &mut out).unwrap();
        stream.write_all(&out).unwrap();
        let answer = listen(&mut stream, &mut engine, &mut reader, |n| matches!(n, Note::Outcome(_)));
        println!("rehearsed {same:?}: {answer:?}");
        assert_eq!(answer, Note::Outcome(Outcome::Succeeded));
    }
}

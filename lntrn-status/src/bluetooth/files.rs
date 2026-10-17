//! Files over Bluetooth, through obexd (`org.bluez.obex`) on the session
//! bus; sending is `send`'s. Receiving: an agent on the files thread's own connection, which
//! obexd asks before a device's file comes in (`AuthorizePush`), and the
//! person says yes or no ([`Offer`] on the board). A yes names the file
//! for obexd, which writes it in its own folder; once whole it moves to
//! Downloads, and a notification says so. Sending: a session per file, on
//! a connection and a thread of its own, which obexd ties the transfer to
//! (it ends with the connection), and a notification says how it went.
//! Each follows its transfer's properties for how far it got, and either
//! can be stopped: a send by its own thread (obexd lets only a session's
//! owner stop it), a file coming in by the agent's. A name a device sends is only ever a name: no
//! folders, nothing hidden.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use lntrn_dbus::{BodyReader, Message, Value, encode_string};

use super::{BluetoothState, Ended, Offer, Transfer};
use crate::board::Board;
use crate::bus::{Answer, Bus, CallError};
use crate::sys::{self, Bell};

pub(super) const OBEX: &str = "org.bluez.obex";
pub(super) const ROOT: &str = "/org/bluez/obex";
const AGENT_MANAGER: &str = "org.bluez.obex.AgentManager1";
const AGENT: &str = "org.bluez.obex.Agent1";
pub(super) const TRANSFER: &str = "org.bluez.obex.Transfer1";
const SESSION: &str = "org.bluez.obex.Session1";
const PROPERTIES: &str = "org.freedesktop.DBus.Properties";
const AGENT_PATH: &str = "/org/lantern/status/obex_agent";
const REJECTED: &str = "org.bluez.obex.Error.Rejected";
pub(super) const DENIED: &str = "org.freedesktop.DBus.Error.AccessDenied";
/// Between tries at obexd while it isn't there.
const RECONNECT: Duration = Duration::from_secs(15);
/// How often a transfer is read, besides what its signals say.
pub(super) const POLL: Duration = Duration::from_secs(1);
/// Ended transfers kept on the board, at most.
const KEPT: usize = 6;
/// Names for sends obexd hasn't named yet, and for offers.
pub(super) static NEXT: AtomicU64 = AtomicU64::new(1);


enum FileCmd {
    /// The person's answer to the offer `id`.
    Take(u64, bool),
    /// Stop the file coming in at this path.
    Stop(String),
}

pub(super) struct Files {
    tx: Sender<FileCmd>,
    bell: Arc<Bell>,
}

impl Files {
    /// Be obexd's agent from a thread of its own, for files coming in.
    pub(super) fn start(board: Arc<Board<BluetoothState>>) -> Option<Files> {
        let bell = Bell::new().map(Arc::new).map_err(|e| eprintln!("[lntrn-status] bluetooth files: no bell ({e}); files aren't taken")).ok()?;
        let (tx, rx) = channel();
        let theirs = bell.clone();
        let spawned = std::thread::Builder::new().name("bluetooth-files".into()).spawn(move || run(&board, &rx, &theirs));
        if let Err(e) = spawned {
            eprintln!("[lntrn-status] bluetooth files: no thread: {e}; files aren't taken");
            return None;
        }
        Some(Files { tx, bell })
    }

    /// Take the file offered as `id`, or turn it down.
    pub(super) fn take(&self, id: u64, yes: bool) {
        self.send(FileCmd::Take(id, yes));
    }

    /// Stop the file coming in at `path`.
    pub(super) fn stop(&self, path: &str) {
        self.send(FileCmd::Stop(path.to_owned()));
    }

    fn send(&self, cmd: FileCmd) {
        if self.tx.send(cmd).is_ok() {
            self.bell.ring();
        }
    }
}

/// An offer waiting on the person: whose call to answer, and the
/// transfer it would be.
struct Waiting {
    id: u64,
    serial: u32,
    sender: String,
    transfer: Transfer,
}

/// A transfer coming in, followed until it's whole.
struct Coming {
    path: String,
    name: String,
    /// Where obexd writes it, once it says.
    file: Option<String>,
}

fn run(board: &Arc<Board<BluetoothState>>, rx: &Receiver<FileCmd>, bell: &Bell) {
    let mut bus: Option<(Bus, String)> = None;
    let mut retry = Instant::now();
    let mut waiting: Option<Waiting> = None;
    let mut coming: Vec<Coming> = Vec::new();
    loop {
        if bus.is_none() && Instant::now() >= retry {
            retry = Instant::now() + RECONNECT;
            bus = connect().ok();
        }
        let wake = if !coming.is_empty() { POLL } else { retry.saturating_duration_since(Instant::now()).max(POLL) };
        let mut fds = vec![bell.fd()];
        fds.extend(bus.as_ref().map(|(b, _)| b.fd()));
        let ready = sys::wait(&fds, Some(wake));
        bell.quiet();

        for cmd in rx.try_iter() {
            let (id, yes) = match cmd {
                FileCmd::Take(id, yes) => (id, yes),
                FileCmd::Stop(path) => {
                    if let (Some((b, _)), Some(c)) = (bus.as_mut(), coming.iter().find(|c| c.path == path)) {
                        let _ = b.call(OBEX, &path, TRANSFER, "Cancel", "", &[]);
                        // What came of it so far isn't a file anyone wants.
                        let _ = std::fs::remove_file(c.file.clone().map(PathBuf::from).unwrap_or_else(|| cache().join(&c.name)));
                        end(board, &path, Ended::Stopped);
                    }
                    continue;
                }
            };
            let (Some((b, _)), Some(w)) = (bus.as_mut(), waiting.take_if(|w| w.id == id)) else { continue };
            board.update(|s| s.offer = None);
            if !yes {
                b.answer(w.serial, &w.sender, Answer::Refuse(REJECTED, "Turned down"));
                continue;
            }
            let mut body = Vec::new();
            encode_string(&mut body, &w.transfer.name);
            b.answer(w.serial, &w.sender, Answer::Reply("s", body));
            coming.push(Coming { path: w.transfer.path.clone(), name: w.transfer.name.clone(), file: None });
            board.update(|s| s.transfers.push(w.transfer));
        }
        if ready.get(1).copied().unwrap_or(false)
            && let Some((b, obexd)) = bus.as_mut()
        {
            match b.drain() {
                Ok(messages) => {
                    for msg in messages {
                        if msg.is_method_call() {
                            let answer = agent_call(b, &msg, obexd, board, &mut waiting);
                            if let Some(answer) = answer {
                                b.answer(msg.serial, &msg.sender, answer);
                            }
                        } else if msg.is_signal() && msg.member == "PropertiesChanged" {
                            let changed = BodyReader::new(&msg.body, &msg.signature).read_all();
                            if let Some(c) = coming.iter_mut().find(|c| c.path == msg.path) {
                                follow(board, c, changed.get(1).and_then(Value::as_dict));
                            }
                        }
                    }
                }
                Err(_) => {
                    bus = None;
                    waiting = None;
                    board.update(|s| s.offer = None);
                }
            }
        }
        // Signals can go missing: what each one coming in says now.
        if let Some((b, _)) = bus.as_mut() {
            for c in &mut coming {
                match properties(b, &c.path, TRANSFER) {
                    Ok(props) => follow(board, c, Some(&props)),
                    // Gone: whole, if the file is there.
                    Err(CallError::Refused { .. }) => land(board, c),
                    Err(CallError::Bus(_)) => {}
                }
            }
        }
        coming.retain(|c| board.get().transfers.iter().any(|t| t.path == c.path && t.end.is_none()));
    }
}

/// A connection with our agent on it, and obexd's unique name: obexd is
/// started for it if it isn't running.
fn connect() -> Result<(Bus, String), CallError> {
    let mut bus = Bus::session().map_err(CallError::Bus)?;
    bus.listen(OBEX);
    let mut path = Vec::new();
    encode_string(&mut path, AGENT_PATH);
    bus.call(OBEX, ROOT, AGENT_MANAGER, "RegisterAgent", "o", &path)?;
    let obexd = bus.owner(OBEX)?;
    Ok((bus, obexd))
}

/// Take obexd's call `msg` to our agent: what to say at once, or nothing
/// while an offer waits on the person.
fn agent_call(bus: &mut Bus, msg: &Message, obexd: &str, board: &Board<BluetoothState>, waiting: &mut Option<Waiting>) -> Option<Answer> {
    if msg.sender != obexd || msg.path != AGENT_PATH || msg.interface != AGENT {
        return Some(Answer::Refuse(DENIED, "Not obexd's agent"));
    }
    match msg.member.as_str() {
        "AuthorizePush" => {
            let path = BodyReader::new(&msg.body, &msg.signature).read_all().first().and_then(Value::as_str).unwrap_or_default().to_owned();
            let props = properties(bus, &path, TRANSFER).unwrap_or_default();
            let session = text(&props, "Session").unwrap_or_default();
            let device = properties(bus, &session, SESSION).ok().and_then(|p| text(&p, "Destination")).unwrap_or_default();
            let transfer = Transfer {
                path,
                device: device.clone(),
                name: safe_name(&text(&props, "Name").unwrap_or_default()),
                size: number(&props, "Size"),
                done: 0,
                incoming: true,
                end: None,
            };
            let id = NEXT.fetch_add(1, Ordering::Relaxed);
            board.update(|s| {
                let from = s.devices.iter().chain(&s.nearby).find(|d| d.address.eq_ignore_ascii_case(&device)).map_or_else(|| device.clone(), |d| d.name.clone());
                s.offer = Some(Offer { id, device: device.clone(), from, name: transfer.name.clone(), size: transfer.size });
            });
            // One at a time: an older one still waiting is turned down.
            if let Some(old) = waiting.replace(Waiting { id, serial: msg.serial, sender: msg.sender.clone(), transfer }) {
                bus.answer(old.serial, &old.sender, Answer::Refuse(REJECTED, "Another came"));
            }
            None
        }
        "Cancel" | "Release" => {
            *waiting = None;
            board.update(|s| s.offer = None);
            Some(Answer::Reply("", Vec::new()))
        }
        _ => Some(Answer::Refuse("org.freedesktop.DBus.Error.UnknownMethod", "Not a question this agent takes")),
    }
}

/// What `props` (a transfer's, or what changed of them) say of the one
/// coming in as `c`.
fn follow(board: &Board<BluetoothState>, c: &mut Coming, props: Option<&std::collections::HashMap<String, Value>>) {
    let Some(props) = props else { return };
    if let Some(file) = text(props, "Filename") {
        c.file = Some(file);
    }
    if props.contains_key("Transferred") {
        let done = number(props, "Transferred");
        update(board, &c.path, |t| t.done = done);
    }
    match text(props, "Status").as_deref() {
        Some("complete") => land(board, c),
        Some("error") => end(board, &c.path, Ended::Failed("It stopped coming".into())),
        _ => {}
    }
}

/// The file coming in as `c` is whole (or gone): into Downloads with it,
/// and say so.
fn land(board: &Board<BluetoothState>, c: &Coming) {
    let from = c.file.clone().map(PathBuf::from).unwrap_or_else(|| cache().join(&c.name));
    if !from.is_file() {
        end(board, &c.path, Ended::Failed("It never all came".into()));
        return;
    }
    let Some(dir) = downloads() else { return end(board, &c.path, Ended::Failed("No Downloads folder".into())) };
    let _ = std::fs::create_dir_all(&dir);
    let to = unique_in(&dir, &c.name);
    let moved = std::fs::rename(&from, &to).is_ok() || (std::fs::copy(&from, &to).is_ok() && std::fs::remove_file(&from).is_ok());
    if !moved {
        return end(board, &c.path, Ended::Failed("Couldn't put it in Downloads".into()));
    }
    let shown = to.display().to_string();
    let who = board.get().transfers.iter().find(|t| t.path == c.path).map(|t| t.device.clone()).unwrap_or_default();
    let who = board.get().devices.iter().find(|d| d.address.eq_ignore_ascii_case(&who)).map_or(who, |d| d.name.clone());
    end(board, &c.path, Ended::Saved(shown.clone()));
    tell_saved(&format!("Received {}", c.name), &format!("From {who}, saved in Downloads"), to);
}

/// Every property `iface` has at `path`.
pub(super) fn properties(bus: &mut Bus, path: &str, iface: &str) -> Result<std::collections::HashMap<String, Value>, CallError> {
    let mut body = Vec::new();
    encode_string(&mut body, iface);
    let reply = bus.call(OBEX, path, PROPERTIES, "GetAll", "s", &body)?;
    Ok(reply.first().and_then(Value::as_dict).cloned().unwrap_or_default())
}

pub(super) fn text(props: &std::collections::HashMap<String, Value>, key: &str) -> Option<String> {
    props.get(key).and_then(Value::as_str).map(str::to_owned)
}

pub(super) fn number(props: &std::collections::HashMap<String, Value>, key: &str) -> u64 {
    props.get(key).and_then(Value::as_u64).unwrap_or(0)
}

/// Change the board's transfer at `path`.
pub(super) fn update(board: &Board<BluetoothState>, path: &str, f: impl FnOnce(&mut Transfer)) {
    board.update(|s| {
        if let Some(t) = s.transfers.iter_mut().find(|t| t.path == path) {
            f(t);
        }
    });
}

/// The transfer at `path` ended as `how`; only the last few ended ones
/// are kept.
pub(super) fn end(board: &Board<BluetoothState>, path: &str, how: Ended) {
    board.update(|s| {
        if let Some(t) = s.transfers.iter_mut().find(|t| t.path == path && t.end.is_none()) {
            t.end = Some((how, Instant::now()));
        }
        while s.transfers.iter().filter(|t| t.end.is_some()).count() > KEPT {
            let Some(oldest) = s.transfers.iter().position(|t| t.end.is_some()) else { break };
            s.transfers.remove(oldest);
        }
    });
}

/// What a file a device sends is saved as: the last part of its name,
/// no control characters, nothing hidden (no dot first), at most 200
/// bytes; "File" when nothing is left of it.
fn safe_name(name: &str) -> String {
    let last = name.rsplit(['/', '\\']).next().unwrap_or_default();
    let clean: String = last.chars().filter(|c| !c.is_control()).collect();
    let clean = clean.trim().trim_start_matches('.').trim_start();
    let mut out = String::new();
    for c in clean.chars() {
        if out.len() + c.len_utf8() > 200 {
            break;
        }
        out.push(c);
    }
    if out.is_empty() { "File".to_owned() } else { out }
}

/// `name` in `dir`, or `name (2)`, `name (3)`… when it's taken.
fn unique_in(dir: &Path, name: &str) -> PathBuf {
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 => name.split_at(i),
        _ => (name, ""),
    };
    std::iter::once(dir.join(name)).chain((2..1000).map(|n| dir.join(format!("{stem} ({n}){ext}")))).find(|p| !p.exists()).unwrap_or_else(|| dir.join(name))
}

/// Where obexd writes what comes in: its own folder in the cache.
fn cache() -> PathBuf {
    std::env::var_os("XDG_CACHE_HOME").map(PathBuf::from).or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache"))).unwrap_or_default().join("obexd")
}

fn downloads() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Downloads"))
}

/// A notification that `file` came in, with Open (a click on it, too)
/// and Show in folder; what the person clicks is done from its thread.
fn tell_saved(title: &str, body: &str, file: PathBuf) {
    let (title, body) = (title.to_owned(), body.to_owned());
    let _ = std::thread::Builder::new().name("bluetooth-tell".into()).spawn(move || {
        let actions = ["-A", "default=Open", "-A", "open=Open", "-A", "folder=Show in folder"];
        let Ok(out) = std::process::Command::new("notify-send").args(["-a", "Bluetooth", "-i", "bluetooth"]).args(actions).args([&title, &body]).output() else { return };
        let clicked = String::from_utf8_lossy(&out.stdout);
        let (program, path) = match clicked.lines().next().map(str::trim) {
            Some("default" | "open") => ("xdg-open", file.clone()),
            Some("folder") => ("lntrn-file-manager", file.parent().map(Path::to_path_buf).unwrap_or_default()),
            _ => return,
        };
        let _ = std::process::Command::new(program).arg(path).status();
    });
}

/// A desktop notification, from a thread so nothing waits on it.
pub(super) fn tell(title: &str, body: &str) {
    let (title, body) = (title.to_owned(), body.to_owned());
    let _ = std::thread::Builder::new().name("bluetooth-tell".into()).spawn(move || {
        let _ = std::process::Command::new("notify-send").args(["-a", "Bluetooth", "-i", "bluetooth", &title, &body]).status();
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Against the real obexd: our agent is taken, and let go. Takes no
    /// file.
    #[test]
    #[ignore = "talks to the session bus and obexd"]
    fn obexd_takes_our_agent_and_lets_it_go() {
        let (mut bus, obexd) = connect().unwrap();
        assert!(obexd.starts_with(':'));
        let mut path = Vec::new();
        encode_string(&mut path, AGENT_PATH);
        bus.call(OBEX, ROOT, AGENT_MANAGER, "UnregisterAgent", "o", &path).unwrap();
    }

    #[test]
    fn a_name_a_device_sends_is_only_a_name() {
        assert_eq!(safe_name("photo.jpg"), "photo.jpg");
        assert_eq!(safe_name("../../.bashrc"), "bashrc");
        assert_eq!(safe_name("C:\\Users\\me\\notes.txt"), "notes.txt");
        assert_eq!(safe_name("bad\nname\u{7}.txt"), "badname.txt");
        assert_eq!((safe_name(""), safe_name("..")), ("File".to_owned(), "File".to_owned()));
        assert!(safe_name(&"é".repeat(300)).len() <= 200);
    }

    #[test]
    fn a_name_taken_in_downloads_gets_a_number() {
        let dir = std::env::temp_dir().join(format!("lntrn-status-downloads-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(unique_in(&dir, "photo.jpg"), dir.join("photo.jpg"));
        std::fs::write(dir.join("photo.jpg"), b"x").unwrap();
        std::fs::write(dir.join("photo (2).jpg"), b"x").unwrap();
        assert_eq!(unique_in(&dir, "photo.jpg"), dir.join("photo (3).jpg"));
        std::fs::write(dir.join("README"), b"x").unwrap();
        assert_eq!(unique_in(&dir, "README"), dir.join("README (2)"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}

//! The socket: who may open it, and a thread for each window on it.
//!
//! The socket file is `root:wheel 0660`, so the kernel itself turns
//! everyone else away; each connection's user is then checked again
//! against the group, from the kernel's own word on who they are. A line
//! is read whole (and no longer than [`MAX_LINE`]), answered whole.

use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use lntrn_afterglowd::wire::{self, Hello, MAX_LINE, Request};
use lntrn_afterglowd::{PROTOCOL, TRIAL_SECONDS};
use lntrn_data::Doc;

use super::ffi;
use super::log::say;
use super::state::Daemon;

/// The most windows that may be connected at once.
const MAX_CLIENTS: usize = 16;

/// Who may use the helper.
pub struct Door {
    /// The user the helper runs as: root, or a user testing it.
    pub own: u32,
    /// The group whose members may.
    pub group: Option<String>,
}

/// A group's id and members, from `/etc/group`.
pub fn group(name: &str) -> Option<(u32, Vec<String>)> {
    let text = std::fs::read_to_string("/etc/group").ok()?;
    text.lines().find_map(|line| {
        let mut f = line.split(':');
        (f.next()? == name).then_some(())?;
        let gid = f.nth(1)?.parse().ok()?;
        let members = f.next().unwrap_or("").split(',').filter(|m| !m.is_empty()).map(str::to_owned).collect();
        Some((gid, members))
    })
}

/// A user's name and main group, from `/etc/passwd`.
fn user(uid: u32) -> Option<(String, u32)> {
    let text = std::fs::read_to_string("/etc/passwd").ok()?;
    text.lines().find_map(|line| {
        let f: Vec<&str> = line.split(':').collect();
        if f.len() < 4 || f[2].parse() != Ok(uid) {
            return None;
        }
        Some((f[0].to_owned(), f[3].parse().ok()?))
    })
}

impl Door {
    pub fn admits(&self, uid: u32) -> bool {
        if uid == 0 || uid == self.own {
            return true;
        }
        let Some(name) = &self.group else { return false };
        // Read fresh each time: someone taken out of the group is out.
        let (Some((gid, members)), Some((user, primary))) = (group(name), user(uid)) else { return false };
        primary == gid || members.contains(&user)
    }
}

pub fn serve(listener: UnixListener, daemon: Arc<Mutex<Daemon>>, door: Door) {
    let door = Arc::new(door);
    let open = Arc::new(AtomicUsize::new(0));
    for stream in listener.incoming() {
        let Ok(stream) = stream else { continue };
        let Some(uid) = ffi::peer_uid(&stream) else { continue };
        if !door.admits(uid) {
            say(&format!("turned away user {uid}: not in the group"));
            continue;
        }
        if open.load(Ordering::SeqCst) >= MAX_CLIENTS {
            say("turned away a window: too many open");
            continue;
        }
        open.fetch_add(1, Ordering::SeqCst);
        let (daemon, open) = (daemon.clone(), open.clone());
        let spawned = std::thread::Builder::new().name(format!("client-{uid}")).spawn(move || {
            talk(stream, &daemon, uid);
            open.fetch_sub(1, Ordering::SeqCst);
        });
        if let Err(e) = spawned {
            say(&format!("couldn't start a thread for a window: {e}"));
        }
    }
}

/// Answer one window's requests until it hangs up.
fn talk(stream: UnixStream, daemon: &Mutex<Daemon>, uid: u32) {
    // A window that goes quiet mid-line doesn't hold its thread forever.
    let _ = stream.set_write_timeout(Some(Duration::from_secs(10)));
    let Ok(mut writer) = stream.try_clone() else { return };
    let mut reader = BufReader::new(stream);
    loop {
        let mut line = String::new();
        match (&mut reader).take(MAX_LINE as u64 + 1).read_line(&mut line) {
            Ok(0) | Err(_) => return,
            Ok(_) => {}
        }
        let answer = if !line.ends_with('\n') {
            // Longer than any request: nothing more is read from it.
            let _ = writeln!(writer, "{}", wire::fail("that request is too long"));
            return;
        } else {
            match Request::parse(line.trim_end()) {
                Ok(request) => respond(request, daemon, uid),
                Err(e) => wire::fail(&e),
            }
        };
        if writeln!(writer, "{answer}").is_err() {
            return;
        }
    }
}

fn problems(list: Vec<String>) -> Vec<(&'static str, Doc)> {
    vec![("problems", Doc::List(list.into_iter().map(Doc::Str).collect()))]
}

fn respond(request: Request, daemon: &Mutex<Daemon>, uid: u32) -> String {
    let mut d = daemon.lock().unwrap_or_else(|e| e.into_inner());
    match request {
        Request::Hello => wire::ok(Hello { protocol: PROTOCOL, root: ffi::euid() == 0, dry_run: d.dry_run, limits: d.limits.clone() }.entries()),
        Request::Status => wire::ok(d.status().entries()),
        Request::Apply { tune, trial } => {
            say(&format!("user {uid} applies a tune{}", if trial { " on trial" } else { "" }));
            match d.apply(tune, trial.then(|| Duration::from_secs(TRIAL_SECONDS))) {
                Ok(list) => wire::ok(problems(list)),
                Err(e) => {
                    say(&format!("refused: {e}"));
                    wire::fail(&e)
                }
            }
        }
        Request::Keep => {
            d.keep();
            wire::ok(Vec::new())
        }
        Request::Revert => {
            say(&format!("user {uid} goes back"));
            wire::ok(problems(d.revert()))
        }
        Request::Reset => {
            say(&format!("user {uid} resets to stock"));
            wire::ok(problems(d.reset()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_and_the_helpers_own_user_are_always_let_in() {
        let door = Door { own: 4242, group: None };
        assert!(door.admits(0));
        assert!(door.admits(4242));
        assert!(!door.admits(1000), "no group, no one else");
        // A group no one is in lets no one else in.
        let door = Door { own: 0, group: Some("no-such-group-here".into()) };
        assert!(!door.admits(1000));
    }
}

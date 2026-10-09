//! wlr-output-management-unstable-v1, spoken by hand: what arrives is
//! taken in a message at a time and what we know of the monitors builds
//! up; a request for a new setup is written out as the protocol wants it.
//! Nothing here touches a socket, so all of it is tested against bytes.
//!
//! The objects, with the opcodes used:
//! - `wl_display` (always id 1): we ask for the registry and a `sync`.
//! - `wl_registry`: its globals; we bind the output manager.
//! - `zwlr_output_manager_v1`: a head per monitor, then `done` with the
//!   serial a configuration must quote.
//! - `zwlr_output_head_v1`, `zwlr_output_mode_v1`: what a monitor is and
//!   the modes it can run.
//! - `zwlr_output_configuration_v1` and its heads: a setup to apply, and
//!   whether it took.

use std::collections::HashMap;

use super::wire::{Args, Message, Request};
use super::{Change, Head, Mode, Outcome};

const DISPLAY: u32 = 1;
/// The newest version of the manager this was written against.
const MANAGER_VERSION: u32 = 4;
/// From this version a head or a mode that is finished is released.
const RELEASE_SINCE: u32 = 3;

mod op {
    pub const DISPLAY_SYNC: u16 = 0;
    pub const DISPLAY_GET_REGISTRY: u16 = 1;
    pub const DISPLAY_ERROR: u16 = 0;
    pub const DISPLAY_DELETE_ID: u16 = 1;
    pub const REGISTRY_BIND: u16 = 0;
    pub const REGISTRY_GLOBAL: u16 = 0;
    pub const MANAGER_CREATE_CONFIGURATION: u16 = 0;
    pub const MANAGER_HEAD: u16 = 0;
    pub const MANAGER_DONE: u16 = 1;
    pub const MANAGER_FINISHED: u16 = 2;
    pub const HEAD_NAME: u16 = 0;
    pub const HEAD_PHYSICAL_SIZE: u16 = 2;
    pub const HEAD_MODE: u16 = 3;
    pub const HEAD_ENABLED: u16 = 4;
    pub const HEAD_CURRENT_MODE: u16 = 5;
    pub const HEAD_POSITION: u16 = 6;
    pub const HEAD_SCALE: u16 = 8;
    pub const HEAD_FINISHED: u16 = 9;
    pub const HEAD_RELEASE: u16 = 0;
    pub const MODE_SIZE: u16 = 0;
    pub const MODE_REFRESH: u16 = 1;
    pub const MODE_PREFERRED: u16 = 2;
    pub const MODE_FINISHED: u16 = 3;
    pub const MODE_RELEASE: u16 = 0;
    pub const CONFIG_ENABLE_HEAD: u16 = 0;
    pub const CONFIG_DISABLE_HEAD: u16 = 1;
    pub const CONFIG_APPLY: u16 = 2;
    #[cfg(test)]
    pub const CONFIG_TEST: u16 = 3;
    pub const CONFIG_DESTROY: u16 = 4;
    pub const CONFIG_SUCCEEDED: u16 = 0;
    pub const CONFIG_FAILED: u16 = 1;
    pub const CONFIG_CANCELLED: u16 = 2;
    pub const CONFIG_HEAD_SET_MODE: u16 = 0;
    pub const CONFIG_HEAD_SET_POSITION: u16 = 2;
    pub const CONFIG_HEAD_SET_SCALE: u16 = 4;
}

/// What an id on the wire stands for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Registry,
    Sync,
    Manager,
    Head,
    /// A mode, and the head it is one of.
    Mode(u32),
    Config,
    ConfigHead,
}

/// A monitor as it is being told to us, with the ids the compositor
/// knows its parts by.
struct Known {
    id: u32,
    head: Head,
    /// The id of each of `head.modes`, in step with them.
    mode_ids: Vec<u32>,
    current: Option<u32>,
}

/// What a message meant for whoever is running the connection.
#[derive(Debug, PartialEq)]
pub enum Note {
    Nothing,
    /// The compositor finished saying how things are: [`Engine::heads`]
    /// is whole, and a configuration can be sent.
    Settled,
    /// The configuration we sent took, or didn't.
    Outcome(Outcome),
    /// A monitor went away. The compositor numbers its heads by where
    /// they stand in its list, so every head we hold may now name
    /// another: start again on a fresh connection.
    Stale,
    /// The compositor has no output manager.
    Missing,
    /// The connection is no good any more, and why.
    Broken(String),
}

pub struct Engine {
    next_id: u32,
    objects: HashMap<u32, Kind>,
    registry: u32,
    /// The manager's id and the version we bound it at.
    manager: Option<(u32, u32)>,
    known: Vec<Known>,
    /// The serial of the compositor's last `done`.
    serial: Option<u32>,
    /// The configuration that is being decided.
    pending: Option<u32>,
}

impl Engine {
    /// A new connection's state, and the first thing to say on it: the
    /// registry, then a `sync` whose answer comes after every global has
    /// been named.
    pub fn new(out: &mut Vec<u8>) -> Self {
        let mut e = Self { next_id: DISPLAY + 1, objects: HashMap::new(), registry: 0, manager: None, known: Vec::new(), serial: None, pending: None };
        e.registry = e.alloc(Kind::Registry);
        Request::new(DISPLAY, op::DISPLAY_GET_REGISTRY).uint(e.registry).finish(out);
        let sync = e.alloc(Kind::Sync);
        Request::new(DISPLAY, op::DISPLAY_SYNC).uint(sync).finish(out);
        e
    }

    /// The next id of ours. They go up by one and are never used twice:
    /// the compositor takes a new id only if it is the next one along.
    fn alloc(&mut self, kind: Kind) -> u32 {
        let id = self.next_id;
        self.next_id += 1;
        self.objects.insert(id, kind);
        id
    }

    /// Whether a configuration can be sent now.
    pub fn ready(&self) -> bool {
        self.manager.is_some() && self.serial.is_some() && self.pending.is_none()
    }

    /// The monitors as last told, in the order of their names: the order
    /// they are numbered in.
    pub fn heads(&self) -> Vec<Head> {
        let mut heads: Vec<Head> = self
            .known
            .iter()
            .filter(|k| !k.head.name.is_empty())
            .map(|k| {
                let mut head = k.head.clone();
                head.current = k.current.and_then(|id| k.mode_ids.iter().position(|m| *m == id));
                head
            })
            .collect();
        heads.sort_by(|a, b| a.name.cmp(&b.name));
        heads
    }

    fn known_mut(&mut self, id: u32) -> Option<&mut Known> {
        self.known.iter_mut().find(|k| k.id == id)
    }

    /// Take in one message. What must be said back goes on `out`.
    pub fn event(&mut self, m: &Message, out: &mut Vec<u8>) -> Note {
        let mut a = Args::new(&m.body);
        if m.object == DISPLAY {
            return match m.opcode {
                op::DISPLAY_ERROR => {
                    let (object, code, text) = (a.uint().unwrap_or(0), a.uint().unwrap_or(0), a.string().unwrap_or_default());
                    Note::Broken(format!("the compositor refused object {object} (code {code}): {text}"))
                }
                op::DISPLAY_DELETE_ID => {
                    if let Some(id) = a.uint() {
                        self.objects.remove(&id);
                    }
                    Note::Nothing
                }
                _ => Note::Nothing,
            };
        }
        // Something for an object we let go of: said before the
        // compositor heard.
        let Some(kind) = self.objects.get(&m.object).copied() else { return Note::Nothing };
        match (kind, m.opcode) {
            (Kind::Registry, op::REGISTRY_GLOBAL) => {
                let (Some(name), Some(interface), Some(version)) = (a.uint(), a.string(), a.uint()) else { return Note::Nothing };
                if interface == "zwlr_output_manager_v1" && self.manager.is_none() {
                    let version = version.min(MANAGER_VERSION);
                    let id = self.alloc(Kind::Manager);
                    self.manager = Some((id, version));
                    Request::new(self.registry, op::REGISTRY_BIND).uint(name).string(&interface).uint(version).uint(id).finish(out);
                }
                Note::Nothing
            }
            // Every global has been named by now.
            (Kind::Sync, _) if self.manager.is_none() => Note::Missing,
            (Kind::Manager, op::MANAGER_HEAD) => {
                if let Some(id) = a.uint() {
                    self.objects.insert(id, Kind::Head);
                    self.known.push(Known { id, head: Head::default(), mode_ids: Vec::new(), current: None });
                }
                Note::Nothing
            }
            (Kind::Manager, op::MANAGER_DONE) => {
                self.serial = a.uint();
                Note::Settled
            }
            (Kind::Manager, op::MANAGER_FINISHED) => Note::Broken("the compositor stopped its output manager".to_owned()),
            (Kind::Head, opcode) => self.head_event(m.object, opcode, &mut a, out),
            (Kind::Mode(head), opcode) => {
                self.mode_event(head, m.object, opcode, &mut a, out);
                Note::Nothing
            }
            (Kind::Config, opcode) if self.pending == Some(m.object) => {
                let outcome = match opcode {
                    op::CONFIG_SUCCEEDED => Outcome::Succeeded,
                    op::CONFIG_FAILED => Outcome::Failed,
                    op::CONFIG_CANCELLED => Outcome::Cancelled,
                    _ => return Note::Nothing,
                };
                self.pending = None;
                Request::new(m.object, op::CONFIG_DESTROY).finish(out);
                Note::Outcome(outcome)
            }
            _ => Note::Nothing,
        }
    }

    fn head_event(&mut self, id: u32, opcode: u16, a: &mut Args, out: &mut Vec<u8>) -> Note {
        if opcode == op::HEAD_FINISHED {
            self.known.retain(|k| k.id != id);
            self.objects.remove(&id);
            if self.manager.is_some_and(|(_, v)| v >= RELEASE_SINCE) {
                Request::new(id, op::HEAD_RELEASE).finish(out);
            }
            return Note::Stale;
        }
        if opcode == op::HEAD_MODE {
            if let Some(mode) = a.uint() {
                self.objects.insert(mode, Kind::Mode(id));
                if let Some(k) = self.known_mut(id) {
                    k.mode_ids.push(mode);
                    k.head.modes.push(Mode::default());
                }
            }
            return Note::Nothing;
        }
        let Some(k) = self.known_mut(id) else { return Note::Nothing };
        match opcode {
            op::HEAD_NAME => k.head.name = a.string().unwrap_or_default(),
            op::HEAD_PHYSICAL_SIZE => k.head.physical_mm = (a.int().unwrap_or(0), a.int().unwrap_or(0)),
            op::HEAD_ENABLED => k.head.enabled = a.int().unwrap_or(0) != 0,
            op::HEAD_CURRENT_MODE => k.current = a.uint(),
            op::HEAD_POSITION => k.head.position = (a.int().unwrap_or(0), a.int().unwrap_or(0)),
            op::HEAD_SCALE => k.head.scale = a.fixed().filter(|s| *s > 0.0).unwrap_or(1.0),
            // Its description, make, model, serial number and transform:
            // nothing here shows them.
            _ => {}
        }
        Note::Nothing
    }

    fn mode_event(&mut self, head: u32, id: u32, opcode: u16, a: &mut Args, out: &mut Vec<u8>) {
        let Some(k) = self.known_mut(head) else { return };
        let Some(i) = k.mode_ids.iter().position(|m| *m == id) else { return };
        match opcode {
            op::MODE_SIZE => (k.head.modes[i].width, k.head.modes[i].height) = (a.int().unwrap_or(0), a.int().unwrap_or(0)),
            op::MODE_REFRESH => k.head.modes[i].refresh = a.int().unwrap_or(0),
            op::MODE_PREFERRED => k.head.modes[i].preferred = true,
            op::MODE_FINISHED => {
                k.mode_ids.remove(i);
                k.head.modes.remove(i);
                if k.current == Some(id) {
                    k.current = None;
                }
                self.objects.remove(&id);
                if self.manager.is_some_and(|(_, v)| v >= RELEASE_SINCE) {
                    Request::new(id, op::MODE_RELEASE).finish(out);
                }
            }
            _ => {}
        }
    }

    /// Write out a configuration that makes the monitors as `changes`
    /// says. The protocol wants every head named, on or off, so one
    /// `changes` leaves out is asked to stay as it is. Refused, with the
    /// reason, when it can't be sent or would leave nothing on.
    pub fn configure(&mut self, changes: &[Change], out: &mut Vec<u8>) -> Result<(), String> {
        self.write(changes, out, op::CONFIG_APPLY)
    }

    /// The same configuration, only asked about: the compositor answers
    /// whether it would take it and changes nothing. For trying the wire
    /// against a live compositor without touching its monitors.
    #[cfg(test)]
    pub fn rehearse(&mut self, changes: &[Change], out: &mut Vec<u8>) -> Result<(), String> {
        self.write(changes, out, op::CONFIG_TEST)
    }

    /// A configuration, ended with `verb`: apply it, or only test it.
    fn write(&mut self, changes: &[Change], out: &mut Vec<u8>, verb: u16) -> Result<(), String> {
        let (Some((manager, _)), Some(serial)) = (self.manager, self.serial) else { return Err("the compositor has not said what monitors there are yet".to_owned()) };
        if self.pending.is_some() {
            return Err("the last change is still being made".to_owned());
        }
        for c in changes {
            let Some(k) = self.known.iter().find(|k| k.head.name == c.name) else { return Err(format!("there is no monitor called {}", c.name)) };
            if c.mode.is_some_and(|m| m >= k.mode_ids.len()) {
                return Err(format!("{} has no such mode", c.name));
            }
            if c.scale.is_some_and(|s| !s.is_finite() || s <= 0.0) {
                return Err(format!("{} can't be scaled by that", c.name));
            }
        }
        let wanted = |k: &Known| changes.iter().find(|c| c.name == k.head.name);
        if !self.known.iter().any(|k| wanted(k).map_or(k.head.enabled, |c| c.enabled)) {
            return Err("that would switch every monitor off".to_owned());
        }

        let config = self.alloc(Kind::Config);
        Request::new(manager, op::MANAGER_CREATE_CONFIGURATION).uint(config).uint(serial).finish(out);
        // The ids are handed out in the order they are written.
        let rows: Vec<(u32, bool, Option<&Change>, Option<u32>)> = self.known.iter().map(|k| {
            let c = wanted(k);
            (k.id, c.map_or(k.head.enabled, |c| c.enabled), c, c.and_then(|c| c.mode).map(|m| k.mode_ids[m]))
        }).collect();
        for (head, enabled, change, mode) in rows {
            if !enabled {
                Request::new(config, op::CONFIG_DISABLE_HEAD).uint(head).finish(out);
                continue;
            }
            let row = self.alloc(Kind::ConfigHead);
            Request::new(config, op::CONFIG_ENABLE_HEAD).uint(row).uint(head).finish(out);
            let Some(c) = change else { continue };
            if let Some(mode) = mode {
                Request::new(row, op::CONFIG_HEAD_SET_MODE).uint(mode).finish(out);
            }
            Request::new(row, op::CONFIG_HEAD_SET_POSITION).int(c.position.0).int(c.position.1).finish(out);
            if let Some(scale) = c.scale {
                Request::new(row, op::CONFIG_HEAD_SET_SCALE).fixed(scale).finish(out);
            }
        }
        Request::new(config, verb).finish(out);
        self.pending = Some(config);
        Ok(())
    }
}

#[cfg(test)]
#[path = "engine_tests.rs"]
mod tests;

//! The buses as the D-Bus subsystems use them, over `lntrn-dbus`: calls
//! that wait for their reply (and calls that answer the daemon's
//! questions while they wait, as an agent does), answers to calls made to
//! us, an object manager's objects as maps, properties set, and the
//! signals of one daemon heard as they come. On the system bus a call
//! never starts a daemon that isn't running; on the session bus it may
//! (obexd starts when it's first asked for).
//!
//! A message is only ever read whole: the thread waits for the socket to
//! have something, then reads one message blocking (a non-blocking read
//! can stop halfway through one and lose its start).

use std::collections::HashMap;
use std::io;
use std::os::fd::RawFd;
use std::time::{Duration, Instant};

use lntrn_dbus::{BodyReader, Connection, FLAG_NO_AUTO_START, Message, Value, encode_string, encode_variant_bool};

use crate::secret::wipe_bytes;
use crate::sys;

/// A D-Bus object's properties, by interface: what an object manager
/// says of one object.
pub type Props = HashMap<String, Value>;
/// Every object an object manager has: path, interface, properties.
pub type Objects = HashMap<String, HashMap<String, Props>>;

const DBUS: &str = "org.freedesktop.DBus";
const PROPERTIES: &str = "org.freedesktop.DBus.Properties";
const OBJECT_MANAGER: &str = "org.freedesktop.DBus.ObjectManager";

/// Why a call came to nothing.
#[derive(Debug)]
pub enum CallError {
    /// The connection is gone, or the daemon never answered: open another.
    Bus(io::Error),
    /// The daemon answered no: its error's name
    /// (`net.connman.iwd.Failed`), and why in words.
    Refused { name: String, why: String },
}

impl std::fmt::Display for CallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CallError::Bus(e) => write!(f, "{e}"),
            CallError::Refused { why, .. } => f.write_str(why),
        }
    }
}

/// What we say to a call made to us.
#[cfg_attr(test, derive(Debug, PartialEq))]
pub enum Answer {
    /// Values, `sig` saying what they are.
    Reply(&'static str, Vec<u8>),
    /// No: an error's name, and why.
    Refuse(&'static str, &'static str),
}

pub struct Bus {
    conn: Connection,
    /// What every call is flagged with.
    flags: u8,
}

/// A reply's values, or the error it is.
fn values(reply: &Message) -> Result<Vec<Value>, CallError> {
    let values = BodyReader::new(&reply.body, &reply.signature).read_all();
    if reply.is_error() {
        let why = values.first().and_then(Value::as_str).unwrap_or("refused").to_owned();
        return Err(CallError::Refused { name: reply.error_name.clone(), why });
    }
    Ok(values)
}

impl Bus {
    pub fn system() -> io::Result<Bus> {
        Connection::connect_system().map(|conn| Bus { conn, flags: FLAG_NO_AUTO_START })
    }

    pub fn session() -> io::Result<Bus> {
        Connection::connect().map(|conn| Bus { conn, flags: 0 })
    }

    pub fn fd(&self) -> RawFd {
        self.conn.as_raw_fd()
    }

    /// Call `member` and wait for its reply's values. On the system bus
    /// the daemon is never started for it: one that's down fails at once.
    pub fn call(&mut self, dest: &str, path: &str, iface: &str, member: &str, sig: &str, body: &[u8]) -> Result<Vec<Value>, CallError> {
        let serial = self.conn.method_call_with_flags(self.flags, dest, path, iface, member, sig, body);
        let reply = self.conn.read_reply(serial).map_err(CallError::Bus)?;
        values(&reply)
    }

    /// [`Self::call`] for a reply that takes as long as it takes, waited
    /// for up to `wait` (then `TimedOut`), answering meanwhile each call
    /// made to us with `serve`: an agent's questions, which the daemon
    /// asks before it replies. What our replies carried is wiped once
    /// they are sent.
    #[allow(clippy::too_many_arguments)]
    pub fn call_serving(&mut self, dest: &str, path: &str, iface: &str, member: &str, sig: &str, body: &[u8], wait: Duration, mut serve: impl FnMut(&Message) -> Answer) -> Result<Vec<Value>, CallError> {
        let serial = self.conn.method_call_with_flags(self.flags, dest, path, iface, member, sig, body);
        let until = Instant::now() + wait;
        loop {
            let left = until.saturating_duration_since(Instant::now());
            if left.is_zero() || !sys::wait(&[self.fd()], Some(left))[0] {
                return Err(CallError::Bus(io::ErrorKind::TimedOut.into()));
            }
            let msg = self.conn.read_next().map_err(CallError::Bus)?;
            if msg.is_method_call() {
                let answer = serve(&msg);
                self.answer(msg.serial, &msg.sender, answer);
            } else if msg.reply_serial == serial && !msg.is_signal() {
                return values(&msg);
            }
        }
    }

    /// Answer the call `serial` that `sender` made to us; what a reply
    /// carried is wiped once it's sent.
    pub fn answer(&mut self, serial: u32, sender: &str, answer: Answer) {
        match answer {
            Answer::Reply(sig, mut body) => {
                self.conn.send_reply(serial, sender, sig, &body);
                wipe_bytes(&mut body);
            }
            Answer::Refuse(name, why) => self.conn.send_error(serial, sender, name, why),
        }
    }

    /// Who has `name` on the bus now, by their unique name (`:1.7`).
    pub fn owner(&mut self, name: &str) -> Result<String, CallError> {
        let mut body = Vec::new();
        encode_string(&mut body, name);
        let reply = self.call(DBUS, "/org/freedesktop/DBus", DBUS, "GetNameOwner", "s", &body)?;
        reply.first().and_then(Value::as_str).map(str::to_owned).ok_or_else(|| CallError::Refused { name: String::new(), why: format!("no one has {name}") })
    }

    /// Call a method that takes no arguments.
    pub fn call0(&mut self, dest: &str, path: &str, iface: &str, member: &str) -> Result<Vec<Value>, CallError> {
        self.call(dest, path, iface, member, "", &[])
    }

    /// Every object `dest`'s object manager at `/` has.
    pub fn objects(&mut self, dest: &str) -> Result<Objects, CallError> {
        let reply = self.call0(dest, "/", OBJECT_MANAGER, "GetManagedObjects")?;
        Ok(reply.first().map(unpack).unwrap_or_default())
    }

    /// Set the boolean property `name` of `iface` on `path`.
    pub fn set_bool(&mut self, dest: &str, path: &str, iface: &str, name: &str, on: bool) -> Result<(), CallError> {
        let mut body = Vec::new();
        encode_string(&mut body, iface);
        encode_string(&mut body, name);
        encode_variant_bool(&mut body, on);
        self.call(dest, path, PROPERTIES, "Set", "ssv", &body).map(|_| ())
    }

    /// Hear every signal `name` sends, and its coming and going.
    pub fn listen(&mut self, name: &str) {
        self.conn.add_match(&format!("type='signal',sender='{name}'"));
        self.conn.add_match(&format!("type='signal',sender='{DBUS}',member='NameOwnerChanged',arg0='{name}'"));
    }

    /// Every message waiting now, read whole. An error is the connection
    /// going.
    pub fn drain(&mut self) -> io::Result<Vec<Message>> {
        let mut out = Vec::new();
        while sys::wait(&[self.fd()], Some(Duration::ZERO))[0] {
            out.push(self.conn.read_next()?);
        }
        Ok(out)
    }
}

/// An `a{oa{sa{sv}}}` as maps.
fn unpack(value: &Value) -> Objects {
    let Some(paths) = value.as_dict() else { return Objects::new() };
    paths
        .iter()
        .map(|(path, ifaces)| {
            let ifaces = ifaces.as_dict().map(|d| d.iter().map(|(iface, props)| (iface.clone(), props.as_dict().cloned().unwrap_or_default())).collect()).unwrap_or_default();
            (path.clone(), ifaces)
        })
        .collect()
}

/// A property read as text (a string or an object path).
pub fn text(props: &Props, key: &str) -> Option<String> {
    props.get(key).and_then(Value::as_str).map(str::to_owned)
}

pub fn flag(props: &Props, key: &str) -> bool {
    props.get(key).and_then(Value::as_bool).unwrap_or(false)
}

/// A property read as a whole number, whatever its width and sign.
pub fn number(props: &Props, key: &str) -> Option<i64> {
    match props.get(key)? {
        Value::Int32(v) => Some(i64::from(*v)),
        Value::Uint32(v) => Some(i64::from(*v)),
        Value::Int64(v) => Some(*v),
        Value::Uint64(v) => i64::try_from(*v).ok(),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_object_managers_map_reads_as_maps() {
        let props = HashMap::from([("Powered".to_owned(), Value::Bool(true)), ("Name".to_owned(), Value::String("hci0".into()))]);
        let ifaces = HashMap::from([("org.bluez.Adapter1".to_owned(), Value::Dict(props))]);
        let all = Value::Dict(HashMap::from([("/org/bluez/hci0".to_owned(), Value::Dict(ifaces))]));
        let objects = unpack(&all);
        let adapter = &objects["/org/bluez/hci0"]["org.bluez.Adapter1"];
        assert!(flag(adapter, "Powered"));
        assert_eq!(text(adapter, "Name").as_deref(), Some("hci0"));
        assert_eq!(number(adapter, "Name"), None);
        assert!(unpack(&Value::Bool(true)).is_empty());
    }
}

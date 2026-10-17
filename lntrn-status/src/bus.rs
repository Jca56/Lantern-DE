//! The system bus as the D-Bus subsystems use it, over `lntrn-dbus`:
//! calls that wait for their reply and never start a daemon that isn't
//! running, an object manager's objects as maps, properties set, and the
//! signals of one daemon heard as they come.
//!
//! A message is only ever read whole: the thread waits for the socket to
//! have something, then reads one message blocking (a non-blocking read
//! can stop halfway through one and lose its start).

use std::collections::HashMap;
use std::io;
use std::os::fd::RawFd;
use std::time::Duration;

use lntrn_dbus::{BodyReader, Connection, FLAG_NO_AUTO_START, Message, Value, encode_string, encode_variant_bool};

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
    /// The daemon answered no, saying why.
    Refused(String),
}

impl std::fmt::Display for CallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CallError::Bus(e) => write!(f, "{e}"),
            CallError::Refused(why) => f.write_str(why),
        }
    }
}

pub struct Bus {
    conn: Connection,
}

impl Bus {
    pub fn system() -> io::Result<Bus> {
        Connection::connect_system().map(|conn| Bus { conn })
    }

    pub fn fd(&self) -> RawFd {
        self.conn.as_raw_fd()
    }

    /// Call `member` and wait for its reply's values. The daemon is never
    /// started for it: one that's down fails at once.
    pub fn call(&mut self, dest: &str, path: &str, iface: &str, member: &str, sig: &str, body: &[u8]) -> Result<Vec<Value>, CallError> {
        let serial = self.conn.method_call_with_flags(FLAG_NO_AUTO_START, dest, path, iface, member, sig, body);
        let reply = self.conn.read_reply(serial).map_err(CallError::Bus)?;
        let values = BodyReader::new(&reply.body, &reply.signature).read_all();
        if reply.is_error() {
            let why = values.first().and_then(Value::as_str).unwrap_or("refused").to_owned();
            return Err(CallError::Refused(why));
        }
        Ok(values)
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

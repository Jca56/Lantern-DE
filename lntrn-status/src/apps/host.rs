//! The desktop's tray on the session bus: `org.kde.StatusNotifierWatcher`,
//! which apps register their items with (through libappindicator, or
//! their own SNI code), and the host that reads them, all on one
//! connection. Calls go out without waiting and their replies are matched
//! back by serial, so a slow app holds up no other. Each item is followed
//! by its owner's unique name, which its signals come from (a well-known
//! name, `org.kde.StatusNotifierItem-4242-1`, is only what it registered),
//! and dropped when that leaves the bus.
//!
//! Spec: <https://www.freedesktop.org/wiki/Specifications/StatusNotifierItem/>

use std::collections::HashMap;

use lntrn_dbus::{BodyReader, Message, Value, align_to, encode_i32, encode_signature, encode_string, encode_u32, encode_variant_bool};

use super::{AppItem, Menu, item, menu};
use crate::bus::{Answer, Bus};

pub(super) const WATCHER: &str = "org.kde.StatusNotifierWatcher";
const WATCHER_PATH: &str = "/StatusNotifierWatcher";
const ITEM: &str = "org.kde.StatusNotifierItem";
const MENU: &str = "com.canonical.dbusmenu";
const PROPERTIES: &str = "org.freedesktop.DBus.Properties";
const DBUS: &str = "org.freedesktop.DBus";
/// An item's signals that say it changed: its properties are read again.
const CHANGES: [&str; 7] = ["NewIcon", "NewAttentionIcon", "NewOverlayIcon", "NewStatus", "NewTitle", "NewToolTip", "NewMenu"];

/// An item registered: where it is on the bus, who owns it, its menu.
struct Registered {
    bus: String,
    path: String,
    /// Its owner's unique name, once known.
    owner: String,
    menu: Option<String>,
    shown: AppItem,
}

/// A call out, waiting on its reply.
enum Pending {
    Props(String),
    Owner(String),
    AboutToShow { key: String, menu: String, ask: u64 },
    Layout { key: String, ask: u64 },
    Activate(String),
}

#[derive(Default)]
pub(super) struct Host {
    items: Vec<Registered>,
    pending: HashMap<u32, Pending>,
}

impl Host {
    /// Be the watcher on `bus`: hear what items say, tell apps a host is
    /// up. `false` when another tray has the name.
    pub(super) fn start(bus: &mut Bus) -> bool {
        if !bus.claim(WATCHER) {
            return false;
        }
        bus.hear(&format!("type='signal',sender='{DBUS}',member='NameOwnerChanged'"));
        for member in CHANGES {
            bus.hear(&format!("type='signal',interface='{ITEM}',member='{member}'"));
        }
        // libappindicator asks whether a host is up before it registers;
        // this says so to those that listen instead.
        bus.signal(WATCHER_PATH, WATCHER, "StatusNotifierHostRegistered", "", &[]);
        true
    }

    /// What the board shows: every item, in the order they came.
    pub(super) fn shown(&self) -> Vec<AppItem> {
        self.items.iter().map(|r| r.shown.clone()).collect()
    }

    fn find(&self, key: &str) -> Option<&Registered> {
        self.items.iter().find(|r| r.shown.key == key)
    }

    /// Read the item at `key`'s properties again.
    pub(super) fn refresh(&mut self, bus: &mut Bus, key: &str) {
        let Some((dest, path)) = self.find(key).map(|r| (r.bus.clone(), r.path.clone())) else { return };
        let mut body = Vec::new();
        encode_string(&mut body, ITEM);
        let serial = bus.ask(&dest, &path, PROPERTIES, "GetAll", "s", &body);
        self.pending.insert(serial, Pending::Props(key.to_owned()));
    }

    /// Every item's properties again: one gone without the bus saying so
    /// fails to answer, and goes.
    pub(super) fn refresh_all(&mut self, bus: &mut Bus) {
        for key in self.items.iter().map(|r| r.shown.key.clone()).collect::<Vec<_>>() {
            self.refresh(bus, &key);
        }
    }

    /// Ask the item at `key` to do `member` (`Activate`, `SecondaryActivate`)
    /// at (`x`, `y`) on the screen.
    pub(super) fn activate(&mut self, bus: &mut Bus, key: &str, member: &str) {
        let Some(r) = self.find(key) else { return };
        if member == "Activate" && r.shown.item_is_menu {
            return;
        }
        let (dest, path) = (r.bus.clone(), r.path.clone());
        let mut body = Vec::new();
        encode_i32(&mut body, 0);
        encode_i32(&mut body, 0);
        let serial = bus.ask(&dest, &path, ITEM, member, "ii", &body);
        if member == "Activate" {
            self.pending.insert(serial, Pending::Activate(key.to_owned()));
        }
    }

    /// The wheel over the item at `key`: `notches`, up positive.
    pub(super) fn scroll(&mut self, bus: &mut Bus, key: &str, notches: i32) {
        let Some(r) = self.find(key) else { return };
        let mut body = Vec::new();
        encode_i32(&mut body, notches);
        encode_string(&mut body, "vertical");
        bus.ask(&r.bus.clone(), &r.path.clone(), ITEM, "Scroll", "is", &body);
    }

    /// Fetch the item at `key`'s menu, for the ask `ask`: told it's about
    /// to show, then its layout. One with none is asked to show its own.
    pub(super) fn menu(&mut self, bus: &mut Bus, key: &str, ask: u64) {
        let Some(r) = self.find(key) else { return };
        let (dest, path) = (r.bus.clone(), r.path.clone());
        match r.menu.clone() {
            Some(menu) => {
                let serial = bus.ask(&dest, &menu, MENU, "AboutToShow", "i", &menu::about_to_show());
                self.pending.insert(serial, Pending::AboutToShow { key: key.to_owned(), menu, ask });
            }
            None => {
                let mut body = Vec::new();
                encode_i32(&mut body, 0);
                encode_i32(&mut body, 0);
                bus.ask(&dest, &path, ITEM, "ContextMenu", "ii", &body);
            }
        }
    }

    /// A click on the row `id` of the item at `key`'s menu.
    pub(super) fn click(&mut self, bus: &mut Bus, key: &str, id: i32) {
        if let Some((dest, menu)) = self.find(key).and_then(|r| Some((r.bus.clone(), r.menu.clone()?))) {
            bus.ask(&dest, &menu, MENU, "Event", "isvu", &menu::clicked(id));
        }
    }

    /// Take `msg`. Returns whether the items changed, and a menu that came.
    pub(super) fn take(&mut self, bus: &mut Bus, msg: &Message) -> (bool, Option<Menu>) {
        if msg.is_method_call() {
            return (self.called(bus, msg), None);
        }
        if msg.is_signal() {
            return (self.signalled(bus, msg), None);
        }
        match self.pending.remove(&msg.reply_serial) {
            Some(call) => self.replied(bus, msg, call),
            None => (false, None),
        }
    }

    /// A call made to the watcher.
    fn called(&mut self, bus: &mut Bus, msg: &Message) -> bool {
        let args = BodyReader::new(&msg.body, &msg.signature).read_all();
        let text = |i: usize| args.get(i).and_then(Value::as_str).unwrap_or_default().to_owned();
        let ok = || Answer::Reply("", Vec::new());
        let (answer, changed) = match (msg.interface.as_str(), msg.member.as_str()) {
            (WATCHER, "RegisterStatusNotifierItem") => (ok(), self.register(bus, &text(0), &msg.sender)),
            (WATCHER, "RegisterStatusNotifierHost") | ("org.freedesktop.DBus.Peer", "Ping") => (ok(), false),
            (PROPERTIES, "Get") => (self.property(&text(1)), false),
            (PROPERTIES, "GetAll") => (Answer::Reply("a{sv}", self.properties()), false),
            ("org.freedesktop.DBus.Introspectable", "Introspect") => {
                let mut body = Vec::new();
                encode_string(&mut body, INTROSPECT);
                (Answer::Reply("s", body), false)
            }
            _ => (Answer::Refuse("org.freedesktop.DBus.Error.UnknownMethod", "No such method"), false),
        };
        bus.answer(msg.serial, &msg.sender, answer);
        changed
    }

    /// An app registered the item `service`.
    fn register(&mut self, bus: &mut Bus, service: &str, sender: &str) -> bool {
        let (dest, path) = item::place(service, sender);
        let key = format!("{dest}{path}");
        if !self.items.iter().any(|r| r.shown.key == key) {
            // Its unique name: who registered it, unless that was another.
            let owner = if dest.starts_with(':') { dest.clone() } else { String::new() };
            if owner.is_empty() {
                let mut body = Vec::new();
                encode_string(&mut body, &dest);
                let serial = bus.ask(DBUS, "/org/freedesktop/DBus", DBUS, "GetNameOwner", "s", &body);
                self.pending.insert(serial, Pending::Owner(key.clone()));
            }
            self.items.push(Registered { bus: dest, path, owner, menu: None, shown: AppItem { key: key.clone(), ..AppItem::default() } });
        }
        self.refresh(bus, &key);
        let mut body = Vec::new();
        encode_string(&mut body, &key);
        bus.signal(WATCHER_PATH, WATCHER, "StatusNotifierItemRegistered", "s", &body);
        // Shown once its properties come.
        false
    }

    /// A reply to the call `call`.
    fn replied(&mut self, bus: &mut Bus, msg: &Message, call: Pending) -> (bool, Option<Menu>) {
        let values = BodyReader::new(&msg.body, &msg.signature).read_all();
        match call {
            Pending::Props(key) if msg.is_error() => (self.drop_item(bus, |r| r.shown.key == key), None),
            Pending::Props(key) => {
                let Some(r) = self.items.iter_mut().find(|r| r.shown.key == key) else { return (false, None) };
                let before = r.shown.clone();
                r.menu = values.first().and_then(Value::as_dict).and_then(|props| item::apply(&mut r.shown, props));
                (r.shown != before, None)
            }
            Pending::Owner(key) => {
                if let (Some(r), Some(owner)) = (self.items.iter_mut().find(|r| r.shown.key == key), values.first().and_then(Value::as_str)) {
                    r.owner = owner.to_owned();
                }
                (false, None)
            }
            // Whether it needed telling or not, the layout now.
            Pending::AboutToShow { key, menu, ask } => {
                if let Some(dest) = self.find(&key).map(|r| r.bus.clone()) {
                    let serial = bus.ask(&dest, &menu, MENU, "GetLayout", "iias", &menu::get_layout());
                    self.pending.insert(serial, Pending::Layout { key, ask });
                }
                (false, None)
            }
            Pending::Layout { key, ask } => {
                let entries = (!msg.is_error()).then(|| menu::read_layout(&values)).flatten();
                (false, entries.map(|entries| Menu { key, ask, entries }))
            }
            // An item that is only a menu doesn't always say so: its menu.
            Pending::Activate(key) if msg.is_error() => {
                self.menu(bus, &key, 0);
                (false, None)
            }
            Pending::Activate(_) => (false, None),
        }
    }

    /// A signal: an item changed, or its app left the bus.
    fn signalled(&mut self, bus: &mut Bus, msg: &Message) -> bool {
        if msg.member == "NameOwnerChanged" {
            let args = BodyReader::new(&msg.body, &msg.signature).read_all();
            let text = |i: usize| args.get(i).and_then(Value::as_str).unwrap_or_default().to_owned();
            let (name, new) = (text(0), text(2));
            return new.is_empty() && !name.is_empty() && self.drop_item(bus, |r| r.bus == name || r.owner == name);
        }
        if msg.interface == ITEM && CHANGES.contains(&msg.member.as_str()) {
            let keys: Vec<String> = self.items.iter().filter(|r| r.owner == msg.sender || r.bus == msg.sender).map(|r| r.shown.key.clone()).collect();
            for key in keys {
                self.refresh(bus, &key);
            }
        }
        false
    }

    /// Drop the items `gone` says, telling the bus. Returns whether any went.
    fn drop_item(&mut self, bus: &mut Bus, gone: impl Fn(&Registered) -> bool) -> bool {
        let (going, kept): (Vec<_>, Vec<_>) = std::mem::take(&mut self.items).into_iter().partition(|r| gone(r));
        self.items = kept;
        for r in &going {
            let mut body = Vec::new();
            encode_string(&mut body, &r.shown.key);
            bus.signal(WATCHER_PATH, WATCHER, "StatusNotifierItemUnregistered", "s", &body);
        }
        !going.is_empty()
    }

    /// One of the watcher's properties, as a variant.
    fn property(&self, name: &str) -> Answer {
        let mut body = Vec::new();
        match name {
            "RegisteredStatusNotifierItems" => self.items_variant(&mut body),
            "IsStatusNotifierHostRegistered" => encode_variant_bool(&mut body, true),
            "ProtocolVersion" => version_variant(&mut body),
            _ => return Answer::Refuse("org.freedesktop.DBus.Error.InvalidArgs", "No such property"),
        }
        Answer::Reply("v", body)
    }

    /// All the watcher's properties, `a{sv}`.
    fn properties(&self) -> Vec<u8> {
        let mut body = Vec::new();
        align_to(&mut body, 4);
        let len_at = body.len();
        encode_u32(&mut body, 0);
        align_to(&mut body, 8);
        let start = body.len();
        for name in ["RegisteredStatusNotifierItems", "IsStatusNotifierHostRegistered", "ProtocolVersion"] {
            align_to(&mut body, 8);
            encode_string(&mut body, name);
            match name {
                "RegisteredStatusNotifierItems" => self.items_variant(&mut body),
                "IsStatusNotifierHostRegistered" => encode_variant_bool(&mut body, true),
                _ => version_variant(&mut body),
            }
        }
        let len = (body.len() - start) as u32;
        body[len_at..len_at + 4].copy_from_slice(&len.to_le_bytes());
        body
    }

    /// A variant of the items registered, `as` of `name/path`.
    fn items_variant(&self, body: &mut Vec<u8>) {
        encode_signature(body, "as");
        align_to(body, 4);
        let len_at = body.len();
        encode_u32(body, 0);
        let start = body.len();
        for r in &self.items {
            encode_string(body, &r.shown.key);
        }
        let len = (body.len() - start) as u32;
        body[len_at..len_at + 4].copy_from_slice(&len.to_le_bytes());
    }
}

/// `ProtocolVersion` as a variant: 0.
fn version_variant(body: &mut Vec<u8>) {
    encode_signature(body, "i");
    align_to(body, 4);
    encode_i32(body, 0);
}

const INTROSPECT: &str = r#"<!DOCTYPE node PUBLIC "-//freedesktop//DTD D-BUS Object Introspection 1.0//EN"
 "http://www.freedesktop.org/standards/dbus/1.0/introspect.dtd">
<node>
  <interface name="org.kde.StatusNotifierWatcher">
    <method name="RegisterStatusNotifierItem"><arg direction="in" name="service" type="s"/></method>
    <method name="RegisterStatusNotifierHost"><arg direction="in" name="service" type="s"/></method>
    <signal name="StatusNotifierItemRegistered"><arg type="s"/></signal>
    <signal name="StatusNotifierItemUnregistered"><arg type="s"/></signal>
    <signal name="StatusNotifierHostRegistered"/>
    <property name="RegisteredStatusNotifierItems" type="as" access="read"/>
    <property name="IsStatusNotifierHostRegistered" type="b" access="read"/>
    <property name="ProtocolVersion" type="i" access="read"/>
  </interface>
</node>"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_watchers_properties_read_back() {
        let mut host = Host::default();
        host.items.push(Registered { bus: ":1.40".into(), path: "/StatusNotifierItem".into(), owner: ":1.40".into(), menu: None, shown: AppItem { key: ":1.40/StatusNotifierItem".into(), ..AppItem::default() } });
        let all = host.properties();
        let props = BodyReader::new(&all, "a{sv}").read_all();
        let props = props[0].as_dict().unwrap();
        let items: Vec<&str> = props["RegisteredStatusNotifierItems"].as_array().unwrap().iter().filter_map(Value::as_str).collect();
        assert_eq!(items, [":1.40/StatusNotifierItem"]);
        assert_eq!(props["IsStatusNotifierHostRegistered"].as_bool(), Some(true));
        assert!(matches!(host.property("Nope"), Answer::Refuse(..)));
    }
}

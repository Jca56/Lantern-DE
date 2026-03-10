//! StatusNotifierItem watcher + host — we claim the watcher bus name and
//! directly handle RegisterStatusNotifierItem calls from tray apps.

use std::collections::HashMap;

use crate::dbus::{self, BodyReader, Connection, Message, Value};

const WATCHER_NAME: &str = "org.kde.StatusNotifierWatcher";
const WATCHER_PATH: &str = "/StatusNotifierWatcher";
const WATCHER_IFACE: &str = "org.kde.StatusNotifierWatcher";
const ITEM_IFACE: &str = "org.kde.StatusNotifierItem";
const PROPS_IFACE: &str = "org.freedesktop.DBus.Properties";

// ── Public types ────────────────────────────────────────────────────────────

/// An ARGB icon pixmap from an SNI item.
#[derive(Debug, Clone)]
pub struct IconPixmap {
    pub width: u32,
    pub height: u32,
    /// RGBA pixel data (converted from SNI's network-byte-order ARGB).
    pub rgba: Vec<u8>,
}

/// Snapshot of an SNI item's visible state.
#[derive(Debug, Clone)]
pub struct SniItem {
    pub bus_name: String,
    pub obj_path: String,
    pub id: String,
    pub title: String,
    pub status: String,
    pub icon_name: String,
    pub icon_pixmap: Option<IconPixmap>,
}

/// Commands sent from the render thread to the D-Bus thread.
#[derive(Debug)]
pub enum TrayCommand {
    Activate { bus_name: String, obj_path: String, x: i32, y: i32 },
}

// ── SNI Host (acts as watcher) ──────────────────────────────────────────────

enum PendingCall {
    GetAllProps(String, String), // bus_name, obj_path
}

pub struct SniHost {
    conn: Connection,
    items: Vec<SniItem>,
    pending: HashMap<u32, PendingCall>,
    dirty: bool,
}

impl SniHost {
    /// Connect to session bus and claim the StatusNotifierWatcher name.
    pub fn connect() -> Result<Self, String> {
        let mut conn = Connection::connect()
            .map_err(|e| format!("D-Bus connect failed: {e}"))?;

        // Claim the watcher bus name — we ARE the watcher now
        if !conn.request_name(WATCHER_NAME) {
            return Err("failed to claim org.kde.StatusNotifierWatcher".into());
        }
        tracing::info!("claimed {WATCHER_NAME} — we are the watcher");

        // Subscribe to NameOwnerChanged so we can detect when items disconnect
        conn.add_match(
            "type='signal',sender='org.freedesktop.DBus',\
             member='NameOwnerChanged'"
        );

        // Subscribe to item property change signals
        conn.add_match(
            "type='signal',interface='org.kde.StatusNotifierItem',member='NewIcon'"
        );
        conn.add_match(
            "type='signal',interface='org.kde.StatusNotifierItem',member='NewStatus'"
        );
        conn.add_match(
            "type='signal',interface='org.kde.StatusNotifierItem',member='NewTitle'"
        );

        Ok(Self {
            conn,
            items: Vec::new(),
            pending: HashMap::new(),
            dirty: false,
        })
    }

    fn request_item_props(&mut self, bus_name: &str, obj_path: &str) {
        let mut body = Vec::new();
        dbus::encode_string(&mut body, ITEM_IFACE);
        let serial = self.conn.method_call(
            bus_name, obj_path, PROPS_IFACE, "GetAll", "s", &body,
        );
        self.pending.insert(
            serial,
            PendingCall::GetAllProps(bus_name.to_string(), obj_path.to_string()),
        );
    }

    /// Send an Activate call to an item.
    pub fn activate(&mut self, bus_name: &str, obj_path: &str, x: i32, y: i32) {
        let mut body = Vec::new();
        dbus::encode_i32(&mut body, x);
        dbus::encode_i32(&mut body, y);
        self.conn.method_call(bus_name, obj_path, ITEM_IFACE, "Activate", "ii", &body);
    }

    /// Poll for D-Bus messages. Returns true if the item list changed.
    pub fn poll(&mut self) -> bool {
        self.dirty = false;
        while let Some(msg) = self.conn.try_read() {
            self.handle_message(msg);
        }
        self.dirty
    }

    pub fn items(&self) -> &[SniItem] {
        &self.items
    }

    fn handle_message(&mut self, msg: Message) {
        match msg.msg_type {
            1 => self.handle_method_call(msg),     // incoming method call
            2 | 3 => self.handle_reply(msg),       // method_return or error
            4 => self.handle_signal(msg),
            _ => {}
        }
    }

    /// Handle incoming method calls (we're the watcher, apps call us).
    fn handle_method_call(&mut self, msg: Message) {
        match (msg.interface.as_str(), msg.member.as_str()) {
            (WATCHER_IFACE, "RegisterStatusNotifierItem") => {
                let mut reader = BodyReader::new(&msg.body, &msg.signature);
                if let Some(Value::String(service)) = reader.read_value("s") {
                    // The service can be a bus name or bus_name/obj_path
                    let (bus, path) = parse_item_name(&service, &msg.sender);
                    tracing::info!(bus = %bus, path = %path, "SNI item registered");

                    if !self.items.iter().any(|i| i.bus_name == bus) {
                        self.items.push(SniItem {
                            bus_name: bus.clone(),
                            obj_path: path.clone(),
                            id: String::new(),
                            title: String::new(),
                            status: "Active".into(),
                            icon_name: String::new(),
                            icon_pixmap: None,
                        });
                        self.request_item_props(&bus, &path);
                        self.dirty = true;

                        // Emit signal so other hosts know
                        let mut sig_body = Vec::new();
                        let full_name = format!("{bus}{path}");
                        dbus::encode_string(&mut sig_body, &full_name);
                        self.conn.send_signal(
                            WATCHER_PATH, WATCHER_IFACE,
                            "StatusNotifierItemRegistered", "s", &sig_body,
                        );
                    }
                }
                // Reply with empty method return
                self.conn.send_reply(msg.serial, &msg.sender, "", &[]);
            }
            (WATCHER_IFACE, "RegisterStatusNotifierHost") => {
                // Another host registered — just ack it
                tracing::info!(sender = %msg.sender, "SNI host registered");
                self.conn.send_reply(msg.serial, &msg.sender, "", &[]);
            }
            (PROPS_IFACE, "Get") => {
                self.handle_prop_get(&msg);
            }
            (PROPS_IFACE, "GetAll") => {
                self.handle_prop_getall(&msg);
            }
            ("org.freedesktop.DBus.Introspectable", "Introspect") => {
                let xml = WATCHER_INTROSPECT;
                let mut body = Vec::new();
                dbus::encode_string(&mut body, xml);
                self.conn.send_reply(msg.serial, &msg.sender, "s", &body);
            }
            _ => {
                // Unknown method — ignore or send error
            }
        }
    }

    fn handle_prop_get(&mut self, msg: &Message) {
        let mut reader = BodyReader::new(&msg.body, &msg.signature);
        let _iface = reader.read_value("s");
        let prop = reader.read_value("s");
        let prop_name = prop.as_ref().and_then(|v| v.as_str()).unwrap_or("");

        match prop_name {
            "RegisteredStatusNotifierItems" => {
                let body = self.encode_items_variant();
                self.conn.send_reply(msg.serial, &msg.sender, "v", &body);
            }
            "IsStatusNotifierHostRegistered" => {
                let mut body = Vec::new();
                // variant sig "b" + bool true
                body.push(1); // sig len
                body.extend_from_slice(b"b\0");
                // align to 4 for the bool
                while body.len() % 4 != 0 { body.push(0); }
                body.extend_from_slice(&1u32.to_le_bytes());
                self.conn.send_reply(msg.serial, &msg.sender, "v", &body);
            }
            "ProtocolVersion" => {
                let mut body = Vec::new();
                body.push(1); body.extend_from_slice(b"i\0");
                while body.len() % 4 != 0 { body.push(0); }
                body.extend_from_slice(&0i32.to_le_bytes());
                self.conn.send_reply(msg.serial, &msg.sender, "v", &body);
            }
            _ => {
                self.conn.send_reply(msg.serial, &msg.sender, "", &[]);
            }
        }
    }

    fn handle_prop_getall(&mut self, msg: &Message) {
        // Return a{sv} with our watcher properties
        let items_list = self.items.iter()
            .map(|i| format!("{}{}", i.bus_name, i.obj_path))
            .collect::<Vec<_>>();

        // For simplicity, just reply empty — most clients use Get
        let mut body = Vec::new();
        // Empty dict: array length 0
        body.extend_from_slice(&0u32.to_le_bytes());
        self.conn.send_reply(msg.serial, &msg.sender, "a{sv}", &body);
        let _ = items_list; // suppress warning
    }

    fn encode_items_variant(&self) -> Vec<u8> {
        // Variant containing array of strings: v -> "as" -> [items...]
        let mut body = Vec::new();
        // Variant signature: "as"
        body.push(2); // sig length
        body.extend_from_slice(b"as\0");

        // Array of strings
        // First: array byte length (placeholder, we'll fill it)
        while body.len() % 4 != 0 { body.push(0); }
        let array_len_pos = body.len();
        body.extend_from_slice(&0u32.to_le_bytes());

        let array_start = body.len();
        for item in &self.items {
            let name = format!("{}{}", item.bus_name, item.obj_path);
            dbus::encode_string(&mut body, &name);
        }
        let array_len = (body.len() - array_start) as u32;
        body[array_len_pos..array_len_pos + 4].copy_from_slice(&array_len.to_le_bytes());

        body
    }

    fn handle_reply(&mut self, msg: Message) {
        let Some(call) = self.pending.remove(&msg.reply_serial) else { return };

        match call {
            PendingCall::GetAllProps(bus_name, obj_path) => {
                if msg.msg_type == 3 { return; } // error response
                let mut reader = BodyReader::new(&msg.body, &msg.signature);
                if let Some(Value::Dict(props)) = reader.read_value(&msg.signature) {
                    self.apply_props(&bus_name, &obj_path, &props);
                }
            }
        }
    }

    fn handle_signal(&mut self, msg: Message) {
        match msg.member.as_str() {
            "NameOwnerChanged" => {
                // A bus name changed owner — check if any of our items disconnected
                let mut reader = BodyReader::new(&msg.body, &msg.signature);
                let name = reader.read_value("s")
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_default();
                let _old_owner = reader.read_value("s");
                let new_owner = reader.read_value("s")
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_default();

                // If new_owner is empty, the name went away
                if new_owner.is_empty() {
                    let before = self.items.len();
                    self.items.retain(|i| i.bus_name != name);
                    if self.items.len() != before {
                        tracing::info!(name = %name, "SNI item disconnected");
                        // Emit unregistered signal
                        let mut sig_body = Vec::new();
                        dbus::encode_string(&mut sig_body, &name);
                        self.conn.send_signal(
                            WATCHER_PATH, WATCHER_IFACE,
                            "StatusNotifierItemUnregistered", "s", &sig_body,
                        );
                        self.dirty = true;
                    }
                }
            }
            "NewIcon" | "NewTitle" | "NewStatus" => {
                if let Some(item) = self.items.iter().find(|i| i.bus_name == msg.sender) {
                    let bus = item.bus_name.clone();
                    let path = item.obj_path.clone();
                    self.request_item_props(&bus, &path);
                }
            }
            _ => {}
        }
    }

    fn apply_props(&mut self, bus_name: &str, _obj_path: &str, props: &HashMap<String, Value>) {
        let Some(item) = self.items.iter_mut().find(|i| i.bus_name == bus_name) else { return };

        if let Some(v) = props.get("Id") {
            item.id = v.as_str().unwrap_or("").to_string();
        }
        if let Some(v) = props.get("Title") {
            item.title = v.as_str().unwrap_or("").to_string();
        }
        if let Some(v) = props.get("Status") {
            item.status = v.as_str().unwrap_or("Active").to_string();
        }
        if let Some(v) = props.get("IconName") {
            item.icon_name = v.as_str().unwrap_or("").to_string();
        }
        if let Some(Value::Array(pixmaps)) = props.get("IconPixmap") {
            item.icon_pixmap = pick_best_pixmap(pixmaps, 64);
        }

        self.dirty = true;
        tracing::info!(
            id = %item.id, title = %item.title, status = %item.status,
            icon = %item.icon_name, has_pixmap = item.icon_pixmap.is_some(),
            "SNI item props updated"
        );
    }
}

// ── Helpers ─────────────────────────────────────────────────────────────────

/// Parse an SNI item name into (bus_name, object_path).
/// The service string can be just an object path (sender is the bus name),
/// or "bus_name/obj_path", or just a bus name.
fn parse_item_name(service: &str, sender: &str) -> (String, String) {
    if service.starts_with('/') {
        // It's just an object path — use the sender as bus name
        (sender.to_string(), service.to_string())
    } else if let Some(idx) = service.find('/') {
        (service[..idx].to_string(), service[idx..].to_string())
    } else {
        (service.to_string(), "/StatusNotifierItem".to_string())
    }
}

/// Pick the icon pixmap closest to `target_size`, convert ARGB→RGBA.
fn pick_best_pixmap(pixmaps: &[Value], target_size: u32) -> Option<IconPixmap> {
    let mut best: Option<(u32, u32, &[u8])> = None;
    let mut best_dist = u32::MAX;

    for pm in pixmaps {
        if let Value::Struct(fields) = pm {
            if fields.len() >= 3 {
                let w = fields[0].as_i32().unwrap_or(0) as u32;
                let h = fields[1].as_i32().unwrap_or(0) as u32;
                if let Some(data) = fields[2].as_bytes() {
                    let dist = (w as i64 - target_size as i64).unsigned_abs() as u32;
                    if dist < best_dist {
                        best = Some((w, h, data));
                        best_dist = dist;
                    }
                }
            }
        }
    }

    let (w, h, argb_data) = best?;
    let expected = (w * h * 4) as usize;
    if argb_data.len() < expected { return None; }

    // Convert network-byte-order ARGB to RGBA
    let mut rgba = vec![0u8; expected];
    for i in 0..(w * h) as usize {
        let src = i * 4;
        let a = argb_data[src];
        let r = argb_data[src + 1];
        let g = argb_data[src + 2];
        let b = argb_data[src + 3];
        rgba[src] = r;
        rgba[src + 1] = g;
        rgba[src + 2] = b;
        rgba[src + 3] = a;
    }

    Some(IconPixmap { width: w, height: h, rgba })
}

// ── Introspection XML ───────────────────────────────────────────────────────

const WATCHER_INTROSPECT: &str = r#"<!DOCTYPE node PUBLIC "-//freedesktop//DTD D-BUS Object Introspection 1.0//EN"
 "http://www.freedesktop.org/standards/dbus/1.0/introspect.dtd">
<node>
  <interface name="org.kde.StatusNotifierWatcher">
    <method name="RegisterStatusNotifierItem">
      <arg direction="in" name="service" type="s"/>
    </method>
    <method name="RegisterStatusNotifierHost">
      <arg direction="in" name="service" type="s"/>
    </method>
    <signal name="StatusNotifierItemRegistered">
      <arg type="s"/>
    </signal>
    <signal name="StatusNotifierItemUnregistered">
      <arg type="s"/>
    </signal>
    <property name="RegisteredStatusNotifierItems" type="as" access="read"/>
    <property name="IsStatusNotifierHostRegistered" type="b" access="read"/>
    <property name="ProtocolVersion" type="i" access="read"/>
  </interface>
</node>"#;

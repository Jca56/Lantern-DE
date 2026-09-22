//! StatusNotifierItem watcher + host on the session bus.
//!
//! We claim `org.kde.StatusNotifierWatcher` ourselves and answer the
//! `RegisterStatusNotifierItem` calls tray apps (Steam, Discord, Spotify,
//! Telegram, …) make through libappindicator / their own SNI client.
//! Being both watcher and host in one connection keeps the protocol to
//! a handful of messages: registration, a `GetAll` of the item's
//! properties, `NameOwnerChanged` for teardown, and the dbusmenu calls
//! behind the right-click menu.
//!
//! Spec: <https://www.freedesktop.org/wiki/Specifications/StatusNotifierItem/>
//!
//! Runs on the tray worker thread (see [`super::worker`]); nothing here
//! touches the GPU or the render loop.

use std::collections::HashMap;

use lntrn_dbus::{self as dbus, BodyReader, Connection, Message, Value};

use super::dbusmenu::{self, DbusmenuItem};
use super::{IconPixmap, TrayItem};

const WATCHER_NAME: &str = "org.kde.StatusNotifierWatcher";
const WATCHER_PATH: &str = "/StatusNotifierWatcher";
const WATCHER_IFACE: &str = "org.kde.StatusNotifierWatcher";
const ITEM_IFACE: &str = "org.kde.StatusNotifierItem";
const PROPS_IFACE: &str = "org.freedesktop.DBus.Properties";
const MENU_IFACE: &str = "com.canonical.dbusmenu";

/// Pixmap size we prefer when an item offers several. The dock draws
/// tray icons small, so a 32–64 px source is plenty.
const PIXMAP_TARGET: u32 = 48;

/// Re-read every item's properties this often as a liveness check for
/// apps that die without the bus noticing promptly (rare, but a stale
/// icon is worse than one extra `GetAll` per item every 15 s).
const LIVENESS_INTERVAL: std::time::Duration = std::time::Duration::from_secs(15);

enum PendingCall {
    /// `GetAll` on an item's SNI interface.
    GetAllProps { bus: String },
    /// `AboutToShow` on an item's menu — chained into `GetLayout`.
    AboutToShow { bus: String, menu_path: String },
    /// `GetLayout` on an item's menu.
    GetMenuLayout { bus: String, menu_path: String },
    /// `Activate` on an item. On error we fall back to fetching the
    /// menu (libappindicator items don't implement Activate — they are
    /// menu-only, but don't always say so via `ItemIsMenu`).
    Activate { bus: String },
}

/// Something the host wants the worker to forward to the render thread.
pub enum HostEvent {
    MenuReady {
        bus: String,
        menu_path: String,
        items: Vec<DbusmenuItem>,
    },
}

pub struct SniHost {
    conn: Connection,
    items: Vec<TrayItem>,
    pending: HashMap<u32, PendingCall>,
    events: Vec<HostEvent>,
    last_liveness: std::time::Instant,
}

impl SniHost {
    /// Connect to the session bus and claim the watcher name. Fails when
    /// the bus is unreachable or another watcher already owns the name.
    pub fn connect() -> Result<Self, String> {
        let mut conn = Connection::connect().map_err(|e| format!("D-Bus connect failed: {e}"))?;
        if !conn.request_name(WATCHER_NAME) {
            return Err(format!("failed to claim {WATCHER_NAME} (another tray running?)"));
        }
        tracing::info!("claimed {WATCHER_NAME} — Command Center is the system tray");

        conn.add_match("type='signal',sender='org.freedesktop.DBus',member='NameOwnerChanged'");
        for member in [
            "NewIcon",
            "NewAttentionIcon",
            "NewOverlayIcon",
            "NewStatus",
            "NewTitle",
            "NewToolTip",
            "NewMenu",
        ] {
            conn.add_match(&format!(
                "type='signal',interface='{ITEM_IFACE}',member='{member}'"
            ));
        }

        // Tell any host-watching clients a host is up. libappindicator
        // checks `IsStatusNotifierHostRegistered` before registering;
        // the signal covers clients that listen instead of polling.
        conn.send_signal(WATCHER_PATH, WATCHER_IFACE, "StatusNotifierHostRegistered", "", &[]);

        Ok(Self {
            conn,
            items: Vec::new(),
            pending: HashMap::new(),
            events: Vec::new(),
            last_liveness: std::time::Instant::now(),
        })
    }

    pub fn dbus_fd(&self) -> i32 {
        self.conn.as_raw_fd()
    }

    pub fn items(&self) -> &[TrayItem] {
        &self.items
    }

    pub fn take_events(&mut self) -> Vec<HostEvent> {
        std::mem::take(&mut self.events)
    }

    // ── Outgoing calls ──────────────────────────────────────────────────

    fn request_item_props(&mut self, bus: &str, obj_path: &str) {
        let mut body = Vec::new();
        dbus::encode_string(&mut body, ITEM_IFACE);
        let serial = self
            .conn
            .method_call(bus, obj_path, PROPS_IFACE, "GetAll", "s", &body);
        self.pending
            .insert(serial, PendingCall::GetAllProps { bus: bus.to_string() });
    }

    /// Left-click. `x`/`y` are screen coordinates the item may use to
    /// position a popup.
    pub fn activate(&mut self, bus: &str, x: i32, y: i32) {
        let Some(item) = self.items.iter().find(|i| i.bus_name == bus) else {
            return;
        };
        let obj_path = item.obj_path.clone();
        let mut body = Vec::new();
        dbus::encode_i32(&mut body, x);
        dbus::encode_i32(&mut body, y);
        let serial = self
            .conn
            .method_call(bus, &obj_path, ITEM_IFACE, "Activate", "ii", &body);
        self.pending
            .insert(serial, PendingCall::Activate { bus: bus.to_string() });
    }

    /// Request the item's menu. Items that export a dbusmenu get
    /// `AboutToShow` → `GetLayout` (some apps only populate the menu on
    /// the former); items without one get the SNI `ContextMenu` call so
    /// they can pop their own.
    pub fn request_menu(&mut self, bus: &str, x: i32, y: i32) {
        let Some(item) = self.items.iter().find(|i| i.bus_name == bus) else {
            return;
        };
        match item.menu_path.clone() {
            Some(menu_path) => {
                let mut body = Vec::new();
                dbus::encode_i32(&mut body, 0);
                let serial =
                    self.conn
                        .method_call(bus, &menu_path, MENU_IFACE, "AboutToShow", "i", &body);
                self.pending.insert(
                    serial,
                    PendingCall::AboutToShow {
                        bus: bus.to_string(),
                        menu_path,
                    },
                );
            }
            None => {
                let obj_path = item.obj_path.clone();
                let mut body = Vec::new();
                dbus::encode_i32(&mut body, x);
                dbus::encode_i32(&mut body, y);
                self.conn
                    .method_call(bus, &obj_path, ITEM_IFACE, "ContextMenu", "ii", &body);
            }
        }
    }

    fn get_menu_layout(&mut self, bus: &str, menu_path: &str) {
        let mut body = Vec::new();
        dbusmenu::encode_get_layout(&mut body);
        let serial = self
            .conn
            .method_call(bus, menu_path, MENU_IFACE, "GetLayout", "iias", &body);
        self.pending.insert(
            serial,
            PendingCall::GetMenuLayout {
                bus: bus.to_string(),
                menu_path: menu_path.to_string(),
            },
        );
    }

    /// Send a `clicked` event for a dbusmenu item.
    pub fn menu_click(&mut self, bus: &str, menu_path: &str, item_id: i32) {
        let mut body = Vec::new();
        dbusmenu::encode_menu_event(&mut body, item_id);
        self.conn
            .method_call(bus, menu_path, MENU_IFACE, "Event", "isvu", &body);
    }

    // ── Incoming ────────────────────────────────────────────────────────

    /// Drain every readable message. Returns true when the item list
    /// or any item's visible state changed.
    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        while let Some(msg) = self.conn.try_read() {
            changed |= self.handle_message(msg);
        }
        if self.last_liveness.elapsed() >= LIVENESS_INTERVAL && !self.items.is_empty() {
            self.last_liveness = std::time::Instant::now();
            let targets: Vec<(String, String)> = self
                .items
                .iter()
                .map(|i| (i.bus_name.clone(), i.obj_path.clone()))
                .collect();
            for (bus, path) in targets {
                self.request_item_props(&bus, &path);
            }
        }
        changed
    }

    fn handle_message(&mut self, msg: Message) -> bool {
        match msg.msg_type {
            1 => self.handle_method_call(msg),
            2 | 3 => self.handle_reply(msg),
            4 => self.handle_signal(msg),
            _ => false,
        }
    }

    fn handle_method_call(&mut self, msg: Message) -> bool {
        match (msg.interface.as_str(), msg.member.as_str()) {
            (WATCHER_IFACE, "RegisterStatusNotifierItem") => {
                let mut reader = BodyReader::new(&msg.body, &msg.signature);
                let service = reader
                    .read_value("s")
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_default();
                let (bus, path) = parse_item_name(&service, &msg.sender);
                self.conn.send_reply(msg.serial, &msg.sender, "", &[]);
                tracing::info!(bus = %bus, path = %path, "SNI item registered");
                if let Some(existing) = self.items.iter_mut().find(|i| i.bus_name == bus) {
                    // Re-registration (app restarted its indicator):
                    // just refresh the path + props.
                    existing.obj_path = path.clone();
                } else {
                    self.items.push(TrayItem::new(bus.clone(), path.clone()));
                }
                self.request_item_props(&bus, &path);
                let mut sig_body = Vec::new();
                dbus::encode_string(&mut sig_body, &format!("{bus}{path}"));
                self.conn.send_signal(
                    WATCHER_PATH,
                    WATCHER_IFACE,
                    "StatusNotifierItemRegistered",
                    "s",
                    &sig_body,
                );
                true
            }
            (WATCHER_IFACE, "RegisterStatusNotifierHost") => {
                tracing::info!(sender = %msg.sender, "another SNI host registered");
                self.conn.send_reply(msg.serial, &msg.sender, "", &[]);
                false
            }
            (PROPS_IFACE, "Get") => {
                self.handle_prop_get(&msg);
                false
            }
            (PROPS_IFACE, "GetAll") => {
                let body = self.encode_watcher_props();
                self.conn.send_reply(msg.serial, &msg.sender, "a{sv}", &body);
                false
            }
            ("org.freedesktop.DBus.Introspectable", "Introspect") => {
                let mut body = Vec::new();
                dbus::encode_string(&mut body, WATCHER_INTROSPECT);
                self.conn.send_reply(msg.serial, &msg.sender, "s", &body);
                false
            }
            ("org.freedesktop.DBus.Peer", "Ping") => {
                self.conn.send_reply(msg.serial, &msg.sender, "", &[]);
                false
            }
            _ => {
                if !msg.sender.is_empty() {
                    self.conn.send_error(
                        msg.serial,
                        &msg.sender,
                        "org.freedesktop.DBus.Error.UnknownMethod",
                        "unknown method",
                    );
                }
                false
            }
        }
    }

    fn handle_prop_get(&mut self, msg: &Message) {
        let mut reader = BodyReader::new(&msg.body, &msg.signature);
        let _iface = reader.read_value("s");
        let prop = reader
            .read_value("s")
            .and_then(|v| v.as_str().map(String::from))
            .unwrap_or_default();
        let mut body = Vec::new();
        match prop.as_str() {
            "RegisteredStatusNotifierItems" => self.encode_items_variant(&mut body),
            "IsStatusNotifierHostRegistered" => dbus::encode_variant_bool(&mut body, true),
            "ProtocolVersion" => {
                dbus::encode_signature(&mut body, "i");
                dbus::align_to(&mut body, 4);
                dbus::encode_i32(&mut body, 0);
            }
            _ => {
                self.conn.send_error(
                    msg.serial,
                    &msg.sender,
                    "org.freedesktop.DBus.Error.InvalidArgs",
                    "no such property",
                );
                return;
            }
        }
        self.conn.send_reply(msg.serial, &msg.sender, "v", &body);
    }

    /// `a{sv}` with all three watcher properties.
    fn encode_watcher_props(&self) -> Vec<u8> {
        let mut body = Vec::new();
        dbus::align_to(&mut body, 4);
        let len_pos = body.len();
        dbus::encode_u32(&mut body, 0);
        dbus::align_to(&mut body, 8);
        let start = body.len();

        dbus::align_to(&mut body, 8);
        dbus::encode_string(&mut body, "RegisteredStatusNotifierItems");
        self.encode_items_variant(&mut body);

        dbus::align_to(&mut body, 8);
        dbus::encode_string(&mut body, "IsStatusNotifierHostRegistered");
        dbus::encode_variant_bool(&mut body, true);

        dbus::align_to(&mut body, 8);
        dbus::encode_string(&mut body, "ProtocolVersion");
        dbus::encode_signature(&mut body, "i");
        dbus::align_to(&mut body, 4);
        dbus::encode_i32(&mut body, 0);

        let len = (body.len() - start) as u32;
        body[len_pos..len_pos + 4].copy_from_slice(&len.to_le_bytes());
        body
    }

    /// Variant holding `as` of `bus/path` strings.
    fn encode_items_variant(&self, body: &mut Vec<u8>) {
        dbus::encode_signature(body, "as");
        dbus::align_to(body, 4);
        let len_pos = body.len();
        dbus::encode_u32(body, 0);
        let start = body.len();
        for item in &self.items {
            dbus::encode_string(body, &format!("{}{}", item.bus_name, item.obj_path));
        }
        let len = (body.len() - start) as u32;
        body[len_pos..len_pos + 4].copy_from_slice(&len.to_le_bytes());
    }

    fn handle_reply(&mut self, msg: Message) -> bool {
        let Some(call) = self.pending.remove(&msg.reply_serial) else {
            return false;
        };
        let is_error = msg.msg_type == 3;
        match call {
            PendingCall::GetAllProps { bus } => {
                if is_error {
                    tracing::info!(bus = %bus, "GetAll failed — dropping tray item");
                    let before = self.items.len();
                    self.items.retain(|i| i.bus_name != bus);
                    return self.items.len() != before;
                }
                let mut reader = BodyReader::new(&msg.body, &msg.signature);
                match reader.read_value("a{sv}") {
                    Some(Value::Dict(props)) => self.apply_props(&bus, &props),
                    _ => false,
                }
            }
            PendingCall::AboutToShow { bus, menu_path } => {
                // Whether or not the app needed the hint (or errored),
                // fetch the layout now.
                self.get_menu_layout(&bus, &menu_path);
                false
            }
            PendingCall::GetMenuLayout { bus, menu_path } => {
                if is_error {
                    tracing::warn!(bus = %bus, "GetLayout failed");
                    return false;
                }
                let mut reader = BodyReader::new(&msg.body, &msg.signature);
                let _revision = reader.read_value("u");
                if let Some(root) = reader.read_value("(ia{sv}av)") {
                    if let Some(root_item) = dbusmenu::parse_layout(&root) {
                        tracing::debug!(
                            bus = %bus,
                            children = root_item.children.len(),
                            "dbusmenu layout received"
                        );
                        self.events.push(HostEvent::MenuReady {
                            bus,
                            menu_path,
                            items: root_item.children,
                        });
                    }
                }
                false
            }
            PendingCall::Activate { bus } => {
                if is_error {
                    // Menu-only item: treat the click like a right-click.
                    tracing::debug!(bus = %bus, "Activate rejected → falling back to menu");
                    self.request_menu(&bus, 0, 0);
                }
                false
            }
        }
    }

    fn handle_signal(&mut self, msg: Message) -> bool {
        match msg.member.as_str() {
            "NameOwnerChanged" => {
                let mut reader = BodyReader::new(&msg.body, &msg.signature);
                let name = reader
                    .read_value("s")
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_default();
                let _old = reader.read_value("s");
                let new_owner = reader
                    .read_value("s")
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_default();
                if new_owner.is_empty() && !name.is_empty() {
                    let before = self.items.len();
                    self.items.retain(|i| i.bus_name != name);
                    if self.items.len() != before {
                        tracing::info!(name = %name, "tray item's owner left the bus");
                        let mut sig_body = Vec::new();
                        dbus::encode_string(&mut sig_body, &name);
                        self.conn.send_signal(
                            WATCHER_PATH,
                            WATCHER_IFACE,
                            "StatusNotifierItemUnregistered",
                            "s",
                            &sig_body,
                        );
                        return true;
                    }
                }
                false
            }
            "NewIcon" | "NewAttentionIcon" | "NewOverlayIcon" | "NewStatus" | "NewTitle"
            | "NewToolTip" | "NewMenu" => {
                if let Some(item) = self.items.iter().find(|i| i.bus_name == msg.sender) {
                    let bus = item.bus_name.clone();
                    let path = item.obj_path.clone();
                    self.request_item_props(&bus, &path);
                }
                false
            }
            _ => false,
        }
    }

    fn apply_props(&mut self, bus: &str, props: &HashMap<String, Value>) -> bool {
        let Some(item) = self.items.iter_mut().find(|i| i.bus_name == bus) else {
            return false;
        };
        let before = item.visual_key();

        let str_prop = |k: &str| props.get(k).and_then(|v| v.as_str()).map(String::from);
        if let Some(v) = str_prop("Id") {
            item.id = v;
        }
        if let Some(v) = str_prop("Title") {
            item.title = v;
        }
        if let Some(v) = str_prop("Status") {
            item.status = v;
        }
        if let Some(v) = str_prop("IconName") {
            item.icon_name = v;
        }
        if let Some(v) = str_prop("AttentionIconName") {
            item.attention_icon_name = v;
        }
        if let Some(v) = str_prop("IconThemePath") {
            item.icon_theme_path = v;
        }
        if let Some(v) = props.get("ItemIsMenu").and_then(|v| v.as_bool()) {
            item.item_is_menu = v;
        }
        if let Some(Value::Array(pixmaps)) = props.get("IconPixmap") {
            item.icon_pixmap = pick_best_pixmap(pixmaps, PIXMAP_TARGET);
        }
        if let Some(Value::Array(pixmaps)) = props.get("AttentionIconPixmap") {
            item.attention_pixmap = pick_best_pixmap(pixmaps, PIXMAP_TARGET);
        }
        if let Some(v) = props.get("Menu") {
            let path = v.as_str().unwrap_or("").to_string();
            item.menu_path = (!path.is_empty() && path != "/").then_some(path);
        }
        // ToolTip is (sv(iiay)ss): icon name, pixmaps, title, body.
        if let Some(Value::Struct(fields)) = props.get("ToolTip") {
            if let Some(title) = fields.get(2).and_then(|v| v.as_str()) {
                if !title.is_empty() {
                    item.tooltip = title.to_string();
                }
            }
        }
        // Ayatana/appindicator items carry the app's human name here.
        if item.title.is_empty() {
            item.title = item.id.clone();
        }

        let changed = item.visual_key() != before;
        tracing::info!(
            id = %item.id,
            title = %item.title,
            status = %item.status,
            icon = %item.icon_name,
            theme_path = %item.icon_theme_path,
            has_pixmap = item.icon_pixmap.is_some(),
            has_menu = item.menu_path.is_some(),
            "tray item updated"
        );
        changed
    }
}

// ── Helpers ─────────────────────────────────────────────────────────────────

/// The registered service string can be just an object path (sender is
/// the bus name), `bus_name/obj_path`, or a bare bus name.
fn parse_item_name(service: &str, sender: &str) -> (String, String) {
    if service.starts_with('/') {
        (sender.to_string(), service.to_string())
    } else if let Some(idx) = service.find('/') {
        (service[..idx].to_string(), service[idx..].to_string())
    } else if service.is_empty() {
        (sender.to_string(), "/StatusNotifierItem".to_string())
    } else {
        (service.to_string(), "/StatusNotifierItem".to_string())
    }
}

/// Pick the pixmap closest to `target` and convert from SNI's
/// network-byte-order ARGB to RGBA.
fn pick_best_pixmap(pixmaps: &[Value], target: u32) -> Option<IconPixmap> {
    let mut best: Option<(u32, u32, &[u8])> = None;
    let mut best_dist = u32::MAX;
    for pm in pixmaps {
        let Value::Struct(fields) = pm else { continue };
        if fields.len() < 3 {
            continue;
        }
        let w = fields[0].as_i32().unwrap_or(0).max(0) as u32;
        let h = fields[1].as_i32().unwrap_or(0).max(0) as u32;
        let Some(data) = fields[2].as_bytes() else {
            continue;
        };
        if w == 0 || h == 0 {
            continue;
        }
        let dist = w.abs_diff(target);
        if dist < best_dist {
            best = Some((w, h, data));
            best_dist = dist;
        }
    }
    let (w, h, argb) = best?;
    let expected = (w as usize) * (h as usize) * 4;
    if argb.len() < expected {
        return None;
    }
    let mut rgba = vec![0u8; expected];
    for (dst, src) in rgba.chunks_exact_mut(4).zip(argb.chunks_exact(4)) {
        dst[0] = src[1];
        dst[1] = src[2];
        dst[2] = src[3];
        dst[3] = src[0];
    }
    Some(IconPixmap {
        width: w,
        height: h,
        rgba,
    })
}

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
    <signal name="StatusNotifierHostRegistered"/>
    <property name="RegisteredStatusNotifierItems" type="as" access="read"/>
    <property name="IsStatusNotifierHostRegistered" type="b" access="read"/>
    <property name="ProtocolVersion" type="i" access="read"/>
  </interface>
</node>"#;

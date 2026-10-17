//! What an app's tray item says of itself (`org.kde.StatusNotifierItem`'s
//! properties), read into the [`AppItem`] the board shows: its name, its
//! status, its picture (a name to look up, or pixels it sends), whether it
//! has a menu; and where on the bus a registration says it is.

use std::collections::HashMap;
use std::sync::Arc;

use lntrn_dbus::Value;

use super::{AppItem, Icon, Pixmap, Status};

/// The pixels picked from an item's sizes: the nearest this wide.
const PIXMAP_WANTED: u32 = 48;

/// Where an item is, from what it registered (`service`) and who
/// registered it (`sender`): an object path alone, `name/path`, or a bare
/// bus name at the usual path.
pub(super) fn place(service: &str, sender: &str) -> (String, String) {
    const PATH: &str = "/StatusNotifierItem";
    match service.find('/') {
        Some(0) => (sender.to_owned(), service.to_owned()),
        Some(at) => (service[..at].to_owned(), service[at..].to_owned()),
        None if service.is_empty() => (sender.to_owned(), PATH.to_owned()),
        None => (service.to_owned(), PATH.to_owned()),
    }
}

/// What the item's properties say, onto `item`; its menu's path, when it
/// has one.
pub(super) fn apply(item: &mut AppItem, props: &HashMap<String, Value>) -> Option<String> {
    let text = |k: &str| props.get(k).and_then(Value::as_str).map(str::to_owned);
    let id = text("Id").unwrap_or_default();
    item.title = text("Title").filter(|t| !t.is_empty()).unwrap_or(id);
    item.status = match text("Status").as_deref() {
        Some("Passive") => Status::Passive,
        Some("NeedsAttention") => Status::NeedsAttention,
        _ => Status::Active,
    };
    let theme = text("IconThemePath").unwrap_or_default();
    let named = |key: &str| text(key).filter(|n| !n.is_empty()).map(|name| Icon::Named { name, theme: theme.clone() });
    let pixels = |key: &str| props.get(key).and_then(Value::as_array).and_then(best).map(Icon::Pixmap);
    let normal = named("IconName").or_else(|| pixels("IconPixmap"));
    let attention = named("AttentionIconName").or_else(|| pixels("AttentionIconPixmap"));
    item.icon = match item.status {
        Status::NeedsAttention => attention.or(normal),
        _ => normal,
    }
    .unwrap_or(Icon::None);
    // (sa(iiay)ss): its icon, its pixels, a title, words under it.
    let tip = props.get("ToolTip").and_then(|t| match t {
        Value::Struct(f) => Some((f.get(2)?.as_str()?.to_owned(), f.get(3).and_then(Value::as_str).unwrap_or_default().to_owned())),
        _ => None,
    });
    item.tooltip = match tip {
        Some((title, body)) if !title.is_empty() && !body.is_empty() => format!("{title}: {}", plain(&body)),
        Some((title, _)) if !title.is_empty() => title,
        _ => item.title.clone(),
    };
    item.item_is_menu = props.get("ItemIsMenu").and_then(Value::as_bool).unwrap_or(false);
    let menu = text("Menu").filter(|m| !m.is_empty() && m != "/");
    item.has_menu = menu.is_some();
    menu
}

/// A tooltip's words without the markup some apps put in them.
fn plain(markup: &str) -> String {
    let mut out = String::with_capacity(markup.len());
    let mut in_tag = false;
    for c in markup.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            c if !in_tag => out.push(c),
            _ => {}
        }
    }
    out.replace("&amp;", "&").replace("&lt;", "<").replace("&gt;", ">").split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The size nearest [`PIXMAP_WANTED`] of an item's `a(iiay)`, from SNI's
/// ARGB (network order) to RGBA.
fn best(pixmaps: &[Value]) -> Option<Pixmap> {
    let sizes = pixmaps.iter().filter_map(|p| {
        let Value::Struct(f) = p else { return None };
        let (w, h) = (u32::try_from(f.first()?.as_i32()?).ok()?, u32::try_from(f.get(1)?.as_i32()?).ok()?);
        let argb = f.get(2)?.as_bytes()?;
        (w > 0 && h > 0 && argb.len() >= (w * h * 4) as usize).then_some((w, h, argb))
    });
    let (w, h, argb) = sizes.min_by_key(|(w, _, _)| w.abs_diff(PIXMAP_WANTED))?;
    let rgba: Vec<u8> = argb[..(w * h * 4) as usize].chunks_exact(4).flat_map(|p| [p[1], p[2], p[3], p[0]]).collect();
    // A cheap stamp of what it shows: a dot changing colour is a new one.
    let stamp = rgba.iter().step_by(7).fold(0xcbf2_9ce4_8422_2325_u64, |h, b| (h ^ u64::from(*b)).wrapping_mul(0x100_0000_01b3));
    Some(Pixmap { width: w, height: h, rgba: Arc::from(rgba), stamp })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn props(pairs: &[(&str, Value)]) -> HashMap<String, Value> {
        pairs.iter().map(|(k, v)| ((*k).to_owned(), v.clone())).collect()
    }

    fn s(v: &str) -> Value {
        Value::String(v.to_owned())
    }

    /// Against this session's tray: the items registered with whoever is
    /// the watcher now, and what their properties read as. Changes nothing.
    #[test]
    #[ignore = "talks to the session bus and its tray"]
    fn the_items_on_this_desktop_read() {
        use crate::bus::Bus;
        let mut bus = Bus::session().unwrap();
        let mut body = Vec::new();
        lntrn_dbus::encode_string(&mut body, "org.kde.StatusNotifierWatcher");
        lntrn_dbus::encode_string(&mut body, "RegisteredStatusNotifierItems");
        let Ok(reply) = bus.call("org.kde.StatusNotifierWatcher", "/StatusNotifierWatcher", "org.freedesktop.DBus.Properties", "Get", "ss", &body) else {
            println!("no tray on this session");
            return;
        };
        let owner = bus.owner("org.kde.StatusNotifierWatcher").unwrap_or_default();
        let mut name = Vec::new();
        lntrn_dbus::encode_string(&mut name, &owner);
        let pid = bus.call("org.freedesktop.DBus", "/org/freedesktop/DBus", "org.freedesktop.DBus", "GetConnectionUnixProcessID", "s", &name).ok().and_then(|r| r.first().and_then(Value::as_u32));
        let comm = pid.and_then(|p| std::fs::read_to_string(format!("/proc/{p}/comm")).ok());
        println!("watcher: {owner} pid {pid:?} {comm:?}");
        for service in reply.first().and_then(Value::as_array).unwrap_or_default().iter().filter_map(Value::as_str) {
            let (dest, path) = place(service, "");
            let mut body = Vec::new();
            lntrn_dbus::encode_string(&mut body, "org.kde.StatusNotifierItem");
            let props = bus.call(&dest, &path, "org.freedesktop.DBus.Properties", "GetAll", "s", &body).unwrap_or_default();
            let mut item = AppItem::default();
            let menu = props.first().and_then(Value::as_dict).and_then(|p| apply(&mut item, p));
            let icon = match &item.icon {
                Icon::Pixmap(p) => format!("{}x{} pixels", p.width, p.height),
                other => format!("{other:?}"),
            };
            println!("{service}: {:?} {:?} tip={:?} icon={icon} menu={menu:?}", item.title, item.status, item.tooltip);
        }
    }

    #[test]
    fn a_registration_says_where_the_item_is() {
        assert_eq!(place("/org/ayatana/NotificationItem/steam", ":1.40"), (":1.40".into(), "/org/ayatana/NotificationItem/steam".into()));
        assert_eq!(place("org.kde.StatusNotifierItem-4242-1", ":1.40"), ("org.kde.StatusNotifierItem-4242-1".into(), "/StatusNotifierItem".into()));
        assert_eq!(place(":1.9/StatusNotifierItem", ":1.40"), (":1.9".into(), "/StatusNotifierItem".into()));
    }

    #[test]
    fn its_properties_read_as_its_name_status_picture_and_menu() {
        let mut item = AppItem::default();
        let tip = Value::Struct(vec![s(""), Value::Array(vec![]), s("Steam"), s("<b>3</b> friends &amp; online")]);
        let menu = apply(&mut item, &props(&[("Id", s("steam")), ("Status", s("NeedsAttention")), ("IconName", s("steam_tray_mono")), ("AttentionIconName", s("steam_tray_mono_alert")), ("IconThemePath", s("/opt/steam/public")), ("ToolTip", tip), ("Menu", Value::ObjectPath("/MenuBar".into()))]));
        assert_eq!((item.title.as_str(), item.status, menu.as_deref(), item.has_menu), ("steam", Status::NeedsAttention, Some("/MenuBar"), true));
        assert_eq!(item.icon, Icon::Named { name: "steam_tray_mono_alert".into(), theme: "/opt/steam/public".into() }, "needing attention, its attention picture");
        assert_eq!(item.tooltip, "Steam: 3 friends & online");
        let none = apply(&mut item, &props(&[("Title", s("Discord")), ("Menu", Value::ObjectPath("/".into()))]));
        assert_eq!((none, item.has_menu, item.status, item.tooltip.as_str()), (None, false, Status::Active, "Discord"));
    }

    #[test]
    fn pixels_come_in_the_nearest_size_as_rgba() {
        let pixmap = |w: i32, px: [u8; 4]| Value::Struct(vec![Value::Int32(w), Value::Int32(w), Value::Bytes(px.repeat((w * w) as usize))]);
        let mut item = AppItem::default();
        apply(&mut item, &props(&[("IconPixmap", Value::Array(vec![pixmap(16, [255, 1, 2, 3]), pixmap(44, [128, 10, 20, 30]), pixmap(128, [255, 0, 0, 0])]))]));
        let Icon::Pixmap(p) = &item.icon else { panic!("{:?}", item.icon) };
        assert_eq!((p.width, &p.rgba[..4]), (44, &[10, 20, 30, 128][..]));
    }
}

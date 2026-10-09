//! `[[monitors]]`: a table a monitor, found by its name. The keys below
//! are this app's; whatever else an entry has (`hdr`, `sdr_brightness`)
//! is the compositor's and stays as it is. An entry is written key by
//! key and only where it differs from the file, so one nobody changed
//! comes through a save untouched, down to how its numbers are written.

use lntrn_data::{Doc, Map};

#[derive(Clone, Debug, PartialEq)]
pub struct Monitor {
    /// What the compositor calls it: `DP-1`.
    pub name: String,
    /// The wallpaper it shows instead of the global one (empty: the
    /// global one).
    pub wallpaper: String,
    /// Its top-left corner on the desktop, in logical pixels.
    pub x: i64,
    pub y: i64,
    /// `None`: the entry doesn't say, and the compositor goes by
    /// `[display] scale`.
    pub scale: Option<f64>,
    /// The mode it was last kept in, in the words the file has always
    /// had them in: `3840x2160`, and the refresh in millihertz. Empty:
    /// not said.
    pub resolution: String,
    pub refresh_rate: String,
    /// The main monitor: where the Command Center lives.
    pub primary: bool,
    /// A fullscreen app may drive its refresh.
    pub vrr: bool,
    /// Off, it is plugged in but no part of the desktop.
    pub enabled: bool,
}

impl Default for Monitor {
    fn default() -> Self {
        Self { name: String::new(), wallpaper: String::new(), x: 0, y: 0, scale: None, resolution: String::new(), refresh_rate: String::new(), primary: false, vrr: false, enabled: true }
    }
}

/// The `[[monitors]]` entries of `doc`, in the file's order. A key an
/// entry leaves out is what the compositor takes it to be.
pub(super) fn read(doc: &Doc) -> Vec<Monitor> {
    let Some(list) = doc.get("monitors").and_then(Doc::as_list) else { return Vec::new() };
    list.iter()
        .filter_map(|m| {
            let d = Monitor::default();
            let word = |key: &str| m.get(key).and_then(Doc::as_str).unwrap_or("").to_owned();
            let flag = |key: &str, default: bool| m.get(key).and_then(Doc::as_bool).unwrap_or(default);
            Some(Monitor {
                name: m.get("name")?.as_str()?.to_owned(),
                wallpaper: word("wallpaper"),
                x: m.get("x").and_then(Doc::as_i64).unwrap_or(d.x),
                y: m.get("y").and_then(Doc::as_i64).unwrap_or(d.y),
                scale: m.get("scale").and_then(Doc::as_f64),
                resolution: word("resolution"),
                refresh_rate: word("refresh_rate"),
                primary: flag("primary", d.primary),
                vrr: flag("vrr", d.vrr),
                enabled: flag("enabled", d.enabled),
            })
        })
        .collect()
}

/// Put `ours` under `key` unless the entry says the same already. A key
/// the entry leaves out says `unsaid`.
fn put(entry: &mut Map, key: &str, ours: Doc, unsaid: &Doc) {
    if entry.get(key).unwrap_or(unsaid) != &ours {
        entry.insert(key, ours);
    }
}

/// Our keys into one entry. The name is not ours to change.
fn fill(entry: &mut Map, ours: &Monitor) {
    let d = Monitor::default();
    // No override is no key.
    if ours.wallpaper.is_empty() {
        entry.remove("wallpaper");
    } else {
        put(entry, "wallpaper", Doc::Str(ours.wallpaper.clone()), &Doc::Null);
    }
    // A number may be written whole or with a point: `2` is `2.0`.
    if entry.get("x").and_then(Doc::as_i64).unwrap_or(d.x) != ours.x {
        entry.insert("x", Doc::Int(ours.x));
    }
    if entry.get("y").and_then(Doc::as_i64).unwrap_or(d.y) != ours.y {
        entry.insert("y", Doc::Int(ours.y));
    }
    if let Some(scale) = ours.scale
        && entry.get("scale").and_then(Doc::as_f64) != Some(scale)
    {
        entry.insert("scale", Doc::Float(scale));
    }
    for (key, word) in [("resolution", &ours.resolution), ("refresh_rate", &ours.refresh_rate)] {
        if !word.is_empty() {
            put(entry, key, Doc::Str(word.clone()), &Doc::Null);
        }
    }
    put(entry, "primary", Doc::Bool(ours.primary), &Doc::Bool(d.primary));
    put(entry, "vrr", Doc::Bool(ours.vrr), &Doc::Bool(d.vrr));
    put(entry, "enabled", Doc::Bool(ours.enabled), &Doc::Bool(d.enabled));
}

/// Put what we hold of each monitor into its entry. A monitor the file
/// has no entry for gets one at the end; an entry we hold nothing for is
/// left alone.
pub(super) fn write(doc: &mut Doc, monitors: &[Monitor]) {
    if monitors.is_empty() {
        return;
    }
    let Some(root) = doc.as_map_mut() else { return };
    if !matches!(root.get("monitors"), Some(Doc::List(_))) {
        root.insert("monitors", Doc::List(Vec::new()));
    }
    let Some(Doc::List(entries)) = root.get_mut("monitors") else { return };
    for ours in monitors {
        let found = entries.iter_mut().filter_map(Doc::as_map_mut).find(|m| m.get("name").and_then(Doc::as_str) == Some(ours.name.as_str()));
        match found {
            Some(entry) => fill(entry, ours),
            None => {
                let mut entry = Map::new();
                entry.insert("name", Doc::Str(ours.name.clone()));
                fill(&mut entry, ours);
                entries.push(Doc::Map(entry));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lntrn_data::toml;

    const FILE: &str = "[appearance]\naccent = \"#FFC800\"\n\n[[monitors]]\nname = \"eDP-1\"\nscale = 1.5\n\n[[monitors]]\nname = \"DP-2\"\nx = 1280\nwallpaper = \"/old.png\"\nscale = 1.399999976158142\nvrr = true\nhdr = true\nsdr_brightness = 203\n";

    fn through(doc: &Doc) -> Doc {
        toml::parse(&toml::write(doc)).unwrap()
    }

    /// A wallpaper override goes into its own entry and comes out again,
    /// and the rest of the entry (and what is the compositor's) stays.
    #[test]
    fn a_wallpaper_goes_into_its_monitors_entry() {
        let mut doc = toml::parse(FILE).unwrap();
        let mut monitors = read(&doc);
        assert_eq!(monitors.iter().map(|m| (m.name.as_str(), m.wallpaper.as_str())).collect::<Vec<_>>(), [("eDP-1", ""), ("DP-2", "/old.png")]);
        monitors[0].wallpaper = "/new.jpg".into();
        monitors[1].wallpaper.clear();
        write(&mut doc, &monitors);
        let doc = through(&doc);
        assert_eq!(read(&doc), monitors);
        let list = doc.get("monitors").and_then(Doc::as_list).unwrap();
        assert_eq!(list[0].get("scale").and_then(Doc::as_f64), Some(1.5));
        assert_eq!((list[1].get("vrr").and_then(Doc::as_bool), list[1].get("hdr").and_then(Doc::as_bool), list[1].get("sdr_brightness").and_then(Doc::as_i64)), (Some(true), Some(true), Some(203)));
        assert!(list[1].get("wallpaper").is_none(), "no override is no key");
        assert_eq!(doc.path("appearance.accent").and_then(Doc::as_str), Some("#FFC800"));
    }

    /// What an entry leaves out is read as the compositor takes it, and a
    /// save with nothing changed writes the file back as it was: no key
    /// is added for saying what was already so.
    #[test]
    fn an_entry_nobody_changed_is_not_touched() {
        let mut doc = toml::parse(FILE).unwrap();
        let before = doc.clone();
        let monitors = read(&doc);
        let edp = &monitors[0];
        assert_eq!((edp.x, edp.y, edp.scale, edp.primary, edp.vrr, edp.enabled), (0, 0, Some(1.5), false, false, true));
        assert_eq!((monitors[1].x, monitors[1].scale, monitors[1].vrr), (1280, Some(1.399999976158142), true));
        assert!(edp.resolution.is_empty() && edp.refresh_rate.is_empty());
        write(&mut doc, &monitors);
        assert_eq!(doc, before);
        assert_eq!(toml::write(&doc), toml::write(&before));
    }

    /// The keys a kept setup changes are written, each as the compositor
    /// reads it, and a monitor the file has never heard of gets an entry.
    #[test]
    fn a_kept_setup_is_written_key_by_key() {
        let mut doc = toml::parse(FILE).unwrap();
        let mut monitors = read(&doc);
        monitors[1] = Monitor { x: 2560, y: 40, scale: Some(1.4), resolution: "3840x2160".into(), refresh_rate: "240000".into(), primary: true, vrr: false, enabled: false, ..monitors[1].clone() };
        monitors.push(Monitor { name: "HDMI-A-1".into(), x: 0, scale: Some(1.0), resolution: "1920x1080".into(), refresh_rate: "60000".into(), ..Monitor::default() });
        write(&mut doc, &monitors);
        let doc = through(&doc);
        assert_eq!(read(&doc), monitors);
        let list = doc.get("monitors").and_then(Doc::as_list).unwrap();
        assert_eq!(list.len(), 3);
        let dp = &list[1];
        assert_eq!((dp.get("x"), dp.get("y"), dp.get("scale")), (Some(&Doc::Int(2560)), Some(&Doc::Int(40)), Some(&Doc::Float(1.4))));
        assert_eq!((dp.get("resolution").and_then(Doc::as_str), dp.get("refresh_rate").and_then(Doc::as_str)), (Some("3840x2160"), Some("240000")));
        assert_eq!((dp.get("primary"), dp.get("vrr"), dp.get("enabled")), (Some(&Doc::Bool(true)), Some(&Doc::Bool(false)), Some(&Doc::Bool(false))));
        assert_eq!(dp.get("hdr").and_then(Doc::as_bool), Some(true), "the compositor's keys stay");
        // The first entry was not ours to touch: no key was added to it.
        assert_eq!(list[0].as_map().map(Map::len), Some(2));
        // The new one says its name and what isn't the default.
        let new = &list[2];
        assert_eq!((new.get("name").and_then(Doc::as_str), new.get("scale").and_then(Doc::as_f64), new.get("x")), (Some("HDMI-A-1"), Some(1.0), None));
        // The compositor reads these lines with a parser of its own.
        let text = toml::write(&doc);
        assert!(text.contains("x = 2560\n") && text.contains("scale = 1.4\n") && text.contains("enabled = false\n") && text.contains("refresh_rate = \"240000\"\n"), "{text}");
    }

    /// A file with no monitors gets its list when there is one to write,
    /// and not before.
    #[test]
    fn the_list_is_made_when_it_is_needed() {
        let mut doc = toml::parse("[appearance]\naccent = \"#FFC800\"\n").unwrap();
        write(&mut doc, &[]);
        assert!(doc.get("monitors").is_none());
        write(&mut doc, &[Monitor { name: "DP-1".into(), scale: Some(1.25), ..Monitor::default() }]);
        let doc = through(&doc);
        assert_eq!(read(&doc), vec![Monitor { name: "DP-1".into(), scale: Some(1.25), ..Monitor::default() }]);
    }
}

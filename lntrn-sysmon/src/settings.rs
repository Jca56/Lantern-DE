//! The monitor's own settings: the `[sysmon]` section of `lantern.toml`.
//! They are changed from its menu and its Processes page, and written
//! back as they change.

use lntrn_data::{Doc, Map};
use lntrn_kit::desktop;

const SECTION: &str = "sysmon";

/// How often the machine can be looked at: seconds between looks, and
/// what the menu calls that.
pub const SPEEDS: [(f64, &str); 4] = [(0.5, "Fast"), (1.0, "Normal"), (2.0, "Slow"), (5.0, "Very Slow")];

#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    /// Seconds between looks: one of [`SPEEDS`].
    pub interval: f64,
    /// Fold a program's processes into one row.
    pub grouped: bool,
    /// List the kernel's own threads among the processes.
    pub kernel: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self { interval: 1.0, grouped: true, kernel: false }
    }
}

/// The speed nearest `seconds`: whatever the file says, what is used is
/// one the menu can show as picked.
pub fn nearest_speed(seconds: f64) -> f64 {
    SPEEDS.iter().map(|s| s.0).min_by(|a, b| (a - seconds).abs().total_cmp(&(b - seconds).abs())).unwrap_or(1.0)
}

impl Settings {
    /// What a `[sysmon]` table says, the defaults for what it doesn't.
    fn from_table(t: &Doc) -> Settings {
        let d = Settings::default();
        Settings {
            interval: t.get("interval").and_then(Doc::as_f64).filter(|s| s.is_finite()).map_or(d.interval, nearest_speed),
            grouped: t.get("group_by_program").and_then(Doc::as_bool).unwrap_or(d.grouped),
            kernel: t.get("show_kernel_threads").and_then(Doc::as_bool).unwrap_or(d.kernel),
        }
    }

    pub fn load() -> Settings {
        desktop::read_doc().get(SECTION).map_or_else(Settings::default, Self::from_table)
    }

    fn write(&self, t: &mut Map) {
        t.insert("interval", Doc::Float(self.interval));
        t.insert("group_by_program", Doc::Bool(self.grouped));
        t.insert("show_kernel_threads", Doc::Bool(self.kernel));
    }

    /// Write them to `lantern.toml`, leaving the rest of it as it was.
    pub fn save(&self) -> Result<(), String> {
        desktop::update(SECTION, |t| self.write(t))
    }
}

#[cfg(test)]
mod tests {
    use lntrn_data::toml;

    use super::*;

    #[test]
    fn settings_survive_the_file_and_nonsense_is_put_right() {
        let s = Settings { interval: 2.0, grouped: false, kernel: true };
        let mut doc = Doc::map();
        let mut table = Map::new();
        s.write(&mut table);
        doc.set(SECTION, Doc::Map(table));
        let back = toml::parse(&toml::write(&doc)).unwrap();
        assert_eq!(Settings::from_table(back.get(SECTION).unwrap()), s);

        let odd = toml::parse("[sysmon]\ninterval = 0.0001\ngroup_by_program = \"yes\"\n").unwrap();
        assert_eq!(Settings::from_table(odd.get(SECTION).unwrap()), Settings { interval: 0.5, ..Settings::default() });
        assert_eq!(Settings::from_table(&Doc::map()), Settings::default());
        assert_eq!((nearest_speed(1.4), nearest_speed(900.0), nearest_speed(-3.0)), (1.0, 5.0, 0.5));
    }
}

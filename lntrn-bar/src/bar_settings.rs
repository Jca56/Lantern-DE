//! Persist bar settings + pinned sysmon processes to ~/.lantern/config/bar/settings.toml

use std::path::PathBuf;

fn settings_path() -> PathBuf {
    crate::bar_config_dir().join("settings.toml")
}

pub struct BarSettings {
    pub floating: bool,
    pub auto_hide: bool,
    pub height: u32,
    pub opacity: f32,
    pub lava_lamp: bool,
    pub position_top: bool,
    pub pinned_procs: Vec<String>,
}

impl Default for BarSettings {
    fn default() -> Self {
        Self {
            floating: true,
            auto_hide: false,
            height: crate::layershell::BAR_HEIGHT_DEFAULT,
            opacity: 1.0,
            lava_lamp: false,
            position_top: false,
            pinned_procs: Vec::new(),
        }
    }
}

impl BarSettings {
    pub fn load() -> Self {
        let path = settings_path();
        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => return Self::default(),
        };

        let mut s = Self::default();
        for line in content.lines() {
            let line = line.trim();
            if line.starts_with('#') || line.is_empty() { continue; }
            if let Some((key, val)) = line.split_once('=') {
                let key = key.trim();
                let val = val.trim();
                match key {
                    "floating" => s.floating = val == "true",
                    "auto_hide" => s.auto_hide = val == "true",
                    "height" => s.height = val.parse().unwrap_or(s.height),
                    "opacity" => s.opacity = val.parse().unwrap_or(s.opacity),
                    "lava_lamp" => s.lava_lamp = val == "true",
                    "position_top" => s.position_top = val == "true",
                    "pinned_procs" => {
                        if let Some(start) = val.find('[') {
                            if let Some(end) = val.find(']') {
                                for item in val[start + 1..end].split(',') {
                                    let item = item.trim().trim_matches('"');
                                    if !item.is_empty() {
                                        s.pinned_procs.push(item.to_string());
                                    }
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        s
    }

    pub fn save(&self) {
        let procs: Vec<String> = self.pinned_procs.iter()
            .map(|p| format!("\"{}\"", p))
            .collect();
        let content = format!(
            "# Bar settings\n\
             floating = {}\n\
             auto_hide = {}\n\
             height = {}\n\
             opacity = {:.2}\n\
             lava_lamp = {}\n\
             position_top = {}\n\
             pinned_procs = [{}]\n",
            self.floating, self.auto_hide, self.height,
            self.opacity, self.lava_lamp, self.position_top,
            procs.join(", "),
        );
        let _ = std::fs::create_dir_all(crate::bar_config_dir());
        let _ = std::fs::write(settings_path(), content);
    }
}

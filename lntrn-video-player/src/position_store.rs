use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct PositionStore {
    positions: HashMap<String, i64>,
}

impl PositionStore {
    fn data_path() -> Option<PathBuf> {
        let dir = dirs::data_local_dir()?.join("lntrn-video-player");
        fs::create_dir_all(&dir).ok()?;
        Some(dir.join("positions.json"))
    }

    pub fn load() -> Self {
        Self::data_path()
            .and_then(|path| fs::read_to_string(&path).ok())
            .and_then(|data| serde_json::from_str(&data).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        if let Some(path) = Self::data_path() {
            if let Ok(data) = serde_json::to_string_pretty(self) {
                let _ = fs::write(&path, data);
            }
        }
    }

    pub fn get_position(&self, uri: &str) -> Option<i64> {
        self.positions.get(uri).copied()
    }

    pub fn set_position(&mut self, uri: &str, timestamp: i64) {
        if timestamp > 1_000_000 {
            self.positions.insert(uri.to_string(), timestamp);
        } else {
            self.positions.remove(uri);
        }
    }

    pub fn clear_position(&mut self, uri: &str) {
        self.positions.remove(uri);
    }
}

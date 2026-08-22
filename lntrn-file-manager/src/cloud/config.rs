// Reads ~/.lantern/config/fox-cloud.json. The file holds Firebase Web config
// (api_key, project_id, storage_bucket). The Web API key is not secret per
// Firebase's docs — security comes from Firestore + Storage rules — but we
// keep it out of the repo as a matter of hygiene.

use serde::Deserialize;
use std::path::PathBuf;

#[derive(Debug, Deserialize, Clone)]
pub struct CloudConfig {
    pub api_key: String,
    pub project_id: String,
    pub storage_bucket: String,
    #[serde(default)]
    pub auth_domain: String,
}

impl CloudConfig {
    pub fn path() -> PathBuf {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/tmp"));
        home.join(".lantern/config/fox-cloud.json")
    }

    pub fn load() -> anyhow::Result<Self> {
        let path = Self::path();
        let raw = std::fs::read_to_string(&path)
            .map_err(|e| anyhow::anyhow!("read {}: {e}", path.display()))?;
        let cfg: CloudConfig = serde_json::from_str(&raw)
            .map_err(|e| anyhow::anyhow!("parse {}: {e}", path.display()))?;
        Ok(cfg)
    }
}

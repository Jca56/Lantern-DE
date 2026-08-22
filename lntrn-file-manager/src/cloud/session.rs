// Cached Firebase Auth session: id_token + refresh_token + uid + expiry.
// Stored at ~/.lantern/config/fox-cloud-session.json. The id_token is a short-lived
// JWT (~1h); refresh_token is long-lived. We refresh on demand when within
// REFRESH_SKEW of expiry.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use super::CloudConfig;

/// Refresh if the id_token would expire within this many seconds.
const REFRESH_SKEW: u64 = 120;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub uid: String,
    pub email: String,
    pub id_token: String,
    pub refresh_token: String,
    /// Unix seconds at which id_token expires.
    pub expires_at: u64,
}

impl Session {
    pub fn path() -> PathBuf {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/tmp"));
        home.join(".lantern/config/fox-cloud-session.json")
    }

    pub fn load_cached(_cfg: &CloudConfig) -> anyhow::Result<Self> {
        let path = Self::path();
        let raw = std::fs::read_to_string(&path)
            .map_err(|e| anyhow::anyhow!("read {}: {e}", path.display()))?;
        let s: Session = serde_json::from_str(&raw)?;
        Ok(s)
    }

    pub fn save(&self) -> anyhow::Result<()> {
        let path = Self::path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let raw = serde_json::to_string_pretty(self)?;
        std::fs::write(&path, raw)?;
        // Tighten perms — this file holds a refresh token.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
        }
        Ok(())
    }

    pub fn forget() {
        let _ = std::fs::remove_file(Self::path());
    }

    pub fn is_fresh(&self) -> bool {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        self.expires_at > now + REFRESH_SKEW
    }
}

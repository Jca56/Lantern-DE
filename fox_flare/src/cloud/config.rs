use serde::{Deserialize, Serialize};
use std::path::PathBuf;

// ── Fox Den configuration ────────────────────────────────────────────────────

// Baked-in Firebase project config — no external file needed
const DEFAULT_API_KEY: &str = "AIzaSyAmOQ_Ag2STa-oLpkj3yzMxDQG3ESV4Fbc";
const DEFAULT_PROJECT_ID: &str = "fox-flare-f023f";
const DEFAULT_STORAGE_BUCKET: &str = "fox-flare-f023f.firebasestorage.app";

#[derive(Clone, Serialize, Deserialize)]
pub struct FoxDenConfig {
    pub api_key: String,
    pub project_id: String,
    pub storage_bucket: String,
    pub auth: Option<AuthTokens>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct AuthTokens {
    pub id_token: String,
    pub refresh_token: String,
    pub email: String,
    pub local_id: String,
    pub expires_at: u64,
}

impl AuthTokens {
    pub fn is_expired(&self) -> bool {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        // Consider expired 60 seconds early to avoid edge cases
        now >= self.expires_at.saturating_sub(60)
    }
}

// ── File paths ───────────────────────────────────────────────────────────────

fn config_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    PathBuf::from(home).join(".config").join("fox-flare")
}

fn config_path() -> PathBuf {
    config_dir().join("fox_den.json")
}

pub fn cache_dir() -> PathBuf {
    config_dir().join("fox_den_cache")
}

impl FoxDenConfig {
    /// Create config with baked-in project defaults and no auth
    pub fn with_defaults() -> Self {
        Self {
            api_key: DEFAULT_API_KEY.to_string(),
            project_id: DEFAULT_PROJECT_ID.to_string(),
            storage_bucket: DEFAULT_STORAGE_BUCKET.to_string(),
            auth: None,
        }
    }
}

// ── Load / Save ──────────────────────────────────────────────────────────────

pub fn load_config() -> FoxDenConfig {
    let path = config_path();
    // Start from baked-in defaults
    let mut config = FoxDenConfig::with_defaults();
    // Merge saved auth tokens if config file exists
    if let Ok(data) = std::fs::read_to_string(path) {
        if let Ok(saved) = serde_json::from_str::<FoxDenConfig>(&data) {
            config.auth = saved.auth;
        }
    }
    config
}

pub fn save_config(config: &FoxDenConfig) -> Result<(), String> {
    let dir = config_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("Failed to create config dir: {}", e))?;

    let json = serde_json::to_string_pretty(config)
        .map_err(|e| format!("Failed to serialize config: {}", e))?;

    let path = config_path();
    std::fs::write(&path, json).map_err(|e| format!("Failed to write config: {}", e))?;

    // Restrict permissions to owner-only (0600)
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o600);
        std::fs::set_permissions(&path, perms).ok();
    }

    Ok(())
}

pub fn ensure_cache_dir() -> Result<PathBuf, String> {
    let dir = cache_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("Failed to create cache dir: {}", e))?;
    Ok(dir)
}

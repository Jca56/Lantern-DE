//! Per-session UI state that survives daemon restarts.
//!
//! Kept deliberately tiny: which tab the user was on and whether the
//! panel was collapsed to its bar. Lives at
//! `~/.lantern/config/command-center/state.json` and is rewritten on
//! every relevant transition. Panel size lives in `settings.toml` now.

use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PanelViewKind {
    Default,
    Terminal,
    Files,
}

impl From<crate::app::PanelView> for PanelViewKind {
    fn from(v: crate::app::PanelView) -> Self {
        match v {
            crate::app::PanelView::Default => Self::Default,
            crate::app::PanelView::Terminal => Self::Terminal,
            crate::app::PanelView::Files => Self::Files,
        }
    }
}

impl From<PanelViewKind> for crate::app::PanelView {
    fn from(v: PanelViewKind) -> Self {
        match v {
            PanelViewKind::Default => Self::Default,
            PanelViewKind::Terminal => Self::Terminal,
            PanelViewKind::Files => Self::Files,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistedState {
    #[serde(default)]
    pub panel_view: PanelViewKind,
    #[serde(default)]
    pub collapsed: bool,
}

impl Default for PanelViewKind {
    fn default() -> Self { Self::Default }
}

impl Default for PersistedState {
    fn default() -> Self {
        Self {
            panel_view: PanelViewKind::Default,
            collapsed: false,
        }
    }
}

fn state_path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    let mut p = PathBuf::from(home);
    p.push(".lantern/config/command-center/state.json");
    Some(p)
}

pub fn load() -> PersistedState {
    let Some(path) = state_path() else { return PersistedState::default() };
    let Ok(text) = fs::read_to_string(&path) else { return PersistedState::default() };
    serde_json::from_str(&text).unwrap_or_default()
}

pub fn save(state: &PersistedState) {
    let Some(path) = state_path() else { return };
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let Ok(body) = serde_json::to_vec_pretty(state) else { return };
    let tmp = path.with_extension("json.tmp");
    if fs::write(&tmp, &body).is_ok() {
        let _ = fs::rename(&tmp, &path);
    }
}

//! What the picker remembers between recordings, in
//! `~/.lantern/config/lntrn-screen-capture.toml`: which sound to record.

use std::path::PathBuf;

use lntrn_core::log_warn;
use lntrn_data::{Doc, toml};

use crate::audio::Sources;

fn path() -> PathBuf {
    let config = lntrn_sys::dirs::lantern_config().unwrap_or_else(|| PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".lantern/config"));
    config.join("lntrn-screen-capture.toml")
}

/// The sound last recorded; desktop audio and no mic the first time.
pub fn load() -> Sources {
    let doc = std::fs::read_to_string(path()).ok().and_then(|text| toml::parse(&text).ok()).unwrap_or_else(Doc::map);
    let on = |key: &str, default: bool| doc.get(key).and_then(Doc::as_bool).unwrap_or(default);
    Sources { desktop: on("desktop_audio", true), mic: on("microphone", false) }
}

pub fn save(sources: Sources) {
    let mut doc = Doc::map();
    doc.set("desktop_audio", Doc::from(sources.desktop));
    doc.set("microphone", Doc::from(sources.mic));
    let path = path();
    let written = path.parent().map_or(Ok(()), std::fs::create_dir_all).and_then(|()| std::fs::write(&path, toml::write(&doc)));
    if let Err(e) = written {
        log_warn!("screen-capture: can't keep the sound settings in {}: {e}", path.display());
    }
}

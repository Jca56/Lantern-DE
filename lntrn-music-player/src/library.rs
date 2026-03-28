use crate::track::Track;
use lofty::file::{AudioFile, TaggedFileExt};
use lofty::tag::Accessor;
use std::path::Path;

const AUDIO_EXTENSIONS: &[&str] = &[
    "mp3", "flac", "wav", "ogg", "opus", "aac", "m4a",
];

pub fn scan_directory(dir: &Path) -> Vec<Track> {
    let mut tracks = Vec::new();
    scan_recursive(dir, &mut tracks);
    tracks.sort_by(|a, b| {
        a.title
            .to_lowercase()
            .cmp(&b.title.to_lowercase())
    });
    tracks
}

fn scan_recursive(dir: &Path, tracks: &mut Vec<Track>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            scan_recursive(&path, tracks);
        } else if is_audio_file(&path) {
            tracks.push(read_metadata(&path));
        }
    }
}

fn is_audio_file(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| AUDIO_EXTENSIONS.contains(&ext.to_lowercase().as_str()))
        .unwrap_or(false)
}

pub fn read_track_metadata(path: &Path) -> Track {
    read_metadata(path)
}

fn read_metadata(path: &Path) -> Track {
    let file_name = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Unknown")
        .to_string();

    let (title, artist, album, duration) = match lofty::read_from_path(path) {
        Ok(tagged_file) => {
            let duration = tagged_file.properties().duration();

            let tag = tagged_file
                .primary_tag()
                .or_else(|| tagged_file.first_tag());

            match tag {
                Some(tag) => (
                    tag.title()
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| file_name.clone()),
                    tag.artist()
                        .map(|s| s.to_string())
                        .unwrap_or_default(),
                    tag.album()
                        .map(|s| s.to_string())
                        .unwrap_or_default(),
                    Some(duration),
                ),
                None => (file_name.clone(), String::new(), String::new(), Some(duration)),
            }
        }
        Err(_) => (file_name, String::new(), String::new(), None),
    };

    Track {
        path: path.to_path_buf(),
        title,
        artist,
        album,
        duration,
    }
}

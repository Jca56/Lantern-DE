use crate::track::Track;
use std::path::PathBuf;

pub struct PlayQueue {
    pub tracks: Vec<Track>,
    pub current: Option<usize>,
}

impl PlayQueue {
    pub fn new() -> Self {
        Self {
            tracks: Vec::new(),
            current: None,
        }
    }

    pub fn set_tracks(&mut self, tracks: Vec<Track>) {
        self.tracks = tracks;
        self.current = None;
    }

    pub fn current_track(&self) -> Option<&Track> {
        self.current.and_then(|i| self.tracks.get(i))
    }

    pub fn play_index(&mut self, index: usize) -> Option<PathBuf> {
        if index < self.tracks.len() {
            self.current = Some(index);
            Some(self.tracks[index].path.clone())
        } else {
            None
        }
    }

    pub fn next(&mut self) -> Option<PathBuf> {
        match self.current {
            Some(i) if i + 1 < self.tracks.len() => {
                self.current = Some(i + 1);
                Some(self.tracks[i + 1].path.clone())
            }
            None if !self.tracks.is_empty() => {
                self.current = Some(0);
                Some(self.tracks[0].path.clone())
            }
            _ => None,
        }
    }

    pub fn previous(&mut self) -> Option<PathBuf> {
        match self.current {
            Some(i) if i > 0 => {
                self.current = Some(i - 1);
                Some(self.tracks[i - 1].path.clone())
            }
            _ => None,
        }
    }
}

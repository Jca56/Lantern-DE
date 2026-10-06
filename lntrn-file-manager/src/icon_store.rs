//! The textures the icon cache holds, with a bound on the thumbnails.
//!
//! Folder icons, the icon theme's and the picker's SVGs are few and shared
//! (one texture per icon, whatever the number of entries drawn with it),
//! and stay for the life of the window. Thumbnails are one texture per
//! file (up to about
//! 147 KB each at 192 px RGBA): a folder of ten thousand photos scrolled
//! from top to bottom used to leave every one of them on the GPU until the
//! next navigation, over a gigabyte. Here the limit is kept when a
//! thumbnail comes in: past it, the ones looked at longest ago go first,
//! and never one that was on screen in the last frame drawn. An evicted
//! thumbnail comes back from the disk cache when its row is scrolled to
//! again.
//!
//! Generic over the texture type so the bookkeeping can be tested without
//! a GPU.

use std::cell::Cell;
use std::collections::HashMap;
use std::path::Path;

/// Thumbnails kept at once (about 75 MB of textures).
pub const THUMB_CAP: usize = 512;

const THUMB_PREFIX: &str = "thumb:";

/// The cache key of a file's thumbnail. Size and modification stamp are
/// part of it, so a file that changed on disk gets a fresh texture and a
/// decode that failed mid-download is tried again once the file is whole.
pub fn thumb_key(path: &Path, size: u64, stamp: u128) -> String {
    format!("{THUMB_PREFIX}{}:{size}:{stamp}", path.display())
}

/// The path a thumbnail key was made for; `None` for any other key.
pub fn thumb_path(key: &str) -> Option<&str> {
    // The two fields after the path are numbers: split from the right, a
    // path may hold colons of its own.
    let rest = key.strip_prefix(THUMB_PREFIX)?;
    rest.rsplitn(3, ':').nth(2)
}

struct Slot<T> {
    value: T,
    /// The frame this was last looked up in. A `Cell`: the draw lists
    /// borrow textures from a shared reference to the store.
    used: Cell<u64>,
}

pub struct TextureStore<T> {
    slots: HashMap<String, Slot<T>>,
    /// Counts the frames drawn.
    frame: u64,
    thumbs: usize,
    cap: usize,
}

impl<T> TextureStore<T> {
    pub fn new() -> Self {
        Self::with_cap(THUMB_CAP)
    }

    pub fn with_cap(cap: usize) -> Self {
        Self {
            slots: HashMap::new(),
            frame: 0,
            thumbs: 0,
            cap,
        }
    }

    /// A new frame is being drawn. What is looked up from here on is what
    /// that frame shows.
    pub fn begin_frame(&mut self) {
        self.frame += 1;
    }

    pub fn contains(&self, key: &str) -> bool {
        self.slots.contains_key(key)
    }

    /// The texture under `key`, marked as used by the frame being drawn.
    pub fn get(&self, key: &str) -> Option<&T> {
        let slot = self.slots.get(key)?;
        slot.used.set(self.frame);
        Some(&slot.value)
    }

    /// Store a texture. A thumbnail replaces every other thumbnail of the
    /// same file (the file changed, the old key is never asked for again)
    /// and may push the least recently used ones out.
    pub fn insert(&mut self, key: String, value: T) {
        let is_thumb = key.starts_with(THUMB_PREFIX);
        if let Some(path) = thumb_path(&key) {
            let stale: Vec<String> = self
                .slots
                .keys()
                .filter(|k| **k != key && thumb_path(k) == Some(path))
                .cloned()
                .collect();
            for k in stale {
                self.remove(&k);
            }
        }
        let slot = Slot {
            value,
            used: Cell::new(self.frame),
        };
        if self.slots.insert(key, slot).is_none() && is_thumb {
            self.thumbs += 1;
        }
        if is_thumb && self.thumbs > self.cap {
            self.evict();
        }
    }

    fn remove(&mut self, key: &str) {
        if self.slots.remove(key).is_some() && key.starts_with(THUMB_PREFIX) {
            self.thumbs -= 1;
        }
    }

    /// Drop every texture `keep` says no to.
    pub fn retain(&mut self, mut keep: impl FnMut(&str) -> bool) {
        self.slots.retain(|k, _| keep(k));
        self.thumbs = self
            .slots
            .keys()
            .filter(|k| k.starts_with(THUMB_PREFIX))
            .count();
    }

    /// Over the limit: the thumbnails used longest ago go, down to a little
    /// under it (so this does not run again on the very next insert). What
    /// the last drawn frame or the one in progress looked up stays, even if
    /// that leaves more than the limit: a window can show that many.
    fn evict(&mut self) {
        let keep = self.cap - self.cap / 8;
        let mut idle: Vec<(u64, String)> = self
            .slots
            .iter()
            .filter(|(k, slot)| k.starts_with(THUMB_PREFIX) && slot.used.get() + 1 < self.frame)
            .map(|(k, slot)| (slot.used.get(), k.clone()))
            .collect();
        idle.sort();
        for (_, key) in idle {
            if self.thumbs <= keep {
                break;
            }
            self.remove(&key);
        }
    }

    #[cfg(test)]
    fn thumb_count(&self) -> usize {
        self.thumbs
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(name: &str) -> String {
        thumb_key(Path::new(name), 10, 1)
    }

    #[test]
    fn a_thumbnail_key_gives_its_path_back_even_with_colons_in_it() {
        let k = thumb_key(Path::new("/photos/12:30 at the lake.jpg"), 4096, 1_700_000_000_123);
        assert_eq!(thumb_path(&k), Some("/photos/12:30 at the lake.jpg"));
        assert_eq!(thumb_path("dir::/icons/a.svg:red"), None);
        assert_eq!(thumb_path("svg:/usr/share/icons/x.svg"), None);
    }

    #[test]
    fn past_the_limit_the_thumbnails_used_longest_ago_go() {
        let mut store: TextureStore<u32> = TextureStore::with_cap(8);
        // Eight thumbnails, each last looked at in a frame of its own.
        for i in 0..8 {
            store.begin_frame();
            store.insert(key(&format!("/p/{i}.jpg")), i);
        }
        for _ in 0..3 {
            store.begin_frame();
        }
        // The ninth goes over the limit: the oldest go, down to 7/8 of it.
        store.insert(key("/p/8.jpg"), 8);
        assert_eq!(store.thumb_count(), 7);
        assert!(!store.contains(&key("/p/0.jpg")));
        assert!(!store.contains(&key("/p/1.jpg")));
        assert!(store.contains(&key("/p/2.jpg")));
        assert!(store.contains(&key("/p/8.jpg")));
    }

    #[test]
    fn what_is_on_screen_is_never_evicted() {
        let mut store: TextureStore<u32> = TextureStore::with_cap(4);
        store.begin_frame();
        for i in 0..4 {
            store.insert(key(&format!("/p/{i}.jpg")), i);
        }
        // A window that shows more thumbnails than the limit: every frame
        // looks all of them up, and more keep arriving.
        for round in 0..3 {
            store.begin_frame();
            for i in 0..(4 + round) {
                assert!(store.get(&key(&format!("/p/{i}.jpg"))).is_some());
            }
            store.insert(key(&format!("/p/{}.jpg", 4 + round)), 0);
        }
        assert_eq!(store.thumb_count(), 7);

        // Scrolled away: two frames later only what is still looked up is
        // safe, and the next arrival brings the count back under the limit.
        for _ in 0..2 {
            store.begin_frame();
            assert!(store.get(&key("/p/6.jpg")).is_some());
        }
        store.insert(key("/p/new.jpg"), 0);
        assert!(store.contains(&key("/p/6.jpg")));
        assert!(store.contains(&key("/p/new.jpg")));
        assert!(store.thumb_count() <= 4);
    }

    #[test]
    fn the_frame_before_counts_as_on_screen_too() {
        // Thumbnails arrive at the start of a frame, before that frame has
        // looked anything up: what the previous frame showed must survive.
        let mut store: TextureStore<u32> = TextureStore::with_cap(2);
        store.begin_frame();
        store.insert(key("/p/a.jpg"), 0);
        store.insert(key("/p/b.jpg"), 0);
        store.begin_frame();
        store.insert(key("/p/c.jpg"), 0);
        assert!(store.contains(&key("/p/a.jpg")) && store.contains(&key("/p/b.jpg")));
    }

    #[test]
    fn a_changed_file_takes_the_place_of_its_old_thumbnail() {
        let mut store: TextureStore<u32> = TextureStore::new();
        store.begin_frame();
        let path = Path::new("/plots/plot.png");
        store.insert(thumb_key(path, 100, 1), 1);
        store.insert(thumb_key(Path::new("/plots/plot.png:2"), 100, 1), 7);
        // Rewritten on disk: new size and stamp, new key.
        store.insert(thumb_key(path, 120, 2), 2);
        assert_eq!(store.thumb_count(), 2);
        assert!(!store.contains(&thumb_key(path, 100, 1)));
        assert_eq!(store.get(&thumb_key(path, 120, 2)), Some(&2));
        // A file whose name merely starts the same is another file.
        assert!(store.contains(&thumb_key(Path::new("/plots/plot.png:2"), 100, 1)));
    }

    #[test]
    fn other_textures_are_not_counted_and_never_evicted() {
        let mut store: TextureStore<u32> = TextureStore::with_cap(2);
        store.begin_frame();
        for i in 0..10 {
            store.insert(format!("svg:/icons/{i}.svg"), i);
        }
        store.insert("dir:::".to_string(), 0);
        for _ in 0..5 {
            store.begin_frame();
        }
        for i in 0..4 {
            store.insert(key(&format!("/p/{i}.jpg")), i);
        }
        assert!((0..10).all(|i| store.contains(&format!("svg:/icons/{i}.svg"))));
        assert!(store.contains("dir:::"));

        // `retain` keeps the count right.
        store.retain(|k| !k.contains("/p/"));
        assert_eq!(store.thumb_count(), 0);
        store.insert(key("/p/z.jpg"), 0);
        assert_eq!(store.thumb_count(), 1);
    }
}

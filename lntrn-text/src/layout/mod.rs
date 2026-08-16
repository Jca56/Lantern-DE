//! Text layout: line building + the shaped-layout LRU cache.
//!
//! The cache mirrors the glyphon wrapper's exactly: keyed by (text, quantized
//! size, quantized max_width, weight, style, family) — **never color**.
//! Layouts are colorless; color is applied per-quad at queue time, so one
//! cached layout serves every color. Keying on color would multiply the cache
//! by every distinct RGBA and flood past the cap (see the wrapper's history).
//!
//! Unlike the wrapper (which resolved cached buffers at *render* time, so an
//! eviction between queue and render silently dropped glyphs), quads here are
//! built from the layout at queue time and stored by value — eviction can
//! never eat text that is already queued for the current frame.

pub(crate) mod line;

use std::collections::HashMap;

pub(crate) use line::Layout;

/// Same cap as the glyphon wrapper.
const MAX_CACHED_LAYOUTS: usize = 512;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct LayoutKey {
    pub text: String,
    pub font_size_bits: u32,
    pub max_width_bits: u32,
    pub weight: u8,
    pub style: u8,
    /// Empty string => the renderer's default family.
    pub family: String,
}

struct CachedLayout {
    layout: Layout,
    last_used: u64,
}

pub(crate) struct LayoutCache {
    map: HashMap<LayoutKey, CachedLayout>,
    use_tick: u64,
}

impl LayoutCache {
    pub fn new() -> Self {
        Self { map: HashMap::new(), use_tick: 0 }
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// Look up a layout, bumping its LRU recency on hit.
    pub fn get(&mut self, key: &LayoutKey) -> Option<&Layout> {
        self.use_tick = self.use_tick.wrapping_add(1).max(1);
        let tick = self.use_tick;
        self.map.get_mut(key).map(|c| {
            c.last_used = tick;
            &c.layout
        })
    }

    /// Insert a layout, evicting the least-recently-used entry at the cap.
    pub fn insert(&mut self, key: LayoutKey, layout: Layout) -> &Layout {
        if self.map.len() >= MAX_CACHED_LAYOUTS {
            if let Some(oldest) = self
                .map
                .iter()
                .min_by_key(|(_, c)| c.last_used)
                .map(|(k, _)| k.clone())
            {
                self.map.remove(&oldest);
            }
        }
        let tick = self.use_tick;
        &self
            .map
            .entry(key)
            .insert_entry(CachedLayout { layout, last_used: tick })
            .into_mut()
            .layout
    }
}

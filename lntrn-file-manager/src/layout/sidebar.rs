//! Sidebar layout.
//!
//! Sections (Places / Favorites / Devices) are collapsible and the Favorites
//! list is user-driven, so item rects can't be computed from index alone. We
//! walk the layout once and hand out a struct of all hit/draw rects. The
//! renderer (draw), the input pass (zone registration), the right-click
//! hit-test and the favourite drag all consume the same `SidebarLayout`, so
//! they can't drift.
//!
//! The rows are stacked from the top of the sidebar's strip and can be
//! taller than it (a dozen favourites and a few drives, or a short window):
//! the layout is then scrolled, and every rect it hands out is already at
//! its scrolled position. What lies outside `viewport` is neither drawn nor
//! clickable.

use lntrn_render::Rect;

use super::SIDEBAR_W;

const SIDEBAR_HEADER_H: f32 = 30.0;
const SIDEBAR_HEADER_GAP: f32 = 12.0; // gap above each section header
const SIDEBAR_PLACE_ITEM_H: f32 = 40.0;
const SIDEBAR_DRIVE_ITEM_H: f32 = 64.0;
const SIDEBAR_PHONE_ITEM_H: f32 = 56.0;
/// Room under the last row, so it does not sit on the status bar.
const SIDEBAR_BOTTOM_PAD: f32 = 8.0;

/// What the sidebar has to show.
#[derive(Clone, Copy, Debug, Default)]
pub struct SidebarSpec {
    pub places: usize,
    pub favorites: usize,
    pub drives: usize,
    pub phones: usize,
    pub places_collapsed: bool,
    pub favorites_collapsed: bool,
    pub devices_collapsed: bool,
}

pub struct SidebarLayout {
    pub places_header: Rect,
    pub place_items: Vec<Rect>,
    pub favorites_header: Rect,
    pub favorites_plus: Rect,
    pub favorite_items: Vec<Rect>,
    pub devices_header: Rect,
    pub drive_items: Vec<Rect>,
    pub phone_items: Vec<Rect>,
    /// True if the Devices section was emitted at all (hidden when no
    /// drives + no phones are present).
    pub has_devices: bool,
    /// The strip the rows live in: nav bar down to the status (or pick) bar.
    pub viewport: Rect,
    /// Height of everything stacked.
    pub content_h: f32,
    /// How far the rows are scrolled (already clamped to what there is).
    pub scroll: f32,
    /// Width kept free at the right for the scrollbar: 0 when all fits.
    pub gutter: f32,
}

impl SidebarLayout {
    pub fn max_scroll(&self) -> f32 {
        (self.content_h - self.viewport.h).max(0.0)
    }

    pub fn scrollable(&self) -> bool {
        self.content_h > self.viewport.h
    }

    /// The part of `rect` that is inside the sidebar's strip, if any: what
    /// may be registered as a zone.
    pub fn visible(&self, rect: Rect) -> Option<Rect> {
        rect.intersect(&self.viewport)
    }
}

/// Height of everything the sidebar stacks, top gap to bottom pad.
fn stacked_height(spec: &SidebarSpec, s: f32) -> f32 {
    let header = (SIDEBAR_HEADER_GAP + SIDEBAR_HEADER_H) * s;
    let mut h = header;
    if !spec.places_collapsed {
        h += spec.places as f32 * SIDEBAR_PLACE_ITEM_H * s;
    }
    h += header;
    if !spec.favorites_collapsed {
        h += spec.favorites as f32 * SIDEBAR_PLACE_ITEM_H * s;
    }
    if spec.drives > 0 || spec.phones > 0 {
        h += header;
        if !spec.devices_collapsed {
            h += spec.drives as f32 * SIDEBAR_DRIVE_ITEM_H * s;
            h += spec.phones as f32 * SIDEBAR_PHONE_ITEM_H * s;
        }
    }
    h + SIDEBAR_BOTTOM_PAD * s
}

/// Lay the sidebar out inside `viewport`, scrolled by `scroll` (clamped
/// here to what there is to scroll). `scrollbar_gutter` is the width a
/// scrollbar needs; it is only taken from the rows when they overflow.
pub fn build_sidebar_layout(
    s: f32,
    spec: &SidebarSpec,
    viewport: Rect,
    scroll: f32,
    scrollbar_gutter: f32,
) -> SidebarLayout {
    let content_h = stacked_height(spec, s);
    let overflow = content_h > viewport.h;
    let gutter = if overflow { scrollbar_gutter } else { 0.0 };
    let scroll = scroll.clamp(0.0, (content_h - viewport.h).max(0.0));

    let item_x = 4.0 * s;
    let item_w = (SIDEBAR_W - 12.0) * s - gutter;
    let header_w = SIDEBAR_W * s - gutter;
    let header_h = SIDEBAR_HEADER_H * s;
    let header_gap = SIDEBAR_HEADER_GAP * s;
    let place_h = SIDEBAR_PLACE_ITEM_H * s;
    let drive_h = SIDEBAR_DRIVE_ITEM_H * s;
    let phone_h = SIDEBAR_PHONE_ITEM_H * s;

    let mut y = viewport.y - scroll + header_gap;

    // ── PLACES ───────────────────────────────────────────────────────
    let places_header = Rect::new(0.0, y, header_w, header_h);
    y += header_h;
    let mut place_items = Vec::with_capacity(spec.places);
    if !spec.places_collapsed {
        for _ in 0..spec.places {
            place_items.push(Rect::new(item_x, y, item_w, place_h));
            y += place_h;
        }
    }

    // ── FAVORITES ────────────────────────────────────────────────────
    y += header_gap;
    let favorites_header = Rect::new(0.0, y, header_w, header_h);
    // Plus button on the right side of the header.
    let plus_sz = 24.0 * s;
    let favorites_plus = Rect::new(
        header_w - plus_sz - 12.0 * s,
        y + (header_h - plus_sz) * 0.5,
        plus_sz,
        plus_sz,
    );
    y += header_h;
    let mut favorite_items = Vec::with_capacity(spec.favorites);
    if !spec.favorites_collapsed {
        for _ in 0..spec.favorites {
            favorite_items.push(Rect::new(item_x, y, item_w, place_h));
            y += place_h;
        }
    }

    // ── DEVICES ──────────────────────────────────────────────────────
    let has_devices = spec.drives > 0 || spec.phones > 0;
    let devices_header;
    let mut drive_items = Vec::with_capacity(spec.drives);
    let mut phone_items = Vec::with_capacity(spec.phones);
    if has_devices {
        y += header_gap;
        devices_header = Rect::new(0.0, y, header_w, header_h);
        y += header_h;
        if !spec.devices_collapsed {
            for _ in 0..spec.drives {
                drive_items.push(Rect::new(item_x, y, item_w, drive_h));
                y += drive_h;
            }
            for _ in 0..spec.phones {
                phone_items.push(Rect::new(item_x, y, item_w, phone_h));
                y += phone_h;
            }
        }
    } else {
        devices_header = Rect::new(0.0, 0.0, 0.0, 0.0);
    }

    SidebarLayout {
        places_header,
        place_items,
        favorites_header,
        favorites_plus,
        favorite_items,
        devices_header,
        drive_items,
        phone_items,
        has_devices,
        viewport,
        content_h,
        scroll,
        gutter,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(favorites: usize, drives: usize) -> SidebarSpec {
        SidebarSpec {
            places: 9,
            favorites,
            drives,
            ..Default::default()
        }
    }

    fn last_row(layout: &SidebarLayout) -> Rect {
        *layout
            .phone_items
            .last()
            .or(layout.drive_items.last())
            .or(layout.favorite_items.last())
            .or(layout.place_items.last())
            .unwrap()
    }

    /// Bottom edge of the lowest header or row.
    fn lowest(layout: &SidebarLayout) -> f32 {
        let mut rects = vec![layout.places_header, layout.favorites_header];
        if layout.has_devices {
            rects.push(layout.devices_header);
        }
        rects.extend(&layout.place_items);
        rects.extend(&layout.favorite_items);
        rects.extend(&layout.drive_items);
        rects.extend(&layout.phone_items);
        rects.iter().map(|r| r.y + r.h).fold(f32::MIN, f32::max)
    }

    #[test]
    fn a_sidebar_that_fits_is_not_scrolled_and_keeps_its_full_width() {
        let viewport = Rect::new(0.0, 44.0, 240.0, 900.0);
        let layout = build_sidebar_layout(1.0, &spec(2, 1), viewport, 500.0, 24.0);
        assert!(!layout.scrollable());
        assert_eq!(
            layout.scroll, 0.0,
            "nothing to scroll: the offset is dropped"
        );
        assert_eq!(layout.gutter, 0.0);
        assert_eq!(layout.place_items[0].w, 228.0);
        // First header sits one gap under the top of the strip.
        assert_eq!(layout.places_header.y, 44.0 + 12.0);
        let last = last_row(&layout);
        assert!(last.y + last.h <= viewport.y + viewport.h);
    }

    #[test]
    fn an_overflowing_sidebar_scrolls_until_its_last_row_is_in_view() {
        // A dozen favourites and three drives in a 600px strip.
        let viewport = Rect::new(0.0, 44.0, 240.0, 600.0);
        let top = build_sidebar_layout(1.0, &spec(12, 3), viewport, 0.0, 24.0);
        assert!(top.scrollable());
        assert_eq!(top.gutter, 24.0);
        assert_eq!(
            top.place_items[0].w,
            228.0 - 24.0,
            "rows make room for the bar"
        );
        let last = last_row(&top);
        assert!(
            last.y > viewport.y + viewport.h,
            "unscrolled, the last drive is below the strip"
        );
        assert!(top.visible(last).is_none(), "and has no zone there");

        // Scrolled as far as it goes (the offset asked for is clamped).
        let end = build_sidebar_layout(1.0, &spec(12, 3), viewport, 1.0e6, 24.0);
        assert_eq!(end.scroll, end.max_scroll());
        let last = last_row(&end);
        assert!(last.y + last.h <= viewport.y + viewport.h);
        assert_eq!(end.visible(last).map(|r| r.h), Some(last.h));
        // ... and the first rows are the ones out of view now.
        assert!(end.visible(end.places_header).is_none());
    }

    #[test]
    fn the_height_counted_is_the_height_laid_out() {
        for (favs, drives, phones) in [(0, 0, 0), (5, 0, 0), (3, 2, 1), (20, 4, 2)] {
            let spec = SidebarSpec {
                places: 9,
                favorites: favs,
                drives,
                phones,
                ..Default::default()
            };
            let viewport = Rect::new(0.0, 0.0, 336.0, 10_000.0);
            let layout = build_sidebar_layout(1.4, &spec, viewport, 0.0, 30.0);
            let bottom = lowest(&layout) + SIDEBAR_BOTTOM_PAD * 1.4;
            assert!(
                (bottom - layout.content_h).abs() < 0.01,
                "{favs}/{drives}/{phones}"
            );
        }
        // Collapsed sections take their header only.
        let folded = SidebarSpec {
            places: 9,
            favorites: 7,
            drives: 2,
            places_collapsed: true,
            favorites_collapsed: true,
            devices_collapsed: true,
            ..Default::default()
        };
        assert_eq!(stacked_height(&folded, 1.0), 3.0 * 42.0 + 8.0);
    }
}

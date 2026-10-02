//! The sidebar's click zones, registered from the same layout it is drawn
//! from. Only what is inside the sidebar's strip gets a zone: a row
//! scrolled out of it (or, before the sidebar scrolled, stacked past its
//! bottom over the status bar) is not there to be clicked.

use lntrn_render::Rect;
use lntrn_ui::gpu::{InteractionContext, InteractionState, Scrollbar};

use crate::layout::SidebarLayout;
use crate::{
    ZONE_DRIVE_ITEM_BASE, ZONE_FAVORITE_ITEM_BASE, ZONE_PHONE_ITEM_BASE,
    ZONE_SIDEBAR_DEVICES_HEADER, ZONE_SIDEBAR_FAVORITES_HEADER, ZONE_SIDEBAR_FAVORITES_PLUS,
    ZONE_SIDEBAR_ITEM_BASE, ZONE_SIDEBAR_PLACES_HEADER, ZONE_SIDEBAR_SCROLLBAR,
};

/// What the pointer is over, and the scrollbar when there is one.
pub struct SidebarHovered {
    pub places: Vec<bool>,
    pub favorites: Vec<bool>,
    pub drives: Vec<bool>,
    pub phones: Vec<bool>,
    pub places_header: bool,
    pub favorites_header: bool,
    pub devices_header: bool,
    pub favorites_plus: bool,
    pub scrollbar: Option<(Scrollbar, InteractionState)>,
}

pub fn register_sidebar_zones(
    input: &mut InteractionContext,
    layout: &SidebarLayout,
    s: f32,
) -> SidebarHovered {
    let mut zone = |id: u32, rect: Rect| -> bool {
        layout
            .visible(rect)
            .is_some_and(|r| input.add_zone(id, r).is_hovered())
    };
    let mut rows = |base: u32, rects: &[Rect]| -> Vec<bool> {
        rects
            .iter()
            .enumerate()
            .map(|(i, rect)| zone(base + i as u32, *rect))
            .collect()
    };
    let places = rows(ZONE_SIDEBAR_ITEM_BASE, &layout.place_items);
    let favorites = rows(ZONE_FAVORITE_ITEM_BASE, &layout.favorite_items);
    let drives = rows(ZONE_DRIVE_ITEM_BASE, &layout.drive_items);
    let phones = rows(ZONE_PHONE_ITEM_BASE, &layout.phone_items);
    // Section header zones (toggle collapse on click).
    let places_header = zone(ZONE_SIDEBAR_PLACES_HEADER, layout.places_header);
    let favorites_header = zone(ZONE_SIDEBAR_FAVORITES_HEADER, layout.favorites_header);
    let favorites_plus = zone(ZONE_SIDEBAR_FAVORITES_PLUS, layout.favorites_plus);
    let devices_header =
        layout.has_devices && zone(ZONE_SIDEBAR_DEVICES_HEADER, layout.devices_header);

    // Last, so it is on top of the rows it runs alongside.
    let scrollbar = crate::scrollbar::sidebar_bar(layout, s).map(|bar| {
        let band = crate::scrollbar::hover_zone(&bar, s);
        let state = match band.intersect(&layout.viewport) {
            Some(visible) => input.add_zone(ZONE_SIDEBAR_SCROLLBAR, visible),
            None => InteractionState::Idle,
        };
        (bar, state)
    });

    SidebarHovered {
        places,
        favorites,
        drives,
        phones,
        places_header,
        favorites_header,
        devices_header,
        favorites_plus,
        scrollbar,
    }
}

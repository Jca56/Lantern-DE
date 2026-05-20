//! Expandable category tree on the left edge.
//!
//! Each entry in [`CATEGORIES`] is a category — either a leaf (no children) that
//! selects a `Panel` directly (Home), or a parent that holds a list of child
//! panels. Parents are click-to-toggle; children select content.
//!
//! Zones are allocated dynamically per-frame: each rendered row (parent or
//! visible child) gets the next `ZONE_SIDEBAR_BASE + i` id, and the row map is
//! stashed on [`SidebarState`] so the click router knows what each id meant.

use lntrn_render::{Color, GpuTexture, Painter, Rect, TextRenderer, TextureDraw};
use lntrn_ui::gpu::{FoxPalette, InteractionContext};

use crate::wayland::{Panel, ZONE_SIDEBAR_BASE};

pub(crate) const SIDEBAR_W: f32 = 300.0;
pub(crate) const SIDEBAR_ITEM_H: f32 = 60.0;
pub(crate) const SIDEBAR_CHILD_H: f32 = 44.0;
pub(crate) const SIDEBAR_ICON_DRAW: f32 = 32.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Category {
    Home, Appearance, Display, Input, Notifications, Power, Apps,
}

pub(crate) struct CategoryDef {
    pub cat: Category,
    pub label: &'static str,
    pub icon_idx: usize,
    pub children: &'static [(Panel, &'static str)],
    /// When clicking a leaf category (no children), the panel to select.
    /// `None` for parent-only categories — clicking just toggles expansion.
    pub leaf_panel: Option<Panel>,
}

pub(crate) const CATEGORIES: &[CategoryDef] = &[
    CategoryDef { cat: Category::Home,          label: "Home",          icon_idx: 0, children: &[], leaf_panel: Some(Panel::Home) },
    CategoryDef { cat: Category::Appearance,    label: "Appearance",    icon_idx: 1, leaf_panel: None, children: &[
        (Panel::Themes,     "Themes"),
        (Panel::Wallpaper,  "Wallpaper"),
        (Panel::Animations, "Animations"),
    ]},
    CategoryDef { cat: Category::Display,       label: "Display",       icon_idx: 2, children: &[], leaf_panel: Some(Panel::Monitors) },
    CategoryDef { cat: Category::Input,         label: "Input",         icon_idx: 3, leaf_panel: None, children: &[
        (Panel::Mouse,      "Mouse"),
    ]},
    CategoryDef { cat: Category::Notifications, label: "Notifications", icon_idx: 4, leaf_panel: None, children: &[
        (Panel::NotifBehavior, "Behavior"),
        (Panel::NotifSound,    "Sound"),
        (Panel::NotifTesting,  "Testing"),
    ]},
    CategoryDef { cat: Category::Power,         label: "Power",         icon_idx: 5, leaf_panel: None, children: &[
        (Panel::LidIdle,    "Lid & Idle"),
        (Panel::Battery,    "Battery"),
        (Panel::WifiPower,  "WiFi Power"),
    ]},
    CategoryDef { cat: Category::Apps,          label: "Apps",          icon_idx: 6, leaf_panel: None, children: &[
        (Panel::AppIcons,   "Icons"),
    ]},
];

/// Find the category that owns `panel` (so we can auto-expand it).
pub(crate) fn category_of(panel: Panel) -> Category {
    if panel == Panel::Home { return Category::Home; }
    for cat in CATEGORIES {
        if cat.children.iter().any(|(p, _)| *p == panel) {
            return cat.cat;
        }
        if cat.leaf_panel == Some(panel) {
            return cat.cat;
        }
    }
    Category::Home
}

/// Human-readable label for `panel` — used for the content header.
pub(crate) fn panel_label(panel: Panel) -> &'static str {
    if panel == Panel::Home { return "Home"; }
    for cat in CATEGORIES {
        for (p, label) in cat.children {
            if *p == panel { return label; }
        }
        if cat.leaf_panel == Some(panel) {
            return cat.label;
        }
    }
    ""
}

#[derive(Clone, Copy)]
pub(crate) enum SidebarAction {
    ToggleCategory(usize),
    SelectPanel(Panel),
}

pub(crate) struct SidebarState {
    /// Per-category expansion flags (parallel to [`CATEGORIES`]).
    expanded: Vec<bool>,
    /// Most-recently-built row map — populated each frame by [`draw_sidebar`].
    /// `route_zone_click` reads it to translate a zone id back to an action.
    pub(crate) row_actions: Vec<SidebarAction>,
}

impl SidebarState {
    pub(crate) fn new(active_panel: Panel) -> Self {
        let mut expanded = vec![false; CATEGORIES.len()];
        // Auto-expand whichever category owns the starting panel.
        let active_cat = category_of(active_panel);
        if let Some(i) = CATEGORIES.iter().position(|c| c.cat == active_cat) {
            expanded[i] = true;
        }
        Self { expanded, row_actions: Vec::new() }
    }

    pub(crate) fn toggle(&mut self, idx: usize) {
        if idx < self.expanded.len() { self.expanded[idx] = !self.expanded[idx]; }
    }

    pub(crate) fn ensure_expanded(&mut self, cat: Category) {
        if let Some(i) = CATEGORIES.iter().position(|c| c.cat == cat) {
            self.expanded[i] = true;
        }
    }

    pub(crate) fn action_for_row(&self, row: usize) -> Option<SidebarAction> {
        self.row_actions.get(row).copied()
    }
}

/// Render the sidebar tree. Pushes icon `TextureDraw`s into `tex_draws` so the
/// caller can batch them. Mutates `state.row_actions` so the click router can
/// resolve zone ids back to actions.
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_sidebar<'a>(
    state: &mut SidebarState,
    painter: &mut Painter,
    text: &mut TextRenderer,
    ix: &mut InteractionContext,
    fox: &FoxPalette,
    icon_textures: &'a [GpuTexture],
    tex_draws: &mut Vec<TextureDraw<'a>>,
    active_panel: Panel,
    sidebar_w: f32,
    body_y: f32,
    hf: f32,
    s: f32,
    sw: u32,
    sh: u32,
) {
    state.row_actions.clear();
    let parent_h = SIDEBAR_ITEM_H * s;
    let child_h  = SIDEBAR_CHILD_H * s;
    let label_size  = 22.0 * s;
    let child_label = 19.0 * s;
    let icon_draw   = SIDEBAR_ICON_DRAW * s;

    // Divider line between sidebar and content.
    painter.rect_filled(
        Rect::new(sidebar_w, body_y, 1.0 * s, hf - body_y),
        0.0,
        fox.muted.with_alpha(0.35),
    );

    let active_cat = category_of(active_panel);
    let mut y = body_y + 8.0 * s;

    for (cat_i, cat) in CATEGORIES.iter().enumerate() {
        let row_idx = state.row_actions.len();
        let zone_id = ZONE_SIDEBAR_BASE + row_idx as u32;
        let row_rect = Rect::new(0.0, y, sidebar_w, parent_h);
        let zone_state = ix.add_zone(zone_id, row_rect);

        let is_leaf = cat.children.is_empty();
        let is_active_parent = !is_leaf && cat.cat == active_cat;
        let is_active_leaf = is_leaf && cat.leaf_panel == Some(active_panel);
        let force_expanded = is_active_parent;
        let expanded = state.expanded[cat_i] || force_expanded;

        // Active pill (only for leaf categories whose panel is active).
        if is_active_leaf {
            draw_active_pill(painter, fox, 0.0, y, sidebar_w, parent_h, s);
        } else if zone_state.is_hovered() {
            draw_hover_pill(painter, fox, 0.0, y, sidebar_w, parent_h, s);
        }

        // Icon
        let icon_x = 22.0 * s;
        let icon_y = y + (parent_h - icon_draw) / 2.0;
        if let Some(tex) = icon_textures.get(cat.icon_idx) {
            tex_draws.push(TextureDraw::new(tex, icon_x, icon_y, icon_draw, icon_draw));
        }

        // Label
        let text_x = icon_x + icon_draw + 14.0 * s;
        let text_y = y + (parent_h - label_size) / 2.0;
        let text_color = if is_active_leaf || is_active_parent { fox.accent } else { fox.text };
        text.queue(cat.label, label_size, text_x, text_y, text_color, sidebar_w - text_x, sw, sh);

        // Chevron on the right for parents (only when they have children).
        if !is_leaf {
            let chev_size = 18.0 * s;
            let chev_x = sidebar_w - chev_size - 20.0 * s;
            let chev_y = y + (parent_h - chev_size) / 2.0;
            draw_chevron(painter, fox, chev_x, chev_y, chev_size, expanded, text_color);
        }

        // Record the click action for this row.
        if is_leaf {
            state.row_actions.push(SidebarAction::SelectPanel(cat.leaf_panel.unwrap_or(Panel::Home)));
        } else {
            state.row_actions.push(SidebarAction::ToggleCategory(cat_i));
        }

        y += parent_h;

        // Children
        if !is_leaf && expanded {
            for (child_panel, child_label_str) in cat.children {
                let row_idx = state.row_actions.len();
                let zone_id = ZONE_SIDEBAR_BASE + row_idx as u32;
                let child_rect = Rect::new(0.0, y, sidebar_w, child_h);
                let zone_state = ix.add_zone(zone_id, child_rect);

                let is_active = *child_panel == active_panel;
                if is_active {
                    draw_active_pill_child(painter, fox, 0.0, y, sidebar_w, child_h, s);
                } else if zone_state.is_hovered() {
                    draw_hover_pill_child(painter, fox, 0.0, y, sidebar_w, child_h, s);
                }

                let label_x_child = icon_x + icon_draw + 14.0 * s;
                let label_y_child = y + (child_h - child_label) / 2.0;
                let col = if is_active { fox.accent } else { fox.text.with_alpha(0.85) };
                text.queue(child_label_str, child_label, label_x_child, label_y_child, col, sidebar_w - label_x_child, sw, sh);

                state.row_actions.push(SidebarAction::SelectPanel(*child_panel));
                y += child_h;
            }
            y += 4.0 * s; // small gap after a group
        }
    }
}

fn draw_active_pill(painter: &mut Painter, fox: &FoxPalette, x: f32, y: f32, w: f32, h: f32, s: f32) {
    let inset_x = 10.0 * s;
    let inset_y = 4.0 * s;
    let pill = Rect::new(x + inset_x, y + inset_y, w - inset_x * 2.0, h - inset_y * 2.0);
    let radius = 8.0 * s;
    painter.rect_filled(pill, radius, Color::from_rgba8(255, 180, 30, 56));
    painter.rect_stroke_sdf(pill, radius, 1.0 * s, Color::from_rgba8(255, 180, 30, 110));
    painter.rect_filled(
        Rect::new(x, y + 6.0 * s, 4.0 * s, h - 12.0 * s),
        2.0 * s,
        fox.accent,
    );
}

fn draw_active_pill_child(painter: &mut Painter, fox: &FoxPalette, x: f32, y: f32, w: f32, h: f32, s: f32) {
    let inset_x = 18.0 * s;
    let inset_y = 3.0 * s;
    let pill = Rect::new(x + inset_x, y + inset_y, w - inset_x * 2.0, h - inset_y * 2.0);
    let radius = 6.0 * s;
    painter.rect_filled(pill, radius, Color::from_rgba8(255, 180, 30, 42));
    painter.rect_filled(
        Rect::new(x + 12.0 * s, y + 8.0 * s, 3.0 * s, h - 16.0 * s),
        1.5 * s,
        fox.accent,
    );
}

fn draw_hover_pill(painter: &mut Painter, fox: &FoxPalette, x: f32, y: f32, w: f32, h: f32, s: f32) {
    let inset_x = 10.0 * s;
    let inset_y = 4.0 * s;
    let pill = Rect::new(x + inset_x, y + inset_y, w - inset_x * 2.0, h - inset_y * 2.0);
    painter.rect_filled(pill, 8.0 * s, fox.text.with_alpha(0.06));
}

fn draw_hover_pill_child(painter: &mut Painter, fox: &FoxPalette, x: f32, y: f32, w: f32, h: f32, s: f32) {
    let inset_x = 18.0 * s;
    let inset_y = 3.0 * s;
    let pill = Rect::new(x + inset_x, y + inset_y, w - inset_x * 2.0, h - inset_y * 2.0);
    painter.rect_filled(pill, 6.0 * s, fox.text.with_alpha(0.05));
}

/// Chevron pointing right (collapsed) or down (expanded). Two short line
/// segments meeting at the apex.
fn draw_chevron(painter: &mut Painter, _fox: &FoxPalette, x: f32, y: f32, size: f32, expanded: bool, color: Color) {
    let stroke = (size * 0.16).max(2.0);
    let c = color.with_alpha(0.75);
    if expanded {
        // v — apex bottom-center, lines coming down from top-left and top-right
        let pad = size * 0.18;
        let ax = x + pad;             let ay = y + pad;
        let bx = x + size - pad;      let by = y + pad;
        let mx = x + size * 0.5;      let my = y + size - pad;
        painter.line(ax, ay, mx, my, stroke, c);
        painter.line(bx, by, mx, my, stroke, c);
    } else {
        // > — apex right-center
        let pad = size * 0.18;
        let ax = x + pad;             let ay = y + pad;
        let bx = x + pad;             let by = y + size - pad;
        let mx = x + size - pad;      let my = y + size * 0.5;
        painter.line(ax, ay, mx, my, stroke, c);
        painter.line(bx, by, mx, my, stroke, c);
    }
}

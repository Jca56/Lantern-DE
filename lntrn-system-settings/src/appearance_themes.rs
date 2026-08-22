//! Themes card for the Appearance panel.
//!
//! Renders a horizontal grid of theme preset tiles plus a trailing "+" tile
//! to save the current state as a new theme. Click a tile to apply.
//! Right-click (or the "..." tile button) opens a context menu with
//! Rename / Update from current / Delete.
//!
//! State lives in `ThemesPanelState`, owned by the main wayland loop and
//! passed in alongside the live `LanternConfig`.

use std::collections::HashMap;
use std::path::PathBuf;

use lntrn_render::{
    Color, GpuContext, GpuTexture, Painter, Rect, TextRenderer, TextureDraw, TexturePass,
};
use lntrn_ui::gpu::{Button, ButtonVariant, FoxPalette, InteractionContext};

// Cached tile geometry from the most recent draw, used by
// `collect_theme_thumbs` to push thumbnail textures with the right lifetime.
#[derive(Clone)]
struct TileLayout {
    slug: String,
    inner: Rect,
}

use crate::config::LanternConfig;
use crate::panels::{draw_section_card, PanelState, CARD_INNER_PAD_H};
use crate::text_edit::TextBuffer;
use crate::themes::{self, MoveDir, ThemePreset};

// ── Context menu action IDs (600 range) ─────────────────────────────────────
pub(crate) const ACT_THEME_RENAME: u32 = 600;
pub(crate) const ACT_THEME_UPDATE: u32 = 601;
pub(crate) const ACT_THEME_MOVE_LEFT: u32 = 602;
pub(crate) const ACT_THEME_MOVE_RIGHT: u32 = 603;
pub(crate) const ACT_THEME_DELETE: u32 = 604;

// ── Zone IDs (400 range — keep clear of other panels) ──────────────────────

pub(crate) const ZONE_THEME_TILE_BASE: u32 = 400; // +0..MAX_THEMES
pub(crate) const ZONE_THEME_TILE_MENU_BASE: u32 = 460; // tile "..." button
const MAX_THEMES: u32 = 60;
pub(crate) const ZONE_THEME_ADD: u32 = 478;
pub(crate) const ZONE_THEME_MODAL_DISMISS: u32 = 480;
pub(crate) const ZONE_THEME_MODAL_INPUT: u32 = 481;
pub(crate) const ZONE_THEME_MODAL_CANCEL: u32 = 482;
pub(crate) const ZONE_THEME_MODAL_SAVE: u32 = 483;

// Tile geometry
const TILE_W: f32 = 220.0;
const TILE_H: f32 = 140.0;
const TILE_GAP: f32 = 14.0;
const TILE_RADIUS: f32 = 10.0;

// Thumbnail size we decode to (square-ish to fit tile)
const THUMB_W: u32 = 440;
const THUMB_H: u32 = 280;

// ── State ───────────────────────────────────────────────────────────────────

pub enum ModalState {
    Closed,
    Save(TextBuffer),
    Rename { slug: String, buffer: TextBuffer },
}

pub struct ThemesPanelState {
    pub themes: Vec<ThemePreset>,
    pub needs_reload: bool,
    /// slug → loaded texture. `None` value = tried to load, no wallpaper or
    /// file missing. Missing key = not yet attempted.
    pub thumbnails: HashMap<String, Option<GpuTexture>>,
    pub modal: ModalState,
    /// Geometry of the input field as drawn last frame.
    pub input_rect: Option<Rect>,
    /// Slug whose context menu is currently open. Reserved for the
    /// context-menu pass — not consumed yet.
    #[allow(dead_code)]
    pub context_target: Option<String>,
    /// Inner rect for each rendered tile, in render order. Populated by
    /// `draw_themes_card`, consumed by `collect_theme_thumbs`.
    tile_layouts: Vec<TileLayout>,
}

impl ThemesPanelState {
    pub fn new() -> Self {
        Self {
            themes: themes::list_themes(),
            needs_reload: false,
            thumbnails: HashMap::new(),
            modal: ModalState::Closed,
            input_rect: None,
            context_target: None,
            tile_layouts: Vec::new(),
        }
    }

    pub fn reload(&mut self) {
        self.themes = themes::list_themes();
        // Drop thumbnails for themes that no longer exist.
        let live: std::collections::HashSet<&str> =
            self.themes.iter().map(|t| t.slug.as_str()).collect();
        self.thumbnails.retain(|k, _| live.contains(k.as_str()));
        self.needs_reload = false;
    }

    pub fn modal_open(&self) -> bool {
        !matches!(self.modal, ModalState::Closed)
    }

    /// Handle a keypress while the modal is open. Returns `(consumed,
    /// action)` — caller acts on `action` if Some.
    pub fn handle_key(
        &mut self,
        sym: xkbcommon::xkb::Keysym,
        utf8: Option<String>,
        config: &mut LanternConfig,
    ) -> bool {
        if matches!(self.modal, ModalState::Closed) {
            return false;
        }
        match sym.raw() {
            0xff0d | 0xff8d => {
                // Enter — confirm
                self.confirm_modal(config);
                true
            }
            0xff1b => {
                self.modal = ModalState::Closed;
                true
            } // Escape
            0xff08 => {
                self.with_buffer(|b| b.backspace());
                true
            }
            0xffff => {
                self.with_buffer(|b| b.delete());
                true
            }
            0xff51 => {
                self.with_buffer(|b| b.left());
                true
            }
            0xff53 => {
                self.with_buffer(|b| b.right());
                true
            }
            0xff50 => {
                self.with_buffer(|b| b.home());
                true
            }
            0xff57 => {
                self.with_buffer(|b| b.end());
                true
            }
            _ => {
                if let Some(ch) = utf8 {
                    // Filter control chars
                    if ch.chars().all(|c| !c.is_control()) {
                        self.with_buffer(|b| b.insert(&ch));
                    }
                    true
                } else {
                    false
                }
            }
        }
    }

    fn with_buffer(&mut self, f: impl FnOnce(&mut TextBuffer)) {
        match &mut self.modal {
            ModalState::Save(b) => f(b),
            ModalState::Rename { buffer, .. } => f(buffer),
            ModalState::Closed => {}
        }
    }

    fn confirm_modal(&mut self, config: &mut LanternConfig) {
        match std::mem::replace(&mut self.modal, ModalState::Closed) {
            ModalState::Save(buf) => {
                let name = buf.text.trim();
                if !name.is_empty() {
                    if let Ok(preset) = themes::save_theme(name, config) {
                        config.appearance.active_theme = preset.slug.clone();
                        config.save();
                    }
                    self.needs_reload = true;
                }
            }
            ModalState::Rename { slug, buffer } => {
                let name = buffer.text.trim();
                if !name.is_empty() {
                    let _ = themes::rename_theme(&slug, name);
                    self.needs_reload = true;
                }
            }
            ModalState::Closed => {}
        }
    }
}

// ── Drawing ─────────────────────────────────────────────────────────────────

/// Compute the height the Themes card needs given a card inner width.
/// Wraps tiles to as many rows as needed (plus the `+` tile), so the panel
/// can size its scroll area correctly before drawing.
pub fn themes_card_height(state: &ThemesPanelState, inner_w: f32, s: f32) -> f32 {
    use crate::panels::{CARD_HEADER_H, CARD_INNER_PAD_V};
    let tile_w = TILE_W * s;
    let tile_h = TILE_H * s;
    let gap = TILE_GAP * s;
    let cols = tiles_per_row(inner_w, tile_w, gap);
    let total_tiles = state.themes.len() + 1; // +1 for the "+" tile
    let rows = ((total_tiles + cols - 1) / cols).max(1);
    let card_chrome_h = CARD_HEADER_H * s + CARD_INNER_PAD_V * 2.0 * s;
    card_chrome_h + rows as f32 * tile_h + (rows.saturating_sub(1)) as f32 * gap
}

fn tiles_per_row(inner_w: f32, tile_w: f32, gap: f32) -> usize {
    ((inner_w + gap) / (tile_w + gap)).floor().max(1.0) as usize
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_themes_card(
    state: &mut ThemesPanelState,
    config: &LanternConfig,
    painter: &mut Painter,
    text: &mut TextRenderer,
    ix: &mut InteractionContext,
    tex_pass: &TexturePass,
    gpu: &GpuContext,
    fox: &FoxPalette,
    _tex_draws: &mut Vec<TextureDraw>,
    card_x: f32,
    card_y: f32,
    card_w: f32,
    card_h: f32,
    s: f32,
    sw: u32,
    sh: u32,
) {
    if state.needs_reload {
        state.reload();
    }
    ensure_thumbnails(state, tex_pass, gpu);
    state.tile_layouts.clear();

    let card_inner_x = card_x + CARD_INNER_PAD_H * s;
    let card_inner_w = card_w - CARD_INNER_PAD_H * 2.0 * s;

    let inner_y = draw_section_card(
        painter, text, fox, "Themes", card_x, card_y, card_w, card_h, s, sw, sh,
    );

    let tile_w = TILE_W * s;
    let tile_h = TILE_H * s;
    let gap = TILE_GAP * s;
    let ring_pad = 3.0 * s;
    let cols = tiles_per_row(card_inner_w, tile_w, gap);

    // Lay out theme tiles in a wrap grid; the trailing "+" tile follows in
    // the next free slot.
    for (idx, preset) in state.themes.iter().enumerate() {
        if idx as u32 >= MAX_THEMES {
            break;
        }
        let col = idx % cols;
        let row = idx / cols;
        let tx = card_inner_x + col as f32 * (tile_w + gap);
        let ty = inner_y + row as f32 * (tile_h + gap);

        let zone_id = ZONE_THEME_TILE_BASE + idx as u32;
        let menu_zone = ZONE_THEME_TILE_MENU_BASE + idx as u32;
        let tile_rect = Rect::new(tx, ty, tile_w, tile_h);
        let zone = ix.add_zone(zone_id, tile_rect);
        let is_active = preset.slug == config.appearance.active_theme;
        let thumb_present = state
            .thumbnails
            .get(&preset.slug)
            .map(|t| t.is_some())
            .unwrap_or(false);
        let accent = Color::from_hex(preset.accent_hex()).unwrap_or(fox.accent);

        // 1) Outer ring (painter draws this BEHIND the texture; the visible
        //    margin around the texture shows the ring color).
        let ring_color = if is_active {
            accent
        } else if zone.is_hovered() {
            fox.text_secondary.with_alpha(0.7)
        } else {
            fox.muted.with_alpha(0.3)
        };
        painter.rect_filled(tile_rect, TILE_RADIUS * s, ring_color);

        // 2) Inner rect — texture goes here, or solid fallback if no thumb.
        let inner = Rect::new(
            tile_rect.x + ring_pad,
            tile_rect.y + ring_pad,
            tile_rect.w - ring_pad * 2.0,
            tile_rect.h - ring_pad * 2.0,
        );
        if !thumb_present {
            painter.rect_filled(
                inner,
                (TILE_RADIUS - 2.0).max(2.0) * s,
                accent.with_alpha(0.45),
            );
        }
        state.tile_layouts.push(TileLayout {
            slug: preset.slug.clone(),
            inner,
        });

        // 3) Menu "..." button — registered AFTER tile_zone so it wins for
        //    overlapping clicks. Painter shapes go BENEATH the thumbnail so
        //    the indicator itself is rendered as text (text > textures in
        //    the draw order).
        let menu_size = 28.0 * s;
        let menu_rect = Rect::new(
            tile_rect.x + tile_rect.w - menu_size - 8.0 * s,
            tile_rect.y + 8.0 * s,
            menu_size,
            menu_size,
        );
        let mzone = ix.add_zone(menu_zone, menu_rect);

        // "..." glyph centered in the menu rect. Faux-shadow at +1,+1 for
        // legibility over any wallpaper.
        let dots_sz = 28.0 * s;
        let dots_x = menu_rect.x + (menu_rect.w - dots_sz * 0.7) / 2.0;
        let dots_y = menu_rect.y + (menu_rect.h - dots_sz) / 2.0 - dots_sz * 0.2;
        let bg_alpha = if mzone.is_hovered() {
            0.55
        } else if zone.is_hovered() {
            0.3
        } else {
            0.0
        };
        if bg_alpha > 0.0 {
            // Subtle pill behind the dots when hovered for affordance.
            // Drawn before texture so it's mostly hidden unless the texture
            // is transparent (which it isn't). Skipped — kept for shape ref.
            let _ = bg_alpha;
        }
        text.queue(
            "⋯",
            dots_sz,
            dots_x + 1.0 * s,
            dots_y + 1.0 * s,
            Color::rgba(0.0, 0.0, 0.0, 0.85),
            dots_sz * 2.0,
            sw,
            sh,
        );
        text.queue(
            "⋯",
            dots_sz,
            dots_x,
            dots_y,
            Color::rgba(1.0, 1.0, 1.0, 1.0),
            dots_sz * 2.0,
            sw,
            sh,
        );

        // 4) Name with shadow (text > textures so it's always visible)
        let name = preset.name();
        let name_sz = 16.0 * s;
        let name_x = tile_rect.x + 14.0 * s;
        let name_y = tile_rect.y + tile_rect.h - name_sz - 14.0 * s;
        let name_w = tile_rect.w - 28.0 * s;
        text.queue(
            name,
            name_sz,
            name_x + 1.0 * s,
            name_y + 1.0 * s,
            Color::rgba(0.0, 0.0, 0.0, 0.85),
            name_w,
            sw,
            sh,
        );
        text.queue(
            name,
            name_sz,
            name_x,
            name_y,
            Color::rgba(1.0, 1.0, 1.0, 1.0),
            name_w,
            sw,
            sh,
        );

        // 5) Accent dot in the corner — also text so it sits above the
        //    thumbnail.
        let dot_sz = 16.0 * s;
        text.queue(
            "●",
            dot_sz,
            tile_rect.x + 12.0 * s,
            tile_rect.y + 12.0 * s,
            accent,
            dot_sz * 2.0,
            sw,
            sh,
        );
    }

    // Trailing "+" tile — slots into the next grid cell, wrapping if needed.
    {
        let plus_idx = state.themes.len();
        let col = plus_idx % cols;
        let row = plus_idx / cols;
        let tx = card_inner_x + col as f32 * (tile_w + gap);
        let ty = inner_y + row as f32 * (tile_h + gap);
        let add_rect = Rect::new(tx, ty, tile_w, tile_h);
        let zone = ix.add_zone(ZONE_THEME_ADD, add_rect);
        draw_add_tile(painter, text, fox, add_rect, zone.is_hovered(), s, sw, sh);
    }
}

/// Returns thumbnail TextureDraws borrowing from `state.thumbnails`. Call
/// after `draw_themes_card` so `state.tile_layouts` is populated.
pub(crate) fn collect_theme_thumbs<'a>(state: &'a ThemesPanelState) -> Vec<TextureDraw<'a>> {
    let mut out = Vec::new();
    for layout in &state.tile_layouts {
        if let Some(Some(tex)) = state.thumbnails.get(&layout.slug) {
            out.push(TextureDraw {
                texture: tex,
                x: layout.inner.x,
                y: layout.inner.y,
                w: layout.inner.w,
                h: layout.inner.h,
                opacity: 1.0,
                uv: [0.0, 0.0, 1.0, 1.0],
                clip: None,
            });
        }
    }
    out
}

fn draw_add_tile(
    painter: &mut Painter,
    text: &mut TextRenderer,
    fox: &FoxPalette,
    rect: Rect,
    hovered: bool,
    s: f32,
    sw: u32,
    sh: u32,
) {
    let radius = TILE_RADIUS * s;
    let bg = if hovered { fox.surface_2 } else { fox.surface };
    painter.rect_filled(rect, radius, bg.with_alpha(0.4));
    let ring_color = if hovered {
        fox.accent.with_alpha(0.7)
    } else {
        fox.muted.with_alpha(0.3)
    };
    painter.rect_stroke_sdf(rect, radius, 1.5 * s, ring_color);

    // Big plus
    let cx = rect.x + rect.w / 2.0;
    let cy = rect.y + rect.h / 2.0 - 8.0 * s;
    let arm = 24.0 * s;
    let stroke = 3.0 * s;
    painter.rect_filled(
        Rect::new(cx - arm, cy - stroke / 2.0, arm * 2.0, stroke),
        stroke / 2.0,
        fox.text.with_alpha(0.85),
    );
    painter.rect_filled(
        Rect::new(cx - stroke / 2.0, cy - arm, stroke, arm * 2.0),
        stroke / 2.0,
        fox.text.with_alpha(0.85),
    );

    let label = "Save current as theme";
    let label_sz = 14.0 * s;
    let tw = label_sz * 0.55 * label.len() as f32;
    text.queue(
        label,
        label_sz,
        rect.x + (rect.w - tw) / 2.0,
        cy + arm + 16.0 * s,
        fox.text_secondary,
        rect.w,
        sw,
        sh,
    );
}

// ── Modal ──────────────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_themes_modal(
    state: &mut ThemesPanelState,
    painter: &mut Painter,
    text: &mut TextRenderer,
    ix: &mut InteractionContext,
    fox: &FoxPalette,
    win_w: f32,
    win_h: f32,
    s: f32,
    sw: u32,
    sh: u32,
) {
    if matches!(state.modal, ModalState::Closed) {
        return;
    }

    let (title, value_text, cursor) = match &state.modal {
        ModalState::Save(b) => ("Save Theme", b.text.clone(), b.cursor),
        ModalState::Rename { buffer, .. } => ("Rename Theme", buffer.text.clone(), buffer.cursor),
        ModalState::Closed => return,
    };

    // Backdrop catches clicks → close on outside-click.
    let backdrop = Rect::new(0.0, 0.0, win_w, win_h);
    let _ = ix.add_zone(ZONE_THEME_MODAL_DISMISS, backdrop);
    painter.rect_filled(backdrop, 0.0, Color::rgba(0.0, 0.0, 0.0, 0.55));

    let modal_w = 460.0 * s;
    let modal_h = 220.0 * s;
    let modal_x = (win_w - modal_w) / 2.0;
    let modal_y = (win_h - modal_h) / 2.0;
    let modal_rect = Rect::new(modal_x, modal_y, modal_w, modal_h);

    painter.rect_filled(modal_rect, 14.0 * s, fox.surface);
    painter.rect_stroke_sdf(modal_rect, 14.0 * s, 1.0 * s, fox.muted.with_alpha(0.4));

    // Title
    let title_sz = 20.0 * s;
    text.queue(
        title,
        title_sz,
        modal_x + 24.0 * s,
        modal_y + 24.0 * s,
        fox.text,
        modal_w - 48.0 * s,
        sw,
        sh,
    );

    // Input box
    let input_x = modal_x + 24.0 * s;
    let input_y = modal_y + 72.0 * s;
    let input_w = modal_w - 48.0 * s;
    let input_h = 48.0 * s;
    let input_rect = Rect::new(input_x, input_y, input_w, input_h);
    let _zone = ix.add_zone(ZONE_THEME_MODAL_INPUT, input_rect);
    state.input_rect = Some(input_rect);

    painter.rect_filled(input_rect, 8.0 * s, fox.surface_2);
    painter.rect_stroke_sdf(input_rect, 8.0 * s, 1.5 * s, fox.accent.with_alpha(0.8));

    let text_sz = 18.0 * s;
    let display: &str = &value_text;
    text.queue(
        display,
        text_sz,
        input_x + 14.0 * s,
        input_y + (input_h - text_sz) / 2.0,
        fox.text,
        input_w - 28.0 * s,
        sw,
        sh,
    );

    // Cursor caret
    let prefix: String = display.chars().take(cursor).collect();
    let caret_x = input_x + 14.0 * s + text_sz * 0.55 * prefix.chars().count() as f32;
    painter.rect_filled(
        Rect::new(caret_x, input_y + 10.0 * s, 2.0 * s, input_h - 20.0 * s),
        1.0 * s,
        fox.text,
    );

    // Buttons
    let btn_h = 40.0 * s;
    let btn_w = 110.0 * s;
    let btn_gap = 12.0 * s;
    let btn_y = modal_y + modal_h - btn_h - 24.0 * s;
    let save_x = modal_x + modal_w - 24.0 * s - btn_w;
    let cancel_x = save_x - btn_gap - btn_w;

    let cancel_rect = Rect::new(cancel_x, btn_y, btn_w, btn_h);
    let cancel_zone = ix.add_zone(ZONE_THEME_MODAL_CANCEL, cancel_rect);
    Button::new(cancel_rect, "Cancel")
        .variant(ButtonVariant::Ghost)
        .hovered(cancel_zone.is_hovered())
        .pressed(cancel_zone.is_active())
        .scale(s)
        .draw(painter, text, fox, sw, sh);

    let save_rect = Rect::new(save_x, btn_y, btn_w, btn_h);
    let save_zone = ix.add_zone(ZONE_THEME_MODAL_SAVE, save_rect);
    Button::new(save_rect, "Save")
        .variant(ButtonVariant::Primary)
        .hovered(save_zone.is_hovered())
        .pressed(save_zone.is_active())
        .scale(s)
        .draw(painter, text, fox, sw, sh);
}

// ── Click handling ─────────────────────────────────────────────────────────

/// Returns true if the click was consumed by the themes UI (either a tile
/// action or the open modal eating clicks). When `false`, callers should
/// keep dispatching to other panel handlers.
pub fn handle_themes_click(
    state: &mut ThemesPanelState,
    config: &mut LanternConfig,
    panel_state: &mut PanelState,
    zone_id: u32,
    cx: f32,
    cy: f32,
) -> bool {
    // Modal is drawn on top and intercepts every click on the surface.
    if state.modal_open() {
        match zone_id {
            ZONE_THEME_MODAL_SAVE => {
                state.confirm_modal(config);
            }
            ZONE_THEME_MODAL_CANCEL | ZONE_THEME_MODAL_DISMISS => {
                state.modal = ModalState::Closed;
            }
            ZONE_THEME_MODAL_INPUT => {
                // Click in input box — focus is implicit while modal is open.
            }
            _ => {}
        }
        return true;
    }

    if zone_id == ZONE_THEME_ADD {
        state.modal = ModalState::Save(TextBuffer::new(""));
        return true;
    }

    if zone_id >= ZONE_THEME_TILE_BASE && zone_id < ZONE_THEME_TILE_BASE + MAX_THEMES {
        let idx = (zone_id - ZONE_THEME_TILE_BASE) as usize;
        if let Some(preset) = state.themes.get(idx) {
            themes::apply_theme(preset, config);
            config.save();
            return true;
        }
    }

    if zone_id >= ZONE_THEME_TILE_MENU_BASE && zone_id < ZONE_THEME_TILE_MENU_BASE + MAX_THEMES {
        let idx = (zone_id - ZONE_THEME_TILE_MENU_BASE) as usize;
        if let Some(preset) = state.themes.get(idx) {
            state.context_target = Some(preset.slug.clone());
            let total = state.themes.len();
            let items = build_context_menu_items(idx, total);
            panel_state.dropdown_menu.open(cx, cy + 16.0, items);
            return true;
        }
    }

    false
}

fn build_context_menu_items(idx: usize, total: usize) -> Vec<lntrn_ui::gpu::MenuItem> {
    use lntrn_ui::gpu::MenuItem;
    let at_first = idx == 0;
    let at_last = idx + 1 >= total;
    vec![
        MenuItem::action(ACT_THEME_RENAME, "Rename"),
        MenuItem::action(ACT_THEME_UPDATE, "Update from current"),
        MenuItem::Separator,
        if at_first {
            MenuItem::Action {
                id: ACT_THEME_MOVE_LEFT,
                label: "Move Left".into(),
                shortcut: None,
                enabled: false,
                danger: false,
            }
        } else {
            MenuItem::action(ACT_THEME_MOVE_LEFT, "Move Left")
        },
        if at_last {
            MenuItem::Action {
                id: ACT_THEME_MOVE_RIGHT,
                label: "Move Right".into(),
                shortcut: None,
                enabled: false,
                danger: false,
            }
        } else {
            MenuItem::action(ACT_THEME_MOVE_RIGHT, "Move Right")
        },
        MenuItem::Separator,
        MenuItem::Action {
            id: ACT_THEME_DELETE,
            label: "Delete".into(),
            shortcut: None,
            enabled: true,
            danger: true,
        },
    ]
}

/// Handle a context-menu action ID. Returns true if it was a theme action.
/// Called from the appearance panel's shared dropdown event dispatch.
pub fn dispatch_theme_menu_action(
    state: &mut ThemesPanelState,
    config: &mut LanternConfig,
    action_id: u32,
) -> bool {
    if action_id < ACT_THEME_RENAME || action_id > ACT_THEME_DELETE {
        return false;
    }
    let Some(slug) = state.context_target.clone() else {
        return true;
    };
    let preset = state.themes.iter().find(|t| t.slug == slug).cloned();

    match action_id {
        ACT_THEME_RENAME => {
            if let Some(p) = preset {
                state.modal = ModalState::Rename {
                    slug: p.slug.clone(),
                    buffer: TextBuffer::new(p.name()),
                };
            }
        }
        ACT_THEME_UPDATE => {
            if let Some(p) = preset {
                let _ = themes::update_theme(&p.slug, p.name(), config);
                state.needs_reload = true;
            }
        }
        ACT_THEME_MOVE_LEFT => {
            let _ = themes::move_theme(&slug, MoveDir::Left);
            state.needs_reload = true;
        }
        ACT_THEME_MOVE_RIGHT => {
            let _ = themes::move_theme(&slug, MoveDir::Right);
            state.needs_reload = true;
        }
        ACT_THEME_DELETE => {
            let _ = themes::delete_theme(&slug);
            if config.appearance.active_theme == slug {
                config.appearance.active_theme.clear();
                config.save();
            }
            state.needs_reload = true;
        }
        _ => {}
    }
    state.context_target = None;
    true
}

// ── Thumbnail loading ─────────────────────────────────────────────────────

fn ensure_thumbnails(state: &mut ThemesPanelState, tex_pass: &TexturePass, gpu: &GpuContext) {
    for preset in &state.themes {
        if state.thumbnails.contains_key(&preset.slug) {
            continue;
        }

        let tex = preset.wallpaper().and_then(|wp| {
            let path = PathBuf::from(wp);
            if !path.exists() {
                return None;
            }
            decode_thumbnail(&path).map(|rgba| tex_pass.upload(gpu, &rgba, THUMB_W, THUMB_H))
        });
        state.thumbnails.insert(preset.slug.clone(), tex);
    }
}

fn decode_thumbnail(path: &std::path::Path) -> Option<Vec<u8>> {
    use image::GenericImageView;
    let img = image::open(path).ok()?;
    let (src_w, src_h) = img.dimensions();
    let scale = (THUMB_W as f32 / src_w as f32).max(THUMB_H as f32 / src_h as f32);
    let scaled_w = (src_w as f32 * scale).ceil() as u32;
    let scaled_h = (src_h as f32 * scale).ceil() as u32;
    let resized = img.resize_exact(scaled_w, scaled_h, image::imageops::FilterType::Triangle);
    let crop_x = scaled_w.saturating_sub(THUMB_W) / 2;
    let crop_y = scaled_h.saturating_sub(THUMB_H) / 2;
    let cropped = resized.crop_imm(crop_x, crop_y, THUMB_W, THUMB_H);
    Some(cropped.to_rgba8().into_raw())
}

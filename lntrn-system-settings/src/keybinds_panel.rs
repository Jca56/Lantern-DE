//! Keybindings panel — configures the "Lantern layer" (a hold-to-activate
//! navigation layer for 60% keyboards whose firmware `fn` key never reaches
//! the OS) plus a read-only reference list of the compositor's shortcuts.
//!
//! The layer turns ordinary keys into navigation keys while a trigger key is
//! held: by default Menu (right of `fn`) + WASD → arrows, with Q/E→Home/End,
//! R/F→PageUp/PageDown, X→Delete, Z→Backspace. The compositor reads the same
//! `[keybinds]` config (see `lntrn-compositor/src/input/layer.rs`).
//!
//! Per-machine scope: when "This machine only" is on, the enable flag is
//! written under `[keybinds.machine.<hostname>]` so the desktop can run the
//! layer without affecting the laptop.

use lntrn_render::{Painter, Rect, TextRenderer};
use lntrn_ui::gpu::{
    Button, ButtonVariant, FoxPalette, InteractionContext, ScrollArea, Scrollbar, Toggle,
};

use crate::config::{default_layer_maps, machine_hostname, LanternConfig};
use crate::panels::{
    draw_section_card, CARD_GAP, CARD_HEADER_H, CARD_INNER_PAD_H, CARD_INNER_PAD_V,
    CARD_OUTER_PAD_H, CARD_OUTER_PAD_V, LABEL_SIZE, ROW_H, TOGGLE_H, VALUE_SIZE,
};

// Zone IDs — kept in their own band (1000+) to avoid colliding with other
// panels (notifications use 699–720, input 800–970, Save/Cancel 900/901).
const ZONE_LAYER_ENABLED: u32 = 1000;
const ZONE_LAYER_KEY_BASE: u32 = 1010; // +0..LAYER_KEYS.len()
const ZONE_MAP_CYCLE_BASE: u32 = 1030; // +row index; cycles that row's target
const ZONE_MAP_RESET: u32 = 1029;

/// Selectable trigger keys, as (config name, display label). Order matches the
/// button row drawn in the Trigger Key card.
const LAYER_KEYS: &[(&str, &str)] = &[
    ("Menu", "Menu"),
    ("Super", "Super"),
    ("Caps", "Caps Lock"),
    ("RAlt", "Right Alt"),
];

/// The nav targets a map row can cycle through. Display uses arrow glyphs so
/// the WASD→arrow mapping reads at a glance.
const TARGETS: &[(&str, &str)] = &[
    ("Up", "\u{2191} Up"),
    ("Down", "\u{2193} Down"),
    ("Left", "\u{2190} Left"),
    ("Right", "\u{2192} Right"),
    ("Home", "Home"),
    ("End", "End"),
    ("PageUp", "Page Up"),
    ("PageDown", "Page Down"),
    ("Delete", "Delete"),
    ("Backspace", "Backspace"),
];

/// Read-only reference list of the compositor's current shortcuts. Phase 1
/// shows these for orientation; rebinding them lands in a follow-up.
const SHORTCUTS: &[(&str, &str)] = &[
    ("Super (tap)", "Open Command Center"),
    ("Super + Return", "Open Terminal"),
    ("Super + Alt + Return", "Open File Manager"),
    ("Super + Q", "Close focused window"),
    ("Super + \u{2191}/\u{2193}", "Grow / shrink window"),
    ("Super + Shift + Arrow", "Snap window to edge"),
    ("Super + Ctrl + Arrow", "Swap with neighbour"),
    ("Super + Alt + \u{2191}/\u{2193}", "Maximize / minimize"),
    (
        "Super + Alt + \u{2190}/\u{2192}",
        "Previous / next workspace",
    ),
    ("Super + 1-9", "Switch to workspace"),
    ("Super + Shift + 1-9", "Move window to workspace"),
    ("Alt + Tab", "Window switcher"),
    ("F11 / Super + F", "Toggle fullscreen"),
    ("Super + Print", "Screen recorder"),
    ("Print", "Screenshot"),
    ("Super + Shift + C", "Restart compositor"),
];

pub struct KeybindsPanelState {
    pub scroll: f32,
    /// Hostname of the machine we're configuring. Each machine has its own
    /// independent keybinds under `[keybinds.machine.<host>]`, so this page
    /// always edits the local machine — the laptop and desktop never collide.
    host: String,
}

impl KeybindsPanelState {
    pub fn new() -> Self {
        Self {
            scroll: 0.0,
            host: machine_hostname(),
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub fn draw_keybinds_panel(
    config: &mut LanternConfig,
    state: &mut KeybindsPanelState,
    painter: &mut Painter,
    text: &mut TextRenderer,
    ix: &mut InteractionContext,
    fox: &FoxPalette,
    x: f32,
    y: f32,
    w: f32,
    panel_h: f32,
    s: f32,
    sw: u32,
    sh: u32,
    scroll_delta: f32,
) {
    let row = ROW_H * s;
    let lsz = LABEL_SIZE * s;
    let vsz = VALUE_SIZE * s;

    let card_x = x + CARD_OUTER_PAD_H * s;
    let card_w = w - CARD_OUTER_PAD_H * 2.0 * s;
    let card_inner_x = card_x + CARD_INNER_PAD_H * s;
    let card_inner_w = card_w - CARD_INNER_PAD_H * 2.0 * s;
    let card_chrome_h = CARD_HEADER_H * s + CARD_INNER_PAD_V * 2.0 * s;

    let enabled = config.keybinds.effective_enabled(&state.host);

    // Card heights. The map list + trigger key cards only show when enabled,
    // so toggling off collapses the page to the essentials.
    let layer_card_h = card_chrome_h + row + vsz + 12.0 * s; // toggle + 1 hint line
    let trigger_card_h = card_chrome_h + row * 1.6;
    let maps_card_h = card_chrome_h + (config.keybinds.layer_maps.len() as f32 + 1.0) * row;
    let shortcuts_card_h = card_chrome_h + SHORTCUTS.len() as f32 * (row * 0.78);

    let mut heights = vec![layer_card_h];
    if enabled {
        heights.push(trigger_card_h);
        heights.push(maps_card_h);
    }
    heights.push(shortcuts_card_h);

    let content_height = CARD_OUTER_PAD_V * 2.0 * s
        + heights.iter().sum::<f32>()
        + CARD_GAP * s * heights.len().saturating_sub(1) as f32;

    if scroll_delta != 0.0 {
        ScrollArea::apply_scroll(
            &mut state.scroll,
            scroll_delta * 40.0,
            content_height,
            panel_h,
        );
    }

    let viewport = Rect::new(x, y, w, panel_h);
    let scroll_area = ScrollArea::new(viewport, content_height, &mut state.scroll);
    scroll_area.begin(painter, text);
    let mut cy_top = scroll_area.content_y() + CARD_OUTER_PAD_V * s;

    // ── Arrow Key Layer (enable) ────────────────────────────────────────
    {
        let mut cy = draw_section_card(
            painter,
            text,
            fox,
            "Arrow Key Layer",
            card_x,
            cy_top,
            card_w,
            layer_card_h,
            s,
            sw,
            sh,
        );
        // Enable toggle — short and plain.
        {
            let rect = Rect::new(card_inner_x, cy, card_inner_w, TOGGLE_H * s);
            let toggle = Toggle::new(rect, enabled)
                .label("Enable Menu + WASD arrow keys")
                .scale(s);
            let track = toggle.track_rect();
            let zone = ix.add_zone(ZONE_LAYER_ENABLED, track);
            toggle
                .hovered(zone.is_hovered())
                .draw(painter, text, fox, sw, sh);
            cy += row;
        }
        // One-line note: these settings are local to this machine.
        let note = format!("Settings apply to this computer ({}).", state.host);
        text.queue(
            &note,
            vsz,
            card_inner_x,
            cy,
            fox.text_secondary,
            card_inner_w,
            sw,
            sh,
        );
        cy_top += layer_card_h + CARD_GAP * s;
    }

    // ── Trigger Key ─────────────────────────────────────────────────────
    if enabled {
        let cy = draw_section_card(
            painter,
            text,
            fox,
            "Trigger Key",
            card_x,
            cy_top,
            card_w,
            trigger_card_h,
            s,
            sw,
            sh,
        );
        let hint = "Hold this key, then tap a mapped key. (fn is firmware-locked, so Menu is the default.)";
        text.queue(
            hint,
            vsz,
            card_inner_x,
            cy,
            fox.text_secondary,
            card_inner_w,
            sw,
            sh,
        );

        let btn_w = 130.0 * s;
        let btn_h = 40.0 * s;
        let btn_gap = 10.0 * s;
        let btn_y = cy + vsz + 12.0 * s;
        for (i, (name, label)) in LAYER_KEYS.iter().enumerate() {
            let bx = card_inner_x + i as f32 * (btn_w + btn_gap);
            let rect = Rect::new(bx, btn_y, btn_w, btn_h);
            let zone = ix.add_zone(ZONE_LAYER_KEY_BASE + i as u32, rect);
            let selected = config.keybinds.layer_key.eq_ignore_ascii_case(name);
            Button::new(rect, label)
                .variant(if selected {
                    ButtonVariant::Primary
                } else {
                    ButtonVariant::Ghost
                })
                .hovered(zone.is_hovered())
                .pressed(zone.is_active())
                .scale(s)
                .draw(painter, text, fox, sw, sh);
        }
        cy_top += trigger_card_h + CARD_GAP * s;
    }

    // ── Key Maps ────────────────────────────────────────────────────────
    if enabled {
        let mut cy = draw_section_card(
            painter,
            text,
            fox,
            "Key Maps",
            card_x,
            cy_top,
            card_w,
            maps_card_h,
            s,
            sw,
            sh,
        );
        // Header hint + Reset to defaults button
        {
            text.queue(
                "Hold the trigger, tap the source key \u{2192} sends:",
                vsz,
                card_inner_x,
                cy + (row - vsz) / 2.0,
                fox.text_secondary,
                card_inner_w - 160.0 * s,
                sw,
                sh,
            );
            let btn_w = 150.0 * s;
            let btn_h = 36.0 * s;
            let rect = Rect::new(
                card_inner_x + card_inner_w - btn_w,
                cy + (row - btn_h) / 2.0,
                btn_w,
                btn_h,
            );
            let zone = ix.add_zone(ZONE_MAP_RESET, rect);
            Button::new(rect, "Reset Defaults")
                .variant(ButtonVariant::Ghost)
                .hovered(zone.is_hovered())
                .pressed(zone.is_active())
                .scale(s)
                .draw(painter, text, fox, sw, sh);
            cy += row;
        }

        for (i, pair) in config.keybinds.layer_maps.iter().enumerate() {
            let (from, to) = pair.split_once('=').unwrap_or((pair.as_str(), ""));
            // Source key label, e.g. "W"
            let src = from.trim().to_ascii_uppercase();
            text.queue(
                &src,
                lsz,
                card_inner_x,
                cy + (row - lsz) / 2.0,
                fox.text,
                60.0 * s,
                sw,
                sh,
            );
            // Arrow separator
            text.queue(
                "\u{2192}",
                lsz,
                card_inner_x + 64.0 * s,
                cy + (row - lsz) / 2.0,
                fox.text_secondary,
                30.0 * s,
                sw,
                sh,
            );
            // Target as a cycle button
            let to_label = TARGETS
                .iter()
                .find(|(n, _)| n.eq_ignore_ascii_case(to.trim()))
                .map(|(_, l)| *l)
                .unwrap_or(to.trim());
            let btn_w = 170.0 * s;
            let btn_h = 36.0 * s;
            let rect = Rect::new(
                card_inner_x + 104.0 * s,
                cy + (row - btn_h) / 2.0,
                btn_w,
                btn_h,
            );
            let zone = ix.add_zone(ZONE_MAP_CYCLE_BASE + i as u32, rect);
            Button::new(rect, to_label)
                .variant(ButtonVariant::Ghost)
                .hovered(zone.is_hovered())
                .pressed(zone.is_active())
                .scale(s)
                .draw(painter, text, fox, sw, sh);
            cy += row;
        }
        cy_top += maps_card_h + CARD_GAP * s;
    }

    // ── Shortcuts (read-only reference) ─────────────────────────────────
    {
        let mut cy = draw_section_card(
            painter,
            text,
            fox,
            "Shortcuts (reference)",
            card_x,
            cy_top,
            card_w,
            shortcuts_card_h,
            s,
            sw,
            sh,
        );
        let line_h = row * 0.78;
        let chord_w = 240.0 * s;
        for (chord, desc) in SHORTCUTS {
            text.queue(
                chord,
                vsz,
                card_inner_x,
                cy + (line_h - vsz) / 2.0,
                fox.text,
                chord_w,
                sw,
                sh,
            );
            text.queue(
                desc,
                vsz,
                card_inner_x + chord_w,
                cy + (line_h - vsz) / 2.0,
                fox.text_secondary,
                card_inner_w - chord_w,
                sw,
                sh,
            );
            cy += line_h;
        }
        let _ = cy;
    }

    scroll_area.end(painter, text);

    if scroll_area.is_scrollable() {
        let sb = Scrollbar::new(&viewport, content_height, state.scroll);
        sb.draw(painter, lntrn_ui::gpu::InteractionState::Idle, fox);
    }
}

pub fn handle_keybinds_click(
    config: &mut LanternConfig,
    state: &mut KeybindsPanelState,
    zone_id: u32,
) {
    match zone_id {
        ZONE_LAYER_ENABLED => {
            // Always scope to this machine — each computer has its own setup.
            let cur = config.keybinds.effective_enabled(&state.host);
            config.keybinds.set_enabled(!cur, Some(state.host.as_str()));
        }
        ZONE_MAP_RESET => {
            config.keybinds.layer_maps = default_layer_maps();
        }
        z if (ZONE_LAYER_KEY_BASE..ZONE_LAYER_KEY_BASE + LAYER_KEYS.len() as u32).contains(&z) => {
            let i = (z - ZONE_LAYER_KEY_BASE) as usize;
            config.keybinds.layer_key = LAYER_KEYS[i].0.to_string();
        }
        z if (ZONE_MAP_CYCLE_BASE..ZONE_MAP_CYCLE_BASE + 64).contains(&z) => {
            let i = (z - ZONE_MAP_CYCLE_BASE) as usize;
            if let Some(pair) = config.keybinds.layer_maps.get_mut(i) {
                if let Some((from, to)) = pair.clone().split_once('=') {
                    // Advance to the next target in TARGETS, wrapping.
                    let cur = TARGETS
                        .iter()
                        .position(|(n, _)| n.eq_ignore_ascii_case(to.trim()));
                    let next = cur.map(|c| (c + 1) % TARGETS.len()).unwrap_or(0);
                    *pair = format!("{}={}", from.trim(), TARGETS[next].0);
                }
            }
        }
        _ => {}
    }
}

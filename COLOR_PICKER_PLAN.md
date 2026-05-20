# Color Picker Modal + Drag-and-Drop Plan

Supersedes the old "Full Color Picker" doc — that aimed at replacing the
swatch row outright. The new design *adds* a picker that drops into the
existing swatch system via drag-and-drop, instead of replacing it.

## Decisions (locked)

- **Picker style**: HSV square + vertical hue strip (Photoshop/Figma feel)
- **Palette overrides**: per-card (each card maintains its own override list,
  so editing the Background card's swatches doesn't affect the Gradient card)
- **Drag visual**: filled color circle follows cursor + drop targets ring
- **Picker button location**: top-right of each card header (one per card)
- **Hex field**: read-only display for v1 (editable text input later)
- **Modal lifecycle**: stays open after a drop so user can batch-edit
- **Initial picker color**: seeded from the card's current selection

## Phases

### 1. HSV ↔ RGB math
`lntrn-render/draw/src/color.rs`:
- `Color::to_hsv() -> (f32, f32, f32)` (hue 0-360, sat 0-1, val 0-1)
- `Color::from_hsv(h, s, v) -> Color`

### 2. Picker + drag state on `PanelState`
```rust
pub struct ColorPickerState {
    pub open: bool,
    pub origin_card: Option<CardId>,
    pub h: f32, pub s: f32, pub v: f32,
}
pub struct DragState {
    pub color: Color,
    pub start_zone: u32,
}
```

### 3. Per-card "+" button in section headers
Extend `draw_section_card` with an optional picker button param.
Each card has its own `ZONE_PICKER_BUTTON_*` constant.

### 4. HSV modal UI (`color_picker_modal.rs` new file)
- HSV square: 2 stacked shader rects (white→hue, then 0→black overlay)
- Hue strip: stack of 6 linear-gradient quads OR new shader path
- Picker indicators (ring outlines)
- Preview circle (drag handle)
- Read-only hex display
- X close button
- Modal backdrop (translucent black) closes on outside click

### 5. Drag-and-drop framework
- Detect drag start: preview-circle zone enters `Dragging` state
- While dragging: draw ghost circle at `ix.cursor()`
- On left-release in `wayland.rs`: if drag is some, find hovered drop-target zone, fire drop handler
- Drop-target zones registered in a small registry for hover-hit-testing

### 6. Per-card palette override config
```rust
#[derive(Default, Serialize, Deserialize)]
pub struct PaletteOverrides {
    pub background: Vec<String>,
    pub gradient: Vec<String>,
    pub border: Vec<String>,
    pub tint: Vec<String>,
    pub glow: Vec<String>,
}
```
Index-parallel to the default palette; empty string at index i = "use default[i]".
Helper `effective_palette(defaults, overrides) -> Vec<(String, String)>`.

### 7. Drop handlers
- Drop on swatch in card X, index i → `overrides.<card>[i] = drag.color`
- Drop on gradient chip P → `window_gradient_stops[P] = drag.color`
- `config.save()` after mutation

### 8. Drop-target highlight
Each swatch / chip, when drag is active AND cursor over it,
draws an outer pulse ring via `sin(time * 4)` alpha mod.

### 9. (Future) Mouse panel integration
Same picker + drag system, register mouse-panel swatches as drop targets.

## Card IDs (for override storage)
- `background` — Appearance > Background Color
- `gradient` — Appearance > Window Gradient (shared color row)
- `border` — Appearance > Window > Border Color
- `tint` — Appearance > Window > Blur Tint Color
- `glow` — Appearance > Window > Focus Glow Color

## Estimate
~4-5 hours, 2-3 deploy cycles.

---

# Resumption Notes (read these FIRST on next session)

You're a fresh Claude with no context. Read this whole section, then read
`/home/alva/Projects/Lantern-DE/CLAUDE.md` for the project conventions
(file size limits, theme/UI conventions, multi-machine notes).

## State of the codebase as of this plan

The gradient overhaul shipped — Window Gradient now uses **5 independent
radial glows** (TL, TR, BL, BR, Center), driven by chip toggles + shared
color row + Intensity + Radius sliders. Nothing in that system needs to
change for the picker — you're adding ON TOP of it.

Other recent work that shipped:
- Window controls: Super+Arrow = resize (aspect-locked), Super+Shift+Arrow = move,
  Super+Alt+Arrow = max/min/workspace. Plus 9-zone cell-by-cell move logic.
- Tiling subsystem deleted entirely. Don't look for it.
- Drag-snap preview overlay removed (was causing PC freezes).
- Gradient shader has dithering applied to all gradient paths.
- Default cursor bug fixed (was loading xcursor blue triangle on session start).

## Key files (open these on resumption)

| Purpose | Path | Lines of interest |
|---|---|---|
| Color struct + hex parsing | `lntrn-render/draw/src/color.rs` | `from_hex` line 46; add `to_hsv`/`from_hsv` here |
| Modal widget (already exists!) | `lntrn-ui/src/gpu/modal.rs` | struct at line 52; `.backdrop_rect`, `.panel_rect`, `.button_rect` |
| InteractionContext + drag state machine | `lntrn-ui/src/gpu/input.rs` | struct line 35; `InteractionState` enum line 5 (has `Pressed`/`Dragging`); `on_left_pressed` line 111; `on_left_released` line 121 |
| Wayland input loop (where mouse events route) | `lntrn-system-settings/src/wayland.rs` | cursor move line 265; click routing line 366-372 |
| Appearance panel main render | `lntrn-system-settings/src/appearance_panel.rs` | `draw_appearance_panel` is the entry; gradient card around line 415; `handle_appearance_click` around line 700 |
| PanelState (add picker + drag state here) | `lntrn-system-settings/src/panels.rs` | `PanelState` struct ~line 79; `BG_COLORS` at line 50; `GLOW_COLORS` at line 33 |
| Config schema + persistence | `lntrn-system-settings/src/config.rs` | `AppearanceConfig` line 76; `LanternConfig::save()` line 474; `::load()` line 461 |
| Section card chrome (where to add + button) | `lntrn-system-settings/src/panels.rs` | `draw_section_card` line 246; `CARD_HEADER_H` line 70; `CARD_INNER_PAD_H` line 68 |
| Existing slider drag pattern (mirror this) | `lntrn-system-settings/src/panels.rs` | `slider_value_from_cursor` line 102 — uses `is_active()` + `ix.cursor()` |
| Existing dropdown menu (similar lifecycle) | `lntrn-ui/src/gpu/context_menu.rs` | `ContextMenu` struct line 151; `open`/`close`/`contains`/`draw` |
| Click router (outside-click logic example) | `lntrn-system-settings/src/wayland.rs` | line 369 — short-circuit before panel handler |

## Patterns to mirror

- **Frame-based render** — every frame redraws everything from config state.
  No "diff" logic. If you change `panel_state.color_picker.open = true`,
  the next frame renders the modal. Read state, render, repeat.
- **Zones registered each frame** — call `ix.add_zone(zone_id, rect)` in
  every render of an interactive region, every frame. Returns the current
  state (Idle / Hovered / Pressed / Dragging).
- **Click handling separate from render** — render registers zones; click
  events arrive separately via `on_left_pressed() -> Option<u32>` (the
  topmost hit zone id) and are routed to `handle_appearance_click` etc.
- **Config save pattern** — mutate `config.appearance.*`, then `config.save()`.
  Atomic write of the whole TOML to `~/.lantern/config/lantern.toml`.

## Build + deploy

```bash
cd /home/alva/Projects/Lantern-DE
cargo build --release -p lntrn-system-settings
cp target/release/lntrn-system-settings /tmp/lntrn-system-settings-new && \
  mv -f /tmp/lntrn-system-settings-new ~/.lantern/bin/lntrn-system-settings
```

If `lntrn-render` (where Color lives) changed too, just `cargo build` from
the root — it's a workspace, all consumers pick it up. Then deploy any
running app that uses the picker (initially just system-settings).

The `/tmp` move-trick is needed because Rust can't overwrite a running
binary directly (`Text file busy` error).

## Where to start (Cycle 1)

1. **Phase 1** — add `to_hsv()`/`from_hsv()` to `Color` in
   `lntrn-render/draw/src/color.rs`. Standard HSV math. Probably 30 lines.
2. **Phase 2** — add `ColorPickerState` + `DragState` to `PanelState` in
   `lntrn-system-settings/src/panels.rs`. Initialize defaults.
3. **Phase 3** — modify `draw_section_card` (or wrap it) so cards can opt
   in to a top-right "+" picker button. Each card that wants one passes
   its `CardId`. Click handler in `handle_appearance_click` opens the
   modal with that card as origin.
4. **Phase 4** — new file `lntrn-system-settings/src/color_picker_modal.rs`.
   Render an HSV square + hue strip in a centered panel. The HSV square
   can be built from 2 stacked gradient quads:
   - White → hue color (horizontal linear gradient at sat axis)
   - Transparent black → opaque black (vertical) on top
   The hue strip is a stack of 6 linear gradient quads or a new shader
   path `SHAPE_HUE_STRIP` that maps Y to hue then converts HSV→RGB
   inline (probably faster than 6 quads).

Cycle 1 deploy goal: button visible, modal opens, HSV picker works
visually (no drag yet). Verify before plumbing drag in Cycle 2.

## Gotchas

- **File size limit is 600 lines, flagged at 500.** `appearance_panel.rs`
  is already long — when adding picker logic, factor into helpers, or
  consider splitting into `appearance_panel.rs` + `color_picker_modal.rs`
  (planned).
- **`InteractionContext` has no native mouse-release event for "drop"** —
  use the `on_left_released` call in `wayland.rs` as a hook: when release
  fires AND `panel_state.drag.is_some()`, scan all registered zones,
  find the topmost containing the cursor, fire `handle_drop(zone_id, color)`.
- **Modal rendering is on top of everything** — render it AFTER all the
  panel cards but BEFORE any debug overlays. The `Modal` widget in
  `lntrn-ui` already handles the backdrop dim layer.
- **Per-card overrides are index-parallel to defaults** — `overrides.background[3]`
  overrides `BG_COLORS[3]`. Empty string = use default. Don't reshape
  the arrays.
- **The hue strip is the trickiest visual** — if a 6-stop linear gradient
  feels too segmented, a custom shader path `SHAPE_HUE_STRIP` that does
  `t -> hsv(t * 360, 1, 1) -> rgb` per pixel gives a perfectly smooth
  rainbow. Look at the existing dither helper in `lntrn-render/draw/src/shader.rs`
  for shader pattern reference. New shape IDs go in both `painter.rs`
  AND `shader.rs` at the top.

## Things I considered and rejected

- **Editable hex text input** — punted to v2 because no text input widget
  exists yet and building one is a 2-hour rabbit hole. Read-only display
  is fine for v1.
- **Eyedropper tool** — would require a Wayland screenshot capability
  on the picker side. Not in scope.
- **System-wide palette** — user explicitly chose per-card so we don't
  build the global one.
- **Color wheel UI** — user picked HSV square + hue strip. Don't redesign.

## Quick smoke test on resumption

Before writing any code, verify the codebase is in the expected state:
```bash
grep -n "fn to_hsv\|fn from_hsv" /home/alva/Projects/Lantern-DE/lntrn-render/draw/src/color.rs
# Should return nothing — confirms Phase 1 not done yet
grep -n "pub struct ColorPickerState\|pub struct DragState" /home/alva/Projects/Lantern-DE/lntrn-system-settings/src/panels.rs
# Should return nothing — confirms Phase 2 not done yet
ls /home/alva/Projects/Lantern-DE/lntrn-system-settings/src/color_picker_modal.rs
# Should error — confirms Phase 4 not done yet
```

If any of those return something, someone (you, in a previous session)
started the work — read the existing code before continuing.

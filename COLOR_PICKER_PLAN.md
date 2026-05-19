# Full Color Picker — Implementation Plan

## Goal

Replace (or augment) every swatch-row color picker in System Settings with
a full HSV picker so the user can dial in **any** color, not just the 8
presets in `GLOW_COLORS`. Live preview, hex input, plus quick-presets
stay available for one-click theming.

## Scope — all color settings in `lantern.toml`

| Field | Section | Currently |
|---|---|---|
| `appearance.accent` | Appearance | Theme preset |
| `appearance.background_color` | Appearance | Background palette swatches |
| `window_manager.border_color` | WM | GLOW swatch row |
| `window_manager.focus_glow_color` | WM | GLOW swatch row |
| `windows.blur_tint_color` | Windows | GLOW swatch row |
| `input.cursor_body_light` | Mouse | GLOW swatch row |
| `input.cursor_body_dark` | Mouse | GLOW swatch row |
| `input.cursor_accent_light` | Mouse | GLOW swatch row |
| `input.cursor_accent_dark` | Mouse | GLOW swatch row |
| `input.cursor_outline_color` | Mouse | GLOW swatch row |
| `input.click_anim_color` | Mouse | GLOW swatch row |

11 fields total. All store hex strings (`"#RRGGBB"`). No schema change
needed — the picker just outputs hex.

## UI design

```
Row layout (replacing or augmenting current swatch rows):

  Label          [●]  ●●●●●●●●  [more]
                 ^    ^^^^^^^^   ^
                 |    GLOW       click to expand the picker
                 |    presets    (or hold Shift+click on preset row)
                 current color tile
                 (click → open picker)

Expanded picker (slides in below the row):

  ┌─────────────────────────────────────────────┐
  │ ┌────────────────┐  H │░░░░░░░│   #FFC800   │
  │ │                │    │░██░░░░│   ┌───────┐ │
  │ │   SV plane     │    │░░░░░░░│   │ Hex   │ │
  │ │  (sat × value, │    │░░██░░░│   │ input │ │
  │ │   hue tinted)  │    │░░░░░░│    └───────┘ │
  │ │                │    │░░░░██│              │
  │ │            [●] │    │░░░░░░│   [R G B HSV]│
  │ └────────────────┘    └──────┘              │
  │                                              │
  │  S: 0.95   V: 0.78    H: 47°                │
  │                                              │
  │  ●●●●●●●● ← still-visible GLOW palette       │
  │                                              │
  │                       [Close]               │
  └─────────────────────────────────────────────┘
```

- **SV plane**: 2D area, X = saturation, Y = value (inverted). Background
  is the current hue at full S+V. Two gradient overlays:
  1. White → transparent left-to-right (S axis)
  2. Transparent → black top-to-bottom (V axis)
  Draggable indicator dot snaps to cursor pos.
- **Hue slider**: vertical (or horizontal) full-rainbow bar, draggable.
  Use `rect_gradient_multi_direct` with 7 stops (red→yellow→green→cyan→blue→magenta→red).
- **Hex input**: text field showing current color in `#RRGGBB` format.
  Typing a valid hex updates the SV+hue indicators.
- **R G B / H S V readouts**: small text labels with current values.
  Optional toggle to display either.
- **Preset row**: the existing `GLOW_COLORS` palette stays at the
  bottom for quick selection. Picking a preset just sets the color +
  closes (or keeps open — decide later).
- **Close**: button (or click outside the expanded area).

## Architecture

### New module: `lntrn-ui/src/gpu/color_picker.rs`

```rust
pub struct ColorPicker {
    pub rect: Rect,           // outer bounds where the picker draws
    pub current: Color,        // current hex value as parsed Color
}

pub struct ColorPickerState {
    pub open_id: Option<u32>,        // which zone is currently expanded
    pub hex_input_buf: String,       // text field buffer
    pub dragging: Option<DragKind>,
}

enum DragKind { SvPlane, HueBar, AlphaBar }

impl ColorPicker {
    pub fn draw(
        &self,
        painter: &mut Painter,
        text: &mut TextRenderer,
        ix: &mut InteractionContext,
        state: &mut ColorPickerState,
        fox: &FoxPalette,
        scale: f32, sw: u32, sh: u32,
    ) -> Option<Color> {
        // Returns Some(new_color) if the user changed it this frame.
        // Layout: SV plane left, hue slider middle, hex+readouts right,
        //         preset palette bottom, close button bottom-right.
        // Handles drag in SV plane: cursor.x → saturation, cursor.y → value.
        // Handles drag in hue bar: cursor position → hue.
        // Handles hex field: validate as #RRGGBB, parse, update state.
    }
}
```

### Hosting in System Settings panels

Each color row needs a tiny wrapper:

```rust
struct ColorSlot {
    label: &'static str,
    zone_id: u32,           // the click-to-open zone
    current_hex: String,    // mutable through &mut
}

fn draw_color_slot(...) -> Option<String> { ... }
```

When the user clicks a slot's tile, `state.color_picker.open_id` becomes
that zone's ID. The panel reserves extra vertical space (~220px scaled)
for the expanded picker below the row. Other rows shift down.

### State management

Picker state lives on `PanelState` (the top-level state struct), not per
panel — only one picker can be open at a time across the entire app.

```rust
struct PanelState {
    // ... existing fields ...
    color_picker: ColorPickerState,
}
```

When `color_picker.open_id` is set, the host panel calls
`color_picker.draw(...)` after rendering the row that owns that zone.

### Hex round-tripping

- All config values stay `String` with `"#RRGGBB"` format.
- On open: parse current hex → HSV.
- On drag: update HSV → re-encode hex → update config.
- On hex input: parse → update HSV → don't loop (skip if hex unchanged
  since last drag).

## New / extracted primitives

- **`hsv_to_color(h: f32, s: f32, v: f32, a: f32) -> Color`** — extract
  from `lntrn-media-player/src/render.rs` (`hue_color`) and
  `lntrn-desktop/src/rainbow.rs` (`hsl_to_color`) into shared
  `lntrn_render::color`.
- **`color_to_hsv(c: Color) -> (h, s, v)`** — new helper.
- **`Color::to_hex(&self) -> String`** — already exists? Check
  `lntrn-render/draw/src/color.rs`. If not, add it.
- **`rect_gradient_2d`** (optional, nice-to-have) — bilinear 4-corner
  gradient. Without it, we layer rect_gradient_linear twice for the SV
  plane.
- **Text input widget** — the existing codebase has TextInput in
  `lntrn-ui`. Use it for the hex field.

## Implementation phases

### Phase 1 — Foundations (no UI changes yet)
- [ ] Extract `hsv_to_color` / `color_to_hsv` into `lntrn-render::color`.
- [ ] Add `Color::to_hex` if missing.
- [ ] Write unit tests for HSV ↔ hex round-trip.

### Phase 2 — Picker widget
- [ ] Create `lntrn-ui/src/gpu/color_picker.rs`.
- [ ] Draw SV plane (3 layers: hue base + S overlay + V overlay).
- [ ] Draw hue bar (rainbow gradient).
- [ ] Draw SV indicator dot + hue indicator triangle.
- [ ] Wire drag handlers (mouse + position → HSV update).
- [ ] Draw hex field + RGB/HSV readouts.
- [ ] Draw embedded preset row (re-use existing `draw_color_swatch_row`).
- [ ] Draw close button.
- [ ] Animation: expand/collapse height interpolation (200ms).

### Phase 3 — Integration into panels
- [ ] Add `color_picker: ColorPickerState` to top-level `PanelState`.
- [ ] Refactor `draw_color_swatch_row` to add a "current color" tile +
      click zone that toggles `open_id`.
- [ ] Card height calc: when a slot in this card has the picker open, add
      ~220 * s extra height + push subsequent rows down.
- [ ] Live preview: every drag updates the config field immediately so
      the affected widget (cursor, window border, etc.) updates in real
      time (just like the slider live-update we already have).

### Phase 4 — Per-panel wiring
For each of the 11 color fields, replace the swatch-row call with a
combined row that uses the new color slot. Order:
- [ ] Mouse panel (5 cursor color slots + click_anim_color)
- [ ] Appearance panel (accent + background)
- [ ] Window manager panel (border + focus glow)
- [ ] Windows panel (blur tint)

### Phase 5 — Polish
- [ ] Keyboard accessibility: hex field gets focus on picker open.
- [ ] Animated SV plane indicator dot.
- [ ] Optional alpha slider (lower priority — only blur_tint_color and
      focus_glow_color reasonably benefit from alpha; skip for v1).
- [ ] Theme preset import: when a theme is loaded, the picker UI
      reflects the new color without reopening.

## Open questions to decide before starting

1. **Alpha support?** Only `focus_glow_color` and `blur_tint_color` have
   intensity/opacity sliders alongside them. Should those merge into a
   single picker with alpha, or stay separate? *Suggestion: keep them
   separate — alpha sliders are already exposed as `*_intensity` /
   `blur_tint` floats.*
2. **Where does the picker open — above or below the row?** Below is
   conventional; above might be needed if the row is near the bottom of
   the panel. *Suggestion: always below, scroll-into-view on open.*
3. **One picker open at a time, or can multiple be open?** *Single is
   simpler and reads cleaner.*
4. **Click outside to close?** Yes (and Escape key).
5. **Should clicking a GLOW preset inside the picker also close it?**
   *Probably yes — the preset row IS a quick-pick.*
6. **Show current hex as the slot's static label, or hide?** *Show it
   in small text under the tile — confirms what's selected at a glance.*

## Visual mockup (terminal preview)

```
Mouse → Cursor Theme card after picker opens for "Body Dark":

  Size           [●━━━━━━━━●] 64px
  Outline        [●━━━━━━━━●] 1.00x
  Roundness      [●━━━━━━━━●] 0%
  Body Light     [●]  ● ● ● ● ● ● ● ●        #FFFFFF
  Body Dark      [●]  ● ● ● ● ● ● ● ●        #0A0A0A   ← clicked
   ┌──────────────────────────────────────────────────┐
   │  ┌─────────────┐  ┌─┐    Hex: [#0A0A0A]          │
   │  │             │  │░│                              │
   │  │   SV plane  │  │░│    R: 10  G: 10  B: 10      │
   │  │             │  │█│    H: 0°  S: 0%   V: 4%     │
   │  │         [●] │  │░│                              │
   │  └─────────────┘  └─┘    ●●●●●●●●  [Close]       │
   └──────────────────────────────────────────────────┘
  Accent Light   [●]  ● ● ● ● ● ● ● ●        #FFC800
  Accent Dark    [●]  ● ● ● ● ● ● ● ●        #0A0A0A
  Outline        [●]  ● ● ● ● ● ● ● ●        #0A0A0A
```

## Effort estimate

| Phase | Estimated effort |
|---|---|
| Phase 1 — foundations | ~1-2 hours |
| Phase 2 — picker widget | ~3-4 hours (the bulk of the work) |
| Phase 3 — integration | ~2 hours |
| Phase 4 — per-panel wiring | ~1-2 hours (mostly mechanical) |
| Phase 5 — polish | ~1-2 hours |
| **Total** | **~8-12 hours** |

Most of the complexity lives in Phase 2 (the picker widget). The rest
is mechanical replacement once the widget is solid.

## Out of scope (for now)

- Eye-dropper / "pick from screen" mode.
- Color palette save/load to disk as named themes.
- Per-monitor color profiles.
- HSL vs HSV picker mode switch.
- Color blindness simulation overlay.

These are all good "v2" features once the v1 picker is stable.

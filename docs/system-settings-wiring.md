# System Settings — Wiring TODO

Status of every setting in `lntrn-system-settings`: what's wired to real
behavior, what still only lives in `~/.lantern/config/lantern.toml`, and where
the wiring needs to happen next time we tackle this.

Config shape lives in `lntrn-system-settings/src/config.rs` (mirrors the TOML).

---

## ✅ Already wired

| Setting                              | Consumer                                            | Notes |
| ------------------------------------ | --------------------------------------------------- | ----- |
| `appearance.window_style`            | `lntrn-system-settings/src/wayland.rs` (View menu)  | Fox / Night Sky. Saves and refreshes on next frame. |
| `appearance.theme`                   | `lntrn-theme::active_variant()`                     | Used by most apps via `FoxPalette::current()`. |
| `appearance.wallpaper*`              | `lntrn-compositor/src/wallpaper.rs`                 | Read at startup and on reload. |
| `window_manager.focus_glow`          | `lntrn-compositor/src/render.rs:470`                | Drop-shadow shader switches to colored glow when focused. |
| `window_manager.focus_glow_color`    | `lntrn-compositor/src/state.rs:345`                 | Parsed via `parse_glow_color()`. |
| `window_manager.focus_glow_intensity`| `lntrn-compositor/src/render.rs:471`                | Alpha override for shadow color (0.0–0.6). |
| `window_manager.focus_follows_mouse` | `lntrn-compositor/src/input.rs:634`                 | |
| `windows.window_opacity`             | `lntrn-compositor/src/render.rs:314`                | |
| `windows.blur_intensity`             | `lntrn-compositor/src/render.rs:853`                | |
| `windows.blur_tint`                  | `lntrn-compositor/src/render.rs:854`                | |
| `windows.blur_darken`                | `lntrn-compositor/src/render.rs:855`                | |
| `windows.background_opacity`         | `lntrn-theme::background_opacity()`                 | Used by terminal, file manager, calculator, sysmon, etc. |
| `input.mouse_speed`                  | `lntrn-compositor/src/input.rs:588`                 | `sensitivity = 2^(mouse_speed * 2)`. |
| `input.cursor_theme`                 | `lntrn-compositor/src/cursor.rs`                    | Hot-reloaded in `render.rs:833`. |
| `power.wifi_power_save`              | `config.apply_wifi_power()` → `pkexec iw`           | Immediate + persisted to `/etc/modprobe.d/`. |
| `power.wifi_power_scheme`            | Same as above                                       | "active" / "balanced" / "battery". |

---

## ❌ Not wired — Window Manager

### `window_manager.border_width`  (u32, 0–10)
- **Current:** Not read anywhere.
- **Wire it:** Border rendering needs to be added in
  `lntrn-compositor/src/render.rs` around the window draw path (after the
  drop shadow, before the window content). Probably a `rect_stroke_sdf` call
  with `corner_radius` matching the window and the user's accent color.
- **Decisions to make:** Border color — fixed gray? Accent from glow color?
  New config field?

### `window_manager.titlebar_height`  (u32, 20–60)
- **Current:** Hardcoded at `lntrn-compositor/src/ssd.rs:21` as
  `const BAR_HEIGHT: i32 = 34;`. `SsdManager::bar_height()` returns it.
- **Wire it:** Replace the const with a function reading the config (similar
  to how `mouse_speed` is polled in `state.rs:367`). Cache in state so the
  render loop doesn't hit disk every frame. Every call site already routes
  through `SsdManager::bar_height()` so it's a one-file change.
- **Gotcha:** `ssd.rs:127` `titlebar_rect()` also uses `BAR_HEIGHT` directly —
  grep for all uses.

### `window_manager.gap`  (u32, 0–32)
- **Current:** Hardcoded at `lntrn-compositor/src/tiling.rs:18` as
  `const DEFAULT_GAP: i32 = 30;` (and `DEFAULT_OUTER_GAP` right after).
- **Wire it:** Read from `[window_manager] gap` on compositor init and
  whenever tiling layout is recomputed. The `TilingState` struct (line 54)
  already has a `gap` field — just populate it from config instead of the
  constant. Outer gap could be separate or derived.

### `window_manager.corner_radius`  (u32, 0–20)
- **Current:** Hardcoded at `lntrn-compositor/src/ssd.rs:25` as
  `pub const CORNER_RADIUS: f32 = 18.0;`.
- **Wire it:** Read from config and cache in state. Propagate through to the
  shadow/glow shader uniforms in `render.rs:483` (already parameterized per
  frame — just need to pass the dynamic value instead of the const).
- **Gotcha:** The compositor's corner radius should probably match each app's
  own chrome corner radius (terminal/settings/etc) to avoid visible clipping
  artifacts. Might want a single source of truth.

---

## ❌ Not wired — Mouse / Input

### `input.pointer_acceleration`  (bool, default true = adaptive, false = flat)
- **Current:** Not read anywhere.
- **Wire it:** Needs libinput device config in `lntrn-compositor/src/input.rs`.
  The input backend should iterate libinput devices on connect and set
  `libinput_device_config_accel_set_profile` to `ADAPTIVE` or `FLAT`. Also
  needs a hot-reload path when the config changes.
- **Gotcha:** Has to run when new devices connect too (hotplug).

### `input.scroll_speed`  (f32, 0.25–3.0, default 1.0)
- **Current:** Not read anywhere.
- **Wire it:** Multiply the scroll delta in the wl_pointer `Axis` event
  handler. For the compositor side, that's wherever scroll events are
  forwarded to clients — probably `lntrn-compositor/src/input.rs` scroll
  handling.
- **Gotcha:** May need to also scale `axis_discrete` steps, not just continuous.

### `input.single_click_activate`  (bool, default false = double-click)
- **Current:** Not read anywhere.
- **Wire it:** Fully client-side. `lntrn-file-manager` needs to read this
  via `lntrn_theme::read_config` (or equivalent) and switch its click handler
  between single-click and double-click activation.
- **Gotcha:** Should also affect hover feedback — single-click mode usually
  shows a hand cursor on hover.

### `input.cursor_size`  (u32, 16–64, default 24)
- **Current:** Compositor reads `XCURSOR_SIZE` env var in
  `lntrn-compositor/src/cursor.rs:33`, falls back to internal default.
  Config value is **ignored**.
- **Wire it:** In `cursor.rs`, replace the `XCURSOR_SIZE` read with a call to
  `read_input_setting_u32("cursor_size", 24)`. Also hot-reload in
  `render.rs:833` alongside `cursor_theme` — if size changed, rebuild the
  cursor textures.

---

## ❌ Not wired — Power

### `power.lid_close_action`  (String: suspend / hibernate / lock / nothing)
- **Current:** Read in `lntrn-compositor/src/input.rs:1125` but the calling
  code comment says *"for now use lid_close_action"* — meaning it's used
  **regardless** of AC state. No actual lid-switch listener wired yet?
- **Wire it:** Need a logind listener (dbus) for the lid switch event, or
  poll `/proc/acpi/button/lid/LID0/state`. On close, dispatch to the
  configured action via `loginctl suspend` / `loginctl hibernate` / our own
  lock screen / nothing.

### `power.lid_close_on_ac`  (same type)
- **Current:** Not read. Comment in `input.rs:1124` says *"if we had AC
  detection"*.
- **Wire it:** Detect AC state via `/sys/class/power_supply/AC*/online`
  or via upower. Choose between `lid_close_action` and `lid_close_on_ac`
  based on current AC state when the lid event fires.

### `power.dim_after`  (u32 seconds, 0 = never)
- **Current:** Not read.
- **Wire it:** Need an idle timer in the compositor that fires after N
  seconds of no input. On fire, reduce the output brightness (backlight
  control) or apply a dim overlay. Reset timer on any input.
- **Possible path:** Add an `IdleState` to `lntrn-compositor::state` with a
  last-input timestamp. Compositor already has `idle_inhibit_manager_state`
  in Smithay — leverage it.

### `power.idle_timeout`  (u32 seconds, 60–1800)
- **Current:** Not read.
- **Wire it:** Second threshold on the same idle timer. After this, trigger
  `idle_action`.

### `power.idle_action`  (String: suspend / lock / nothing)
- **Current:** Not read.
- **Wire it:** Dispatch on idle_timeout fire. Same mechanism as lid action.

### `power.low_battery_threshold`  (u32 %, 5–30)
- **Current:** Not read.
- **Wire it:** Needs battery polling. Options:
  1. Poll `/sys/class/power_supply/BAT*/capacity` every ~30s from a
     background thread in the compositor or `lntrn-bar`.
  2. Subscribe to upower dbus signals.
- **Action on threshold:** Notification via `lntrn-notifyd` / OSD.

### `power.critical_battery_threshold`  (u32 %, 2–15)
- **Current:** Not read.
- **Wire it:** Same battery polling as above; trigger `critical_battery_action`.

### `power.critical_battery_action`  (String: suspend / hibernate / shutdown / nothing)
- **Current:** Not read.
- **Wire it:** Dispatch when battery crosses critical threshold.

---

## Wiring order suggestion

When we pick this up, I'd tackle in this order:

1. **WM layout constants** (titlebar_height, corner_radius, gap) — purely
   local to the compositor, unblocks visual tweaking.
2. **cursor_size** — small, self-contained change in `cursor.rs`.
3. **pointer_acceleration** + **scroll_speed** — libinput config path.
4. **single_click_activate** — client-side only, in `lntrn-file-manager`.
5. **Idle / dim** — new `IdleState` machinery in compositor.
6. **Battery polling** — probably lives in `lntrn-bar` or a new
   `lntrn-powerd` daemon; emits dbus signals for the rest of the stack.
7. **Lid switch listener** — dbus logind hookup. Needs AC detection first.
8. **border_width** — last because it's aesthetic and needs a color decision.

---

_Last updated: 2026-04-10_

# lntrn-command-center — Roadmap

The single drop-down panel that owns the Super-tap. Search-everything launcher
on top, control tiles in the middle, pinned apps on the bottom. One
keystroke, one panel, every important toggle on the system.

> Decisions baked in (from design Q&A on 2026-04-29):
> - **Layout:** stacked — search → controls row → pinned apps grid
> - **Super-tap behavior:** replaces the existing "cycle desktop panel" hook
> - **Launcher scope:** full-Spotlight (apps, files, web, math, commands, clipboard)
> - **Tile expansion:** inline grow (panel resizes, one tile expanded at a time)
> - **App grid:** pinned favorites + search-driven results
> - **Animation:** scale 0.95→1.0 + fade, ~180ms ease-out cubic
> - **Dismiss:** Super tap again / Escape / click outside / auto-close after launch action
> - **Visual:** glassy/acrylic — backdrop blur, warm dark surface, Studio palette
>   (text `#e8dcc8`, accent `#C8860A`)
> - **Phasing:** skeleton → launcher → controls (each phase shippable)

---

## 1. Architecture

```
lntrn-command-center/
├── Cargo.toml
└── src/
    ├── main.rs              # entry; --toggle / --show / --hide flags;
    │                        # send-or-become-daemon socket pattern
    ├── app.rs               # AppState: visible, focused section, anim t,
    │                        # expanded tile id
    ├── layershell.rs        # wlr-layer-shell setup. Forked from
    │                        # lntrn-osd/src/layershell.rs (smallest precedent)
    ├── render.rs            # Painter root, panel chrome, layout dispatch
    ├── animation.rs         # spring + ease-out cubic for open/close
    ├── ipc.rs               # /run/user/{uid}/lntrn-command-center.sock
    ├── search/
    │   ├── mod.rs           # provider dispatcher + ranking
    │   ├── apps.rs          # .desktop scanner + fuzzy match (Lantern-owned, no shared crate)
    │   ├── files.rs         # xdg-recent + ~/Documents|Pictures|Downloads index
    │   ├── web.rs           # DDG / URL fallback
    │   ├── math.rs          # =expr evaluator (small custom)
    │   ├── commands.rs      # :lock :wifi-off :reboot etc.
    │   └── clipboard.rs     # last-N from lntrn-clipboard
    ├── launcher/
    │   ├── mod.rs           # pinned grid + result grid
    │   ├── pins.rs          # ~/.lantern/config/command-center/pins.toml
    │   └── icons.rs         # icon resolver (own implementation, mirrors bar's)
    └── controls/
        ├── mod.rs           # tile row + expansion state machine (one open at a time)
        ├── wifi.rs          # full implementation (own copy of bar's logic)
        ├── bluetooth.rs
        ├── audio.rs
        ├── brightness.rs    # /sys/class/backlight + ddcutil for external
        ├── battery.rs
        ├── clock.rs         # chrono + month-grid widget
        ├── mpris.rs
        └── power.rs         # logind: lock/sleep/logout/reboot/shutdown
```

**Self-contained crate:** Command Center owns its own copy of every
control implementation. The bar stays zero-touch — no extraction, no
shared `lntrn-controls` or `lntrn-apps` crate. We accept the tradeoffs:
two pollers running in parallel (nmcli, wpctl, upower fire from both bar
and Command Center), and any future fix lands in two places. The bar's
code is sacred and we don't refactor across crate boundaries.

---

## 2. Visual + layout spec

```
                   ┌──────────────────────────────────────────┐
                   │  🔍  Search apps, files, web, =math…     │
                   ├──────────────────────────────────────────┤
                   │  📶   🔵   🔊   ☀   🔋   📅   ▶          │  ← controls row
                   │      (one tile inline-expands here)      │  (one open at a time)
                   ├──────────────────────────────────────────┤
                   │  ⭐ Pinned                                │
                   │  [Term] [Files] [Code] [Browser] [Music]  │
                   │                                          │
                   │  📋 Recent / Search results              │
                   │  [App] [App] [App] [App]                 │
                   │  [App] [App] [App] [App]                 │
                   └──────────────────────────────────────────┘
```

- Panel width: **720 logical px**, centered horizontally
- Top margin: **12 logical px** below the screen edge
- Surface: warm dark (~`#1a1612`) at 78 % opacity + backdrop blur (hooks
  into the planned acrylic pipeline; falls back to plain dark if blur
  is unavailable)
- Border: 1 px hairline `rgba(255,255,255,0.08)`, `corner_radius = 14`,
  subtle inner top highlight
- Text: `#e8dcc8` (Studio tan); accents at `#C8860A`
- Open animation: scale `0.95 → 1.0` + fade `0 → 1`, ~180 ms, ease-out cubic.
  **Animated entirely in shader — the wl_surface size never changes.**

**Critical layer-shell decisions** (verified against Lantern precedents):

- Mount one **fullscreen** layer surface (anchored top|left|right|bottom),
  transparent everywhere except the panel rect. The panel is a draw region,
  not a sized surface. This matches `lntrn-menu/src/layershell.rs` exactly.
- `keyboard_interactivity = Exclusive` (not OnDemand — OnDemand is
  implementation-defined; Exclusive guarantees we get the keystrokes the
  instant we open).
- `exclusive_zone = -1` (ignore other exclusive zones; we float over the bar
  if it ever conflicts — same as `lntrn-osd`).
- `set_input_region(None)` so we can receive clicks anywhere; we hit-test
  the panel rect in code. Click outside the rect → close.
- The compositor passes the focused output name (and cursor x/y if useful)
  on argv when it spawns us. We bind only that `wl_output` and pass it to
  `get_layer_surface`. **No client-side "guess the focused monitor" logic.**

---

## 3. Refactors that have to happen first

These are not nice-to-haves — they prevent dual source-of-truth pain.

**Command Center is fully self-contained.** No shared crates, no
extractions, no refactors of other Lantern apps. Every control is
implemented directly inside `lntrn-command-center/src/controls/*`,
mirroring the structure already proven in `lntrn-bar`.

### 3a. Mirror, don't extract

Each control file in `lntrn-command-center/src/controls/` will be a
**parallel implementation** of what `lntrn-bar` already does:

| Bar reference (read-only)             | Command Center file              |
| ------------------------------------- | -------------------------------- |
| `lntrn-bar/src/wifi.rs`               | `controls/wifi.rs`               |
| `lntrn-bar/src/bluetooth*.rs`         | `controls/bluetooth.rs`          |
| `lntrn-bar/src/audio/`                | `controls/audio.rs`              |
| `lntrn-bar/src/battery.rs`            | `controls/battery.rs`            |
| `lntrn-bar/src/mpris.rs`              | `controls/mpris.rs`              |
| `lntrn-bar/src/clock.rs`              | `controls/clock.rs`              |
| `lntrn-bar/src/appmenu/power_modal.rs`| `controls/power.rs`              |

We **read** the bar's files for reference and write our own equivalents.
Cmd/Event channel patterns, nmcli/wpctl/sysfs/D-Bus calls — all copied
in spirit, rewritten in our crate. No imports across crate boundaries,
no shared code, no shared state.

### 3b. Tradeoffs we accept

- **Double polling.** When both the bar and Command Center are running,
  `nmcli`, `wpctl`, `upower`, etc. each fire from both processes. On a
  laptop this is measurable but not catastrophic — typically <1 % CPU
  overhead.
- **Two implementations to maintain.** A nmcli output-format change or
  wpctl flag rename would need fixing in both places. We accept this
  rather than coupling crates.
- **No structural cleanup of the bar.** The bar's `wifi.rs` (734 lines)
  and the compositor's `input.rs` (1165 lines) stay at their current
  sizes. We don't refactor what we're not building.

### 3c. `lntrn-apps` ideas we keep, in our own crate

`.desktop` scanning, icon resolution, and fuzzy ranking still need to
exist for the launcher — they live as **internal modules of
`lntrn-command-center`**, not a shared crate:

- `launcher/icons.rs` — own implementation of theme search (Tela, hicolor,
  Adwaita, breeze, pixmaps). Mirrors `lntrn-bar/src/apptray.rs:733-814`
  in spirit.
- `search/apps.rs` — own `.desktop` scanner + parser, plus a small
  ~150-line subsequence-based fuzzy ranker.

### 3d. `lntrn-menu` is unused, leave in place

`lntrn-menu` ships an SNI watcher and a context-menu helper, but
`lntrn-bar` already runs its own SNI watcher (same `org.kde.StatusNotifierWatcher`
name) — `lntrn-menu/src/sni.rs` and `tray.rs` look like pre-bar drafts
that got left behind. Confirmed nothing else in the workspace spawns it.

**Action: leave it alone.** Nothing in this project is ever deleted.
The crate stays in the workspace as-is. Command Center never owns SNI;
tray stays in the bar.

---

## 4. Compositor changes

`lntrn-compositor/src/input.rs:241-256` currently has a Super-tap branch
that calls `cycle_desktop_panel` (state.rs:690-698) — which writes a
plain string to `~/.lantern/config/desktop-panel`.

Repoint the Super-tap branch to spawn Command Center instead. Leave
`cycle_desktop_panel` and the helper file path intact (nothing gets
deleted in this project) — just stop calling it from the Super-tap
handler. If we ever want it back, it's a one-line revert.

```rust
// Super tap detected (no combo, clean release)
spawn_detached("lntrn-command-center", &data.socket_name);
```

Matching the existing spawn pattern at `input.rs:565-577` (which is how
Super+Return spawns `lntrn-terminal`). `spawn_detached` already resolves
through `~/.lantern/bin/`, sets `WAYLAND_DISPLAY`, and calls
`setsid()`+`setpgid()`.

If we want to pass the focused output name on argv (recommended), add it
as the first positional argument here.

---

## 5. IPC

- Path: **`/run/user/{uid}/lntrn-command-center.sock`** (matches
  `workspace_ipc.rs:19` and `hover_preview.rs:4`).
- Protocol: Unix datagram. Send-or-become-daemon (port the trick from
  `lntrn-osd/src/main.rs:13-26`):
  1. `lntrn-command-center` (no args, or `--toggle`) tries `send_to(path, b"toggle")`.
  2. If send succeeds, exit immediately. The running daemon flips state.
  3. If send fails (no listener), `bind` the socket and start the daemon
     ourselves — show panel on first tick.
- Other commands: `--show`, `--hide`, `--focus-search`, `--reload-pins`.
- Compositor only ever shells out `lntrn-command-center --toggle`.

---

## 6. Keyboard / interaction

| Input                         | Effect                                                             |
| ----------------------------- | ------------------------------------------------------------------ |
| Super (tap, no combo)         | Toggle Command Center                                              |
| Type any character            | Focus moves to search; results swap into grid                      |
| Esc                           | Close (animated)                                                   |
| Click outside panel rect      | Close                                                              |
| ↑ ↓ ← →                       | Navigate result grid / pinned row / control tiles                  |
| Tab / Shift-Tab               | Cycle focus zones: search ↔ controls ↔ pinned ↔ results            |
| Enter                         | Activate selection; auto-close after launch                        |
| Right-click on app            | Pin / unpin                                                        |
| Click control tile            | Inline-expand that tile (collapses any previously expanded tile)   |

---

## 7. Phased delivery

### Phase 0 — Pre-work (no other-crate touching)

0.1. Verify `lntrn-menu` is orphaned; document its status. The crate stays
     in the workspace either way — nothing in this project is ever deleted.
0.2. Confirm no shared crates will be created. Command Center is fully
     self-contained.

**Ship gate:** zero changes to anything outside `lntrn-command-center/`.

### Phase 1 — Skeleton

1.1. Create `lntrn-command-center` crate, register in workspace `Cargo.toml`.
1.2. Layer-shell: fullscreen surface, `Anchor::TOP|LEFT|RIGHT|BOTTOM`,
     `Exclusive` keyboard, `exclusive_zone = -1`, `set_input_region(None)`.
1.3. wgpu Painter setup (mirror `lntrn-osd`).
1.4. Panel chrome (rounded rect + blur backdrop + hairline border) drawn
     centered in the fullscreen surface.
1.5. Open/close animation (scale + fade, in shader; surface stays stable).
1.6. IPC socket + send-or-become-daemon pattern; `--toggle` flag.
1.7. Compositor: redirect Super-tap from `cycle_desktop_panel` to
     `spawn_detached("lntrn-command-center", ...)`.
1.8. Click-outside hit test → close. Esc → close.

**Ship gate:** Super opens an empty pretty panel; Super / Esc / outside-click close it.

### Phase 2 — Launcher

2.1. Search input field + cursor + IME basics.
2.2. App provider: scan `.desktop` files (own implementation in
     `search/apps.rs`), fuzzy match name+exec+keywords.
2.3. Pinned favorites grid (load `pins.toml`, right-click pin/unpin).
2.4. (skipped — no recents) — pins are the only "frequent apps" surface.
2.5. Result grid (icons via own resolver in `launcher/icons.rs`).
2.6. Enter → launch via `spawn_detached`, then auto-close.
2.7. Spotlight providers, in order of cost-to-build:
     - 2.7a. Math (`=2+2`, simple shunting yard) — cheap, instant.
     - 2.7b. Commands (`:lock`, `:reboot`, `:wifi-off`, `:bt-on`, `:dnd`, etc.)
     - 2.7c. Files (indexer that walks ~/Documents,
              ~/Pictures, ~/Downloads on first run, caches paths).
     - 2.7d. Web fallback (`?how to fix x` → DDG URL).
     - 2.7e. Clipboard history (read `lntrn-clipboard` socket).

**Ship gate:** Daily-use launcher with apps, files, web, math, commands,
clipboard. (`lntrn-menu` stays in the workspace untouched as always.)

### Phase 3 — Controls (one tile at a time, ship after each)

Each tile = small icon in the tile row + inline-grown detail panel when
clicked. Only one tile expanded at a time.

3.1. Date / Time / Mini calendar — zero deps, instant gratification.
3.2. Battery + power profile (upower + power-profiles-daemon over D-Bus).
3.3. Audio (output volume, output device picker, input mute, per-app mixer).
3.4. Brightness (sysfs internal, ddcutil external).
3.5. WiFi (toggle → networks list → connect dialog).
3.6. Bluetooth (toggle → devices list → connect/disconnect/send file).
3.7. MPRIS Now Playing (track info, transport, source picker).
3.8. Power menu (lock/sleep/logout/reboot/shutdown via logind).

**Ship gate per tile:** open Command Center, click tile, it works the
same as the bar version.

### Phase 4 — Bonus (no commitment)

- Do Not Disturb + notification feed
- Mini sysmon (CPU/RAM/net snapshot from `lntrn-sysmon`)
- Per-monitor "open on monitor with cursor"
- Keyboard layout switcher
- Optional hot-corner trigger

---

## 8. File-size discipline

Targets within the 700-line cap (CLAUDE.md):

- `render.rs` ≤ 500 (panel chrome + layout dispatch only;
  sections own their draw)
- `controls/wifi.rs` ≤ 600 (full implementation: state, worker, draw)
  — split into `controls/wifi/` if it grows past 600
- Other `controls/*.rs` ≤ 600 each (split into a folder once they cross)
- `search/mod.rs` ≤ 300 (provider dispatcher)
- `search/*.rs` (per provider) ≤ 400 each
- `launcher/mod.rs` ≤ 500
- `layershell.rs` ≤ 400 (mostly forked from lntrn-osd)
- Any file approaching 600 → split before merging

---

## 9. External dependency budget

Allowed (already in workspace or pre-approved):

- `wayland-client`, `wayland-backend`, `wayland-protocols`,
  `wayland-protocols-wlr` — layer-shell, matching the bar's stack
- `wgpu` — rendering (matches all other Lantern apps)
- `lntrn-dbus` — Lantern's in-house D-Bus client (used for MPRIS,
  BlueZ, upower, logind, power-profiles-daemon — *no zbus*)
- `lntrn-render`, `lntrn-ui`, `lntrn-icons`, `lntrn-theme` — Lantern
  internal crates we already share across apps
- `serde`, `serde_json`, `toml` — config (memory-approved exception)
- `raw-window-handle` — wgpu surface wrapping (matches the bar)
- `tracing`, `tracing-subscriber` — logging (matches the bar)
- `png`, `resvg`, `flate2` — icon decoding (matches the bar)

Shell-out (no crate, just `Command::new`):

- `nmcli` for WiFi
- `bluetoothctl` for Bluetooth
- `wpctl` / `pactl` for audio
- `ddcutil` for external display brightness
- `loginctl` for power actions
- Direct sysfs reads for battery + internal backlight

Not allowed without explicit user signoff:

- Any fuzzy-match crate (write our own subsequence ranker, ~150 lines).
- Any search/index crate (cached walked dirs is enough).
- Any expression-evaluator crate (write a small shunting-yard evaluator).
- `zbus` — Lantern uses `lntrn-dbus` for D-Bus, never zbus.

---

## 10. Open questions / things to settle in flight

1. **Auto-close timing on launch:** close the panel before or after
   `spawn_detached` returns? Instant feels best — confirm by feel during
   Phase 2.
2. **(slot freed by skipping recents)** Pinned-only is the design.
3. **Multi-monitor:** open on the output that owns the cursor at toggle time.
   Compositor passes it on argv. Confirm during Phase 1.7.
4. **Blur fallback:** if the acrylic pipeline isn't ready when we ship,
   degrade to a flat warm-dark surface at 92 % opacity.
5. **`lntrn-menu`:** stays in the workspace permanently. Command Center
   replaces its role in practice (nothing spawns lntrn-menu today), but
   the crate itself is not removed. **Nothing is ever deleted in this project.**

---

## 11. Things that could derail us

- **Layer-shell focus quirks across compositors.** We only target our own
  compositor, but worth testing under Niri/Sway during dev for sanity.
- **Per-app audio mixer is involved** (PipeWire node tree). May warrant
  its own follow-up phase after 3.3 ships the basics.
- **Click-outside dismiss interacting with drag gestures** (e.g. a slider
  drag that ends outside the panel rect). Treat drags as captured even
  if the release lands outside the rect.
- **Re-implementing controls in parallel with the bar.** Two copies
  always risk drift. Mitigation: when fixing a bug in either, search
  the other implementation for the same pattern and fix it there too.
- **Compositor `input.rs` is 1165 lines.** We touch one branch of it
  in Phase 1 and leave the rest alone. If our edit pushes the file over
  what feels manageable, surface it and ask before splitting.

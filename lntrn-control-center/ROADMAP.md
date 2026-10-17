# lntrn-control-center — Roadmap

The bar along the bottom of the screen, Plasma-style. The Command Center's
sibling: both run side by side until this one stands on its own, then a
System Settings toggle picks which one the session uses.

> Decisions baked in (design Q&A, 2026-10-16):
> - **Stack:** Lantern UI 2 (+ `lntrn-kit` for the look). LUI2's own Wayland
>   client and `LayerConfig::bar`, no wayland-client crates.
> - **Monitor:** primary only. Read from `[[monitors]] primary = true` in
>   `lantern.toml` and named to the compositor (U086). A primary that isn't
>   plugged in (sysfs `status`) falls back to the compositor's pick.
> - **Style:** floating (Plasma 6): rounded corners, a hairline edge, solid
>   plate. **Shape is user-set (2026-10-16)** in System Settings → Desktop →
>   Control Center, stored as `[control_center]` `height` (64, 40–120),
>   `corner_radius` (16, 0–32), `gap_below` (8, 0–40), `gap_sides` (16,
>   0–120); the bar follows it live. (It used to follow the window gap.)
> - **Keyboard:** none. Clicking the bar never steals focus. Switch to
>   on-demand (`ShellRequest::SetLayer`) once something in it takes typing.
> - **Docking (2026-10-16):** docks full-width, flush and square (Windows
>   style) while a maximized **or fullscreen** window shows on its monitor's
>   visible workspace. The compositor says so on its workspace socket
>   (`covered:<output>:<0|1>`, `window_management/covered.rs`), since
>   foreign-toplevel has no monitor or workspace. Morph: the surface is
>   always the bottom `64 + gap` px at full width, the plate glides inside
>   it, the exclusive zone flips once per glide (64 + gap floating, 64
>   docked), and an input region (LUI2 U092) lets clicks off the plate
>   through to the window underneath.
> - **Glide timing:** docking matches the compositor's maximize glide
>   (`[animations]`: 1 s / speed, preset curve, off when `enabled`/`state`
>   are false) so plate and window land together; floating again is a quick
>   0.2 s ease-out; Reduce Motion makes both instant.
> - **Compositor side (2026-10-16):** a zone change mid-maximize retargets
>   the window's animation instead of cutting it (`reflow.rs` `refit`), and
>   maximized windows draw no border ring (it showed as a yellow stripe
>   above the docked plate). The docked plate's top uses the surface's own
>   pixels-per-logical, floored, so fractional scales never leave a sliver.
> - **Auto hide (2026-10-16):** `[control_center] visibility = "always" |
>   "auto_hide"`, `auto_hide_delay_ms` (500, 0–3000), `auto_hide_slide_ms`
>   (250, 0–1000), all on the Settings page. For the OLED: hidden, the
>   surface is a see-through 2px strip at the bottom edge (zone 0); the
>   pointer on it grows the surface and slides the plate up. **The bar never
>   sits over a window** (Alva): every frame of the slide the exclusive zone
>   is exactly as much of the bar as shows, and the compositor refits
>   maximized windows on each zone commit, so the window's edge rides the
>   plate up and back down, floating or docked. No compositor change was
>   needed. The input region keeps the strip while shown, so a pointer
>   resting on the edge never makes it flicker. Stays up while the launcher
>   is open. Starts shown, hides after the delay.
> - **Session (2026-10-16):** in `lntrn-session-manager`'s `DAEMONS`:
>   started at login, restarted after a crash. The compositor taking the
>   surface away (monitor gone) exits 75, so the session starts it again;
>   its own restarts (new primary, Quick Kill) exec in place, same pid.

> - **Dock (2026-10-16, Alva's answers):** middle of the bar; shows
>   **every** window (all monitors, all workspaces; activating one switches
>   to its workspace); **one icon per app** (a line under it while running,
>   longer + accent when focused, a dot per extra window); click = start /
>   focus / **minimize when already focused** / card when several windows;
>   **pins stay in place** (drag to reorder), unpinned running apps follow
>   in open order, no divider; **hover 0.4 s** shows a card above the icon
>   (app name, per-window title + X + live thumbnail; click focuses);
>   right-click: New Window (also middle-click), Pin/Unpin, Favorites,
>   Close window(s); the launcher's menu can pin too. Pins **start empty**,
>   `~/.lantern/config/control-center/dock.toml`.
> - **Dock plumbing:** the compositor's workspace socket carries a window
>   feed (`win:<id>\t<flags>\t<app_id>\t<title>`, `win-gone:<id>`, ids
>   from its foreign-toplevel list) and takes `activate:`/`minimize:`/
>   `close:<id>`, `icons:<ns>\t…` (minimize flies into the app's icon) and
>   `thumbs:<ns>\t…` (live thumbnails over a layer surface, drawn by the
>   Command Center's thumbnail path): `lntrn-compositor/src/
>   window_management/dock_link.rs`. Windows find their app by desktop id,
>   StartupWMClass (`lntrn-apps` `wm_class`), reverse-domain tail, icon,
>   program. The card and menu live in the **overlay** (`app/overlay.rs`),
>   one full-monitor surface shared with the launcher: a card takes the
>   pointer over itself alone (input region from its first frame), a menu
>   or the launcher take every click and the keyboard.

## Done

- [x] Layer surface on the primary monitor, exclusive zone, floating plate
- [x] Follows `lantern.toml` live via inotify (no idle frames): accent,
      background colour / style, gap; a new primary restarts the bar there
- [x] Single instance (`flock` in `$XDG_RUNTIME_DIR`)
- [x] Float normally, dock under a maximized/fullscreen window, with a glide
- [x] System Settings page (Desktop group): height, radius, gaps, live preview
- [x] Clock (right): time over date (one line on a short bar), libc local
      time, a real-time-clock thread turning the minute (right after a
      suspend); `clock_24h`, `clock_seconds`, `clock_date`,
      `clock_date_style` in `[control_center]` and on the Settings page
- [x] App launcher (far left): nine-dot button; Kickoff-style menu (search,
      category sidebar, app list, power row with confirm) on a full-monitor
      Top-layer surface with Exclusive keyboard (type to search; click off
      the panel / Esc / launching closes). Apps + icons from the new shared
      `lntrn-apps` crate (desktop entries with OnlyShowIn/NotShowIn/TryExec,
      apps in every main category they list, icons rendered off-thread)
- [x] Auto hide (see above), with the Settings page's Visibility card
- [x] Launcher button fills the bar's height, bigger dots, faint face at rest
- [x] Favorites above All apps (the menu opens on them), kept in
      `~/.lantern/config/control-center/favorites.toml` (one id a line, like
      the Command Center's pins); right-click an app for its menu (LUI2
      `ContextMenu`): Open, Add to / Remove from Favorites
- [x] Quick Kill at the right of the power row: execs the newest build
- [x] `clock_ampm` (Show AM/PM) in `[control_center]` and on the page
- [x] Dock: pins + every open window, cards with live thumbnails, menus,
      drag to reorder, minimize into the icon (see above)
- [x] Search field keeps the keyboard after a group is clicked, and has a
      real ring (the bar's theme has `border_width` 0; LUI2 now draws no
      zero-width strokes at all)

## Next (ideas, unordered)

Planned order (Alva, 2026-10-16): clock → app launcher (far left) → pins +
open windows dock (middle).

- [ ] System tray / quick toggles (audio, network, bluetooth, battery)

## Known gaps

- **Monitor gone for long:** the session gives up on a daemon that dies 5
  times in a minute, so with no monitor at all for a while the bar stays
  down until the next login. LUI2 reporting outputs (so the bar could wait
  for one alive) would fix it.
- **Auto hide and a monitor below:** the strip sits on the bottom edge, so
  a monitor arranged under the bar's would let the pointer pass through.
- **Blur:** the compositor doesn't blur behind layer surfaces, so the plate
  stays solid rather than following `windows.background_opacity`.

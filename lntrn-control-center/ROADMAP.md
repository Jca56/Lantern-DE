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
>   surface away (monitor gone) waits for a monitor, then execs in place
>   like its own restarts (new primary, Quick Kill): same pid.

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

> - **System tray (2026-10-17, Alva's answers):** before the clock, left
>   to right battery (icon + percent), WiFi, Bluetooth, sound, brightness,
>   workspace number, Game Mode; each shows only where the machine has it,
>   and System Settings' System Tray card turns each off (`[control_center]`
>   `tray_*`). Icons are our own SVGs (`tray/glyphs.rs`) rendered by
>   lntrn-svg at their exact pixel size. **Own popup per icon** in the
>   overlay (battery details + charge limit, WiFi switch + networks, BT
>   switch + paired devices, volume + outputs, brightness); the overlay
>   takes every click but the bar's plate, so the bar's other items and
>   wheel work while one is up. Wheel: volume and brightness in the keys'
>   5% steps (with the OSD), workspaces; middle click mutes. **Core now,
>   setup next pass:** joining a network iwd doesn't know the password of
>   and pairing new BT devices stay in the Command Center (they need
>   agents, and CC holds the desktop's BlueZ agents while both run).
> - **lntrn-status (2026-10-17):** the shared crate the tray reads (in the
>   workspace, like lntrn-apps), a thread per subsystem, waking the loop
>   only on a change: battery from sysfs on kernel uevents, backlight from
>   sysfs (logind's SetBrightness as a fallback), audio by `wpctl` with
>   `pactl subscribe` saying when (no polling), WiFi from iwd and Bluetooth
>   from BlueZ on our own lntrn-dbus, following their signals, never
>   auto-starting iwd. lntrn-dbus gained 16-bit/double decoding,
>   `FLAG_NO_AUTO_START`, and guards against messages cut short.

> - **One panel at a time (2026-10-17, Alva's answers):** System Settings →
>   Control Center → Panel picks the Control Center or the Command Center
>   (`[session] panel`, default `control_center`). The session manager
>   supervises whichever is chosen (the Command Center as `--hide`) and
>   swaps them live when the file changes (`lntrn-session-manager/src/
>   panel.rs`). A Super tap, the top-center hot edge and the desktop hub's
>   Command Center all go through the compositor's `summon_panel`: the
>   Command Center toggles, the Control Center gets `launcher:toggle` on
>   the workspace socket and opens or closes its launcher (the hub sends
>   `summon` on that socket).

> - **WiFi passwords (2026-10-17, Alva's answers):** a click on a network
>   that needs a password opens a box under its row: the field (dots, an
>   eye to show it), Join, a line saying what's wrong; Enter joins,
>   Escape closes the box before the popup. A password that didn't work
>   keeps the box open, focused, to fix; one that worked, or the box
>   closing, wipes what was typed. A **Join a hidden network…** row under
>   the list takes a name and an optional password. Saved networks have
>   **Forget** (the connected one always, others on hover; a second click
>   within 4 s confirms). `lntrn-status` does the iwd side: the join's
>   own connection registers a short-lived `net.connman.iwd.Agent`
>   (`wifi/join.rs`), answers `RequestPassphrase` only to iwd's unique
>   name and only for the network being joined, then unregisters; errors
>   are worded by iwd's error name (`lntrn-dbus` `Message::error_name`).
>   802.1X company logins stay out (they need a provisioning file).
> - **Bluetooth pairing and files (2026-10-17, Alva's answers):** no more
>   `bluetoothctl`: `lntrn-status`'s Bluetooth thread is the desktop's
>   default BlueZ agent (`KeyboardDisplay`, answering BlueZ's unique name
>   only), so pairing from here and from a phone both come to the popup.
>   **Add a device…** under the paired list scans and makes the PC seen
>   (Discoverable, BlueZ's own timeout if the bar dies) while open; a
>   click on a nearby device pairs, trusts and connects it. What pairing
>   asks shows inline under the device's row (a code to check, a PIN or
>   six digits to type, a code to type on a keyboard), or in a card up top
>   for a device in neither list; such an ask **opens the popup by
>   itself**, once. Paired devices get **Forget** (second click confirms)
>   and, when they take files, **Send a file** on hover (picked with
>   `lntrn-file-manager --pick`, the popup stepping aside). Files coming
>   in ask Accept/Decline inline, land in obexd's cache under a cleaned
>   name and move to `~/Downloads`; progress shows under the device, and
>   a notification says how sends and receives went. obexd takes one
>   agent at a time: the bar's retries every 15 s while another holds it.
> - **App icons in the tray (2026-10-17, Alva's answers):** the bar is the
>   desktop's StatusNotifierWatcher + host (`lntrn-status`'s `apps`, on
>   lntrn-dbus, a port of the Command Center's with calls answered by
>   serial and items followed by their owner's unique name). **App icons**
>   is a row in System Settings' tray order (`apps` in `tray_order`,
>   `tray_apps`), first by default; an order saved before it gets it
>   first. Passive items are hidden; all others show inline. Left click
>   Activate (an item that's only a menu: its menu), middle click
>   SecondaryActivate, wheel Scroll, right click its dbusmenu as the bar's
>   own context menu in the overlay (submenus, ✓ for ticked rows, greyed
>   rows as headers). Pictures: pixels the app sends, or a name looked up
>   in its `IconThemePath`, the hicolor status/panel folders, then
>   `lntrn-apps`; its first letter until one's there; an accent dot while
>   it needs attention. Another tray holding the name: retried every 10 s.
>   XEmbed (old X11 tray icons) isn't supported.
> - **Tray order (2026-10-17, Alva's answers):** System Settings' System
>   Tray card drags its rows by a six-dot grip at their left (or Up/Down,
>   Home/End on a focused grip) into `[control_center] tray_order`, a list
>   of `battery wifi bluetooth audio brightness workspace game_mode`; top
>   to bottom there is left to right on the bar, live. A stray word is
>   passed over and a missing item follows in its own order (both sides).
>   Battery percent rides along nested under Battery. The list is
>   `lntrn-kit`'s `Card::reorder` (and `Card::nested`), for anything else
>   that wants one.

> - **Mini monitors (2026-10-17, Alva's answers):** after the launcher's
>   button, CPU (gold), GPU (green), RAM (blue): each a face with its load
>   graphed behind its numbers (load %, temperature in green/amber/red;
>   load, temperature and `2.0/12GB` of video memory; load and
>   `2.8/32GB`), as wide as its widest numbers so the dock never shifts,
>   one line on a short bar. Totals read as sold (31.0 GiB shows 32). A
>   click opens a **popup** in the overlay (not the System Monitor): a big
>   graph (hover says that moment's load and how long ago), the readings
>   big; CPU: model, clock + governor, load averages + uptime, busiest
>   programs; GPU: name + driver, VRAM and power meters, fans, clocks;
>   RAM: used/cache/free bar with legend, swap, programs holding the most.
>   Each ends in **Open System Monitor**: brings an open one forward, else
>   starts `lntrn-sysmon --page cpu|gpu|memory` (sysmon gained `--page`).
>   System Settings' Mini Monitors card: a switch each, drag order,
>   update every 0.5/1/2 s, graphs over 30 s/1 min/2 min
>   (`[control_center] monitor_cpu/gpu/ram`, `monitor_order`,
>   `monitor_interval_ms`, `monitor_span_s`).
> - **Sampling (2026-10-17):** sysmon's CPU, memory, GPU (NVML) and
>   process samplers moved into `lntrn-status`'s `load` module, and sysmon
>   uses them from there: one copy. `Load` is the bar's thread (one look a
>   beat, waking the loop); the processes are walked only while the CPU or
>   RAM popup is up. CPU temperature from the processor's hwmon chip
>   (Tdie/Tctl/Package id 0). A graphics card that is runtime-suspended is
>   never asked anything (it would wake); the bar also lets an idle card
>   that may sleep (`power/control` auto, not `boot_vga`) rest, looking
>   only every 30 s and letting NVML go between, so a laptop's dGPU can
>   fall asleep. The GPU monitor shows only where a card says how busy it
>   is (not on an Intel-only machine).

> - **Calendar (2026-10-17, Alva's answers):** the clock is its face: lit
>   under the pointer, a click opens a **popup** in the overlay (the time
>   big, the whole date, the month in six weeks from the week's first day
>   with today ringed and each day's events as dots in their colours,
>   `‹ ›`/the wheel/Page Up and Down turning it, Today coming back; the
>   picked day's events under it, title and when). `+ New event`, a double
>   click on a day, or Enter open the **form in the month's place**: title
>   (focused, Enter keeps it), date, all day, start and end (`−`/`+` and the
>   wheel step a quarter hour, a click types `3pm`/`15:30`; the end follows
>   the start), repeat (never, daily, weekly, monthly, yearly), a
>   **reminder per event** (none, at start, 5/10/15/30 min, 1 hour, 1 day
>   before; all day: the morning of or the day before, at 9 AM), six
>   colours, notes. A click on an event changes it; Delete asks first, and
>   of a repeating one "this day" (a `skip` date) or every day. Resting the
>   pointer on the clock 0.5 s shows a **card** (the whole date, the rest
>   of today; it takes no input), and a dot before the clock says today has
>   events still to come. **Events are the bar's own** (not the Command
>   Center's): `~/.lantern/config/control-center/calendar.toml`, one
>   `[[event]]` each, edited by hand and followed live (inotify).
>   **Reminders** are a thread of their own (the clock's real-time minute
>   sleep, so on time after a suspend), said with `notify-send -a Calendar`;
>   one missed while the bar was down or the machine slept is said when it's
>   back if its event isn't over, never twice (`calendar-reminded` keeps
>   where it looked to). System Settings' Calendar card: weeks start on
>   Sunday or Monday, the dot on or off (`[control_center]`
>   `calendar_week_start`, `calendar_dot`). Not yet: events over several
>   days, changing one day of a repeating event, an end date for repeats,
>   `.ics` import/export.

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
- [x] System tray: battery, WiFi, Bluetooth, sound, brightness,
      workspace number, Game Mode, their popups, wheel shortcuts, and the
      System Tray card in System Settings (see above)
- [x] Mini monitors: CPU, GPU, RAM, their popups, the Mini Monitors card
      in System Settings (see above)
- [x] Calendar behind the clock: month popup, events with times, repeats,
      reminders and colours, the clock's card and dot, the Calendar card
      in System Settings (see above)

## Next (ideas, unordered)

Planned order (Alva, 2026-10-16): clock → app launcher (far left) → pins +
open windows dock (middle).

- [x] Tray: WiFi password entry, hidden networks, Forget (see above)
- [x] Tray: Bluetooth pairing (both ways), Forget, files (see above)
- [x] SNI app icons (Steam & co.), see above

## Known gaps

Fixed 2026-10-17:
- **Command Center autostart:** a leftover `~/.config/autostart` entry
  started the Command Center beside the bar at login, whatever the panel
  (holding obexd's one agent, fighting over BlueZ's default agent). The
  session manager now skips autostart entries for anything it runs itself
  (its daemons, either panel), saying so in its log; the entry is gone.
- **Monitor gone for long:** taken away, the bar waits for a monitor (sysfs
  connectors) and starts over in place, the same pid, with growing waits
  if it's taken again soon (`LNTRN_CC_TAKEN_AWAY`): the session's five
  restarts are never spent on it.
- **Auto hide and a monitor below:** the bar tells the compositor it's out
  of sight (`edge:<namespace>:1`); the pointer stops on that edge's last
  row, where the bar hears it, and a 40 px push more crosses
  (`lntrn-compositor/src/edge_barrier.rs`).
- **Blur:** the plate, the tray popups, the launcher and the dock's cards
  are as see-through as the windows (`windows.background_opacity`), with no
  shadows under them then, and tell the compositor where they are
  (`blur:<namespace>\t<x> <y> <w> <h> <r>`, `glass.rs`); it blurs everything
  under the top layers behind them (`lntrn-compositor/src/layer_blur.rs`).

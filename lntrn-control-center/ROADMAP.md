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
> - **Style:** floating (Plasma 6): inset by `[window_manager] gap` on the
>   bottom and gap + 8 on the left/right, rounded corners, a hairline edge,
>   solid plate.
> - **Height:** 64 logical px (bumped from 56). Exclusive zone = 64; the
>   compositor adds the bottom margin, so windows stop at the plate's top edge.
> - **Keyboard:** none. Clicking the bar never steals focus. Switch to
>   on-demand (`ShellRequest::SetLayer`) once something in it takes typing.

## Done

- [x] Layer surface on the primary monitor, exclusive zone, floating plate
- [x] Follows `lantern.toml` live via inotify (no idle frames): accent,
      background colour / style, gap; a new primary restarts the bar there
- [x] Single instance (`flock` in `$XDG_RUNTIME_DIR`)

## Next (ideas, unordered)

- [ ] Launcher button (left)
- [ ] Task list: running windows (foreign-toplevel)
- [ ] Clock (right), `request_redraw_after` to the next minute
- [ ] System tray / quick toggles (audio, network, bluetooth, battery)
- [ ] De-float when a window is maximized, like Plasma 6

## Known gaps

- **Monitor goes away:** the compositor closes the layer surface, and in LUI2
  closing the main surface quits the app. Fix: add the bar to
  `lntrn-session-manager`'s supervised `DAEMONS` (restart on exit), and/or
  teach LUI2 to report outputs and keep the app alive. Both are other crates:
  ask first.
- **Autostart:** not started at login yet (same `DAEMONS` list).
- **Blur:** the compositor doesn't blur behind layer surfaces, so the plate
  stays solid rather than following `windows.background_opacity`.

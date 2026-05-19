# Known Issues — fix later

## Minimizing a maximized window glitches on restore

**Steps to reproduce:**
1. Maximize a window (Super+Up).
2. Minimize it (Super+Down).
3. Click its tray icon to restore.

**Observed:** Window appears at the wrong location — looks like it lands
somewhere between monitors / off-screen / partially clipped. Sometimes
only a border is visible. Disappears or behaves erratically.

**Suspected root cause:**
- `minimize_surface` captures `entry.location = workspaces.element_location(&window)`
  *before* clearing the maximize state.
- The maximize state stays in `maximized_windows` while the window is
  minimized (it's never cleared during minimize).
- On `restore_minimized_surface`, the code checks `is_maximized(surface)`
  — sees it's still flagged as maximized — and re-maps to the current
  output_geo at the maximized rect. But the captured `entry.location`
  was the maximize *target origin* (e.g. (0, 0) on output1), and any
  multi-monitor coord math may be reading the wrong output_geo when
  the window's recorded workspace doesn't match the location's output.

**Likely fix paths:**
- **A (cheap):** Disallow minimize while maximized. `minimize_surface`
  returns false (or first unmaximizes silently) if `is_maximized`.
- **B (correct):** Snapshot the maximize.restore rect into
  `MinimizedWindow` alongside `location`. On restore, if the snapshot
  exists, re-engage maximize state cleanly with that restore rect (so
  the next Super+Down unmaximizes back to the right pre-max rect).

Recommended: **B**, then leave the user the choice of whether
"minimize → restore → still maximized" feels right or whether it
should restore to the pre-maximize rect.

**Related files:**
- `lntrn-compositor/src/window_management/minimize.rs`
- `lntrn-compositor/src/window_management/maximize.rs`
- `lntrn-compositor/src/state.rs` (MinimizedWindow struct)

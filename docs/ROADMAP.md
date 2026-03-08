# Lantern DE — Feature Roadmap

## Remaining Work

### Window Management
- [ ] Infinite desktop — continuous 2D canvas instead of fixed workspaces, pan with Super+middle-drag, zoom for overview, named regions as soft landmarks
- [ ] Virtual workspaces — multiple desktops, Super+1-9 to switch, move windows between (alternative to infinite desktop)
- [ ] Tiling layouts — i3/Sway-style split h/v, tabbed, stacked, auto-tile on spawn
- [ ] Sticky windows — windows that appear on all workspaces
- [ ] Picture-in-Picture — always-on-top small floating window
- [ ] Window rules — per-app defaults (always float, tile, start maximized, specific workspace, custom opacity)
- [ ] Fullscreen — proper exclusive fullscreen for games/video
- [ ] Super+drag to move — compositor-level, no client grab needed
- [ ] Super+right-drag to resize — compositor-level resize
- [x] Scratchpad — dropdown terminal, Super+` to toggle slide in/out from top (commented out, waiting for lntrn-terminal)

### Visual Polish
- [ ] Blur — gaussian blur behind transparent windows (custom GLES shaders)
- [x] Window thumbnails in Alt+Tab — live miniature previews while cycling

### Desktop Integration
- [ ] Desktop widgets — layer shell Background surfaces (clock, sysmon, calendar)
- [ ] Notifications — layer shell overlay (mako/dunst would just work already)
- [ ] "App on desktop" — transparent layer shell on Background layer (conky-style)
- [x] Screen edge actions — hot corners trigger actions (e.g. overview, show desktop)

### Bar Features (lntrn-bar)
- [ ] Auto-hide — panel slides away when not in use (bar-side, not compositor)
- [ ] Window peek — hover taskbar entry to highlight/preview a window

### Input & Gestures
- [ ] Touchpad gestures — 3-finger swipe for workspace switch, pinch for overview
- [ ] Mouse button bindings — configurable mouse button actions
- [ ] Input method — zwp_input_method_v2 for CJK/emoji input

### Wayland Protocols
- [x] xdg-foreign — parent-child window relationships across apps
- [ ] zwp_pointer_constraints — pointer lock/confinement for games/FPS
- [ ] zwp_relative_pointer — relative mouse motion for games
- [ ] ext-session-lock — screen locker protocol
- [ ] wlr-output-management — lets wlr-randr configure monitors
- [ ] xwayland — run X11 apps (big but important for compatibility)

### System Integration
- [ ] Multi-monitor — hotplug, layout config, per-monitor scaling
- [ ] Screen lock — ext-session-lock-v1
- [ ] Power management — DPMS, idle timeout
- [ ] Clipboard manager — persist clipboard after app closes
- [ ] Gamma/night light — wlr-gamma-control for redshift/gammastep

---

## Priority Order

### Tier 1 — Core Usability
1. Infinite desktop (or virtual workspaces)
2. Super+drag move / Super+right-drag resize
3. Fullscreen support

### Tier 2 — Visual & UX
4. Blur behind transparent windows
5. Clipboard manager
6. Gamma/night light

### Tier 3 — Compatibility
7. zwp_pointer_constraints + zwp_relative_pointer (games)
8. xdg-foreign
9. xwayland

### Tier 4 — Advanced
10. Tiling layouts
11. Window rules
12. Multi-monitor
13. Screen lock
14. Touchpad gestures
15. Desktop widgets
16. Input method

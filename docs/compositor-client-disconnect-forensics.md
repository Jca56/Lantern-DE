# Compositor ↔ client connection deaths — forensics handoff

From the lantern-studio Claude session, 2026-08-15 evening (MDT). Alva is
spinning up a fresh session to clean house in lntrn-compositor; this is
everything the client-side investigation established, so you don't re-dig it.
All log timestamps below are UTC (local is MDT, UTC-6).

## The disease

Wayland clients of lntrn-compositor intermittently lose their connection
under interactive load. Observed client-side manifestations ("four costumes"
— all one disease, caught at different points):

1. `ERROR winit ... Error dispatching event loop: other error during loop
   operation` then clean exit 1 (winit's calloop 0.13 swallows the inner
   error text — upgrading winit or logging compositor-side is how you ever
   see the real reason).
2. Silent `Event loop exited: Exit Failure: 1` with NO error line (winit's
   Wayland backend `connection.flush()` failure path sets exit code 1 and
   returns without logging — winit-0.30.13 wayland/event_loop/mod.rs:284).
3. `Io error: Broken pipe (os error 32)` + SIGSEGV during teardown
   (exit 139).
4. Zombie: window vanishes from the desktop, client process keeps spinning
   at 100% CPU forever, absolutely nothing logged anywhere. (lantern-studio
   now has a watchdog for this: >120 consecutive surface-Lost acquires →
   autosave → exit(1).)

## Evidence that it's compositor-side and chronic

- lntrn-command-center's log has **1000+ lifetime "Broken pipe" crash
  entries**; dated ones run **1–5 per day, steadily, since at least
  Aug 2** — long before any recent client changes. It just auto-restarts,
  so nobody noticed.
- lantern-studio's log shows identical dispatch-error deaths going back
  months (always near file-dialog close, i.e. focus/window churn moments).
- 2026-08-16T02:04:26Z, the compositor's OWN loop logged
  `calloop::loop_logic: Received an event for non-existent source
  reg_token=TokenInner { id: 10, version: 5765, sub_id: 0 }` — internal
  event-source lifecycle issue. Historical instances of the same warning
  exist (id: 3) going back weeks.
- The compositor **never logs client disconnects or posted protocol
  errors** (zero matches for disconnect/kill/dead-client in compositor.log).
  Every investigation dead-ends on this silence. Studio-side bisection
  (headless rig) cleared every client-side operation individually.
- Three studio deaths on 2026-08-16 (UTC): ~02:53:06 (costume 1),
  ~03:10:19 (costume 3, reproduced by rig), ~03:29–03:31 (costume 4 —
  window left the alt-tab switcher with NO unmap/close logged by the
  compositor while the process lived; user was just color-picking and
  using paint bucket, no dialogs, no SVG).

## Environment facts

- Running compositor: `~/.lantern/bin/lntrn-compositor --udev`, binary
  mtime Aug 13 01:42, process up since Aug 14 20:13 (local). Built from a
  **dirty working tree** — the exact source state is unrecoverable.
- Crash rate did NOT change after the Aug 13 deploy → the Aug 13 build is
  probably not the cause; the disease predates it.
- Last committed compositor change: `2037589` "Firefox fix" (Jun 20).

## ⚠ Uncommitted compositor changes in the tree RIGHT NOW

~255 lines across 10 files, NOT in the running binary, WILL ship with the
next build (post-restart, if rebuilt):

- **Explicit sync**: a `linux-drm-syncobj-v1` global (udev.rs). Once
  advertised, NVIDIA Vulkan/EGL clients switch to explicit sync — that
  changes frame submission for every wgpu app (lantern-studio first in
  line; the GPU is an RTX 3080 Ti). If client crashes change shape after
  the next deploy, suspect this first. Smithay's commit-blocker path here
  deserves review before it ships: a stuck blocker = frozen client
  swapchain = costume 4; a rejected sync point = posted protocol error =
  costumes 1–3.
- **Dead-window reap refactor**: `toplevel_destroyed` handler +
  `reap_dead_toplevel`/`reap_dead_windows` (xdg_shell.rs, lifecycle.rs).
  Looks like deliberate cleanup of the ghost-dock-entry race; review that
  `find_mapped_window`/`unmap_window_everywhere` can never fire for a
  living client.
- Also touched: handlers/compositor.rs, foreign_toplevel.rs, state.rs,
  udev_device.rs, winit.rs, render/surface.rs. Nothing here is committed;
  decide their fate before cleaning house.

## Recommended first moves (highest value first)

1. **Log client disconnects and posted protocol errors.** Smithay:
   implement/extend `ClientData::disconnected` with reason, and log
   anywhere the compositor posts a protocol error or drops a client.
   Until this exists, every client crash is unattributable — this single
   change converts every future incident into evidence.
2. Chase the compositor's own calloop "non-existent source" warnings —
   an event source being dispatched after removal is exactly the kind of
   bug that can corrupt per-client I/O state.
3. Review client socket buffer handling / flush behavior around bursts
   (crashes cluster at focus handoffs and window churn: dialog close,
   alt-tab, rapid clicking).
4. Gate or instrument the explicit-sync global before it ships.

## Client-side tools you can use

- lantern-studio has an env-gated repro rig (inert without the vars):
  `LANTERN_TEST_SECOND_SVG=defer LANTERN_TEST_LEVEL=real lantern-studio`
  opens SVG tabs on a timer — it reproduced costume 3 once (not
  deterministic; interactive desktop churn increases the hit rate).
- Studio's surface-lost watchdog means costume 4 now self-reports:
  grep its log for "connection presumed dead".

---

## ✅ RESOLVED 2026-08-15 (compositor-session Fable, same evening)

**Root cause found: wayland-backend 0.3.12's fixed 4 KiB per-client send
buffer.** `BufferedSocket::write_message` (socket.rs:225): buffer full →
flush → client's kernel socket full → `WouldBlock` tolerated → retry fails →
`E2BIG` → `client.rs:209` kills the client with `ConnectionClosed`. Zero
logging anywhere in the chain. Any client ~4 KiB + one kernel socket
(~208 KiB) behind during an event burst was executed on the spot. Explains
all four costumes (costume 4 = kill → Space::refresh purges the window with
no unmap logged → NVIDIA swapchain SURFACE_LOST → client spins), the burst
clustering, CC's 1,023 lifetime broken pipes (daily since May 18), and the
mid-June rate drop (CC's drain fix narrowed the stall window).

**Fix shipped (built + deployed to ~/.lantern/bin, effective next restart):**
- wayland-backend 0.3.12 → 0.3.17 (lock bump; upstream made the buffer
  growable — same fix libwayland 1.23 made) + direct dep with feature
  `libwayland_server_1_23` (cfg-only on rs backend, gates the new API).
- `set_default_max_buffer_size(4 MiB)` at listener init (state.rs).
- `ClientData::disconnected` now logs EVERY disconnect with pid + exe
  (captured at connect via new `security::peer_identity`) + full
  ProtocolError details. The silence is over.

**Exonerated:** explicit sync (was already live in the Aug 13 binary —
"dirty tree" assumption above was wrong; blockers verified to
PostAction::Remove themselves). The calloop "non-existent source" warnings
are benign per-frame watchdog/render-timer insert/remove churn (token
version 5765 = slot reuse count), not client I/O corruption.

**Also fixed in passing:** tree didn't compile — `refresh_fractional_scales`
called `workspaces.all_windows()` which doesn't exist in committed
workspaces.rs (a hunk of the Aug 13 dirty build was lost); now iterates the
global Space + minimized list.

**Verify after restart:** compositor.log should show
"client disconnected" lines with pid/exe. If broken-pipe crashes persist,
they're now attributable — check reason + whether the process was alive.

# Native Smooth Resize — Implementation Plan

**Status:** Planned, not yet started
**Target:** `lntrn-compositor`
**Scope:** ~200 new lines, ~30 refactored, 6 files touched
**Risk:** Low (gated by existing trusted-client mechanism)

---

## Goal

Replace the current crossfade-during-resize "workaround" with **true GPU-side
scaling** of the live window content for Lantern-native (trusted) apps. The
compositor stretches the *existing committed buffer* to fit the interpolated
rect throughout the animation, then sends a single configure at the end —
no snapshot texture, no alpha crossfade, no double-render.

Untrusted clients (Wine, Electron, generic Wayland apps) keep the existing
crossfade path. Zero behavioral change for them.

## Background

### What we do today (`render/surface.rs:584-700`)

When a window starts an animated resize:

1. Compositor snapshots the old surface as a texture.
2. Configure event fires **immediately** at the target size.
3. Client commits a new buffer at the target size.
4. During the animation, BOTH the snapshot AND the live surface render into
   the *interpolated* rect via `RescaleRenderElement`, with
   `snap_alpha = (1 - progress)` and `live_alpha = progress`.
5. The alpha crossfade hides the awkward stretch between
   "buffer at final size" and "rect at intermediate size".
6. At rest: `scale=1.0` no-op, snapshot gone.

### What already exists in our favor

- ✅ `wp_viewporter` advertised by compositor, used end-to-end by Lantern apps
- ✅ `wp_fractional_scale` advertised + per-surface preferred scale
- ✅ Trust mechanism (`security/client_trust.rs`): SO_PEERCRED + canonical
      exe path → "is this a `~/.lantern/bin/` binary?"
- ✅ Lantern apps react to configures in 1–2 frames, redraw at new size
      using viewport destination
- ✅ `RescaleRenderElement` already wraps the surface in the render pipeline

### Surface element wrappers (`render/surface.rs:621-691`)

```
WaylandSurfaceRenderElement
   → RescaleRenderElement   (per-axis scaling around phys_loc)
   → RoundedSurfaceElement  (SDF corner mask, optional)
```

## Design decisions (locked)

| Decision | Choice | Reason |
|----------|--------|--------|
| Trust gate | Auto-opt-in for `~/.lantern/bin/` binaries | Simplest. We control these apps. |
| Aspect-ratio mismatch | Stretch + short animation (~150ms) | Brief enough to be invisible |
| Handoff polish | Hold scale until matching commit | Kills the 1–2 frame snap at animation end |
| Slice size | Full implementation in one PR | Coherent change, no half-state |

## Implementation phases

### Phase A — Plumbing (~50 lines)

**`lntrn-compositor/src/security/client_trust.rs`**

Add a surface-level wrapper around the existing client trust check:

```rust
pub fn is_trusted_surface(&self, surface: &WlSurface) -> bool {
    let Some(client) = surface.client() else { return false };
    self.is_trusted_client(&client)
}
```

**`lntrn-compositor/src/window_state_anim.rs`**

Extend `WindowStateAnim` to track animation mode and deferred-configure state:

```rust
pub enum AnimMode {
    Fade,
    Smooth {
        deferred_configure: Option<Size<i32, Logical>>,
        awaiting_commit_size: Option<Size<i32, Logical>>,
    },
}

pub struct WindowStateAnim {
    // ...existing fields...
    pub mode: AnimMode,
}
```

New methods:

- `animate_smooth(surface, src, dst, final_size)` — starts a smooth anim with
  deferred configure target stored
- `take_due_deferred_configure(surface) -> Option<Size>` — called when the
  animation completes; pulls the deferred configure size and clears it
- `clear_held_scale_if_matched(surface, committed_size)` — called from
  the commit handler; if the new buffer size matches the held target,
  drop the residual scale and let the wrapper become a no-op
- `held_scale_for(surface) -> Option<(f64, f64)>` — current scale to apply
  during the "post-animation, awaiting matching commit" window

### Phase B — Caller refactor (~30 lines, mechanical)

New helper, likely in a new file `window_management/smooth_resize.rs`:

```rust
impl Lantern {
    /// Animate a window from `src` to `dst`. For trusted clients, uses the
    /// new deferred-configure smooth path (single buffer, GPU-scaled). For
    /// untrusted clients, falls back to the existing crossfade path.
    pub fn animate_resize(
        &mut self,
        surface: &WlSurface,
        window: &Window,
        src: Rectangle<i32, Logical>,
        dst: Rectangle<i32, Logical>,
    ) {
        if self.is_trusted_surface(surface) {
            // Smooth path: scale existing buffer, defer configure until end.
            self.window_state_anim.animate_smooth(surface, src, dst, dst.size);
            self.remap_tracked_window(window.clone(), dst.loc, true);
            // NOTE: NO window.configure_rect(dst) here.
        } else {
            // Fade path (current behavior).
            window.configure_rect(dst);
            self.remap_tracked_window(window.clone(), dst.loc, true);
            self.window_state_anim.animate_default(surface, src, dst);
        }
    }
}
```

Replace the `configure_rect + animate_default` pair at these call sites:

- `window_management/half_pose.rs::apply_pose`
- `window_management/maximize.rs::maximize_surface` and `unmaximize_surface`
- `window_management/solo_tile.rs::solo_tile_surface` and `unsolo_tile_surface`
- `workspaces.rs::move_focused_to_workspace` (slide-off animation)

### Phase C — Render-path branch (~60 lines in `render/surface.rs`)

After computing `eff_rect` and `combined_scale`, branch on the animation mode:

```rust
match anim.mode {
    AnimMode::Fade => {
        // Existing snapshot + crossfade path (unchanged).
        // ...current code lines 584-700...
    }
    AnimMode::Smooth { .. } => {
        // No snapshot. Just scale the live surface.
        // combined_scale already accounts for committed_size vs eff_rect.
        let elements = render_elements_from_surface_tree(/* ... */);
        for elem in elements {
            let scaled = RescaleRenderElement::from_element(
                elem, phys_loc, combined_scale,
            );
            // optional rounded corners
            // push to scene
        }
    }
}
```

In the tick loop where the animation completes, fire the deferred configure:

```rust
if let Some(final_size) = state.window_state_anim
    .take_due_deferred_configure(&surface)
{
    if let Some(window) = state.find_mapped_window(&surface) {
        let loc = state.workspaces.element_location(&window)
            .unwrap_or_default();
        window.configure_rect(Rectangle::new(loc, final_size));
    }
}
```

### Phase D — Handoff hold (~30 lines)

**`lntrn-compositor/src/handlers/compositor.rs`**

In the surface commit handler, after the buffer commit lands:

```rust
fn commit(&mut self, surface: &WlSurface) {
    // ...existing commit logic...

    if let Some(committed_size) = buffer_size_of(surface) {
        self.window_state_anim
            .clear_held_scale_if_matched(surface, committed_size);
    }
}
```

In `render/surface.rs`, after the animation ends, keep applying the held
scale until cleared:

```rust
let scale = if let Some(held) = state.window_state_anim.held_scale_for(&surface) {
    held
} else if anim.is_active() {
    combined_scale
} else {
    (1.0, 1.0)
};
```

This eliminates the 1–2 frame snap where the still-old buffer would render
1:1 in the new logical rect before the client's redraw catches up.

### Phase E — Edge cases (~30 lines)

- **`forget_window`** (`window_management/lifecycle.rs`): drop any deferred
  configure + held scale state for the surface.
- **Workspace move**: if a window is animating off-screen, fire the deferred
  configure immediately at jump time so the off-screen window doesn't
  silently fall behind on resize state.
- **Re-resize mid-animation**: `animate_smooth` replaces the deferred target
  with the new one. Old configure never fires. (Natural drop via overwrite.)
- **New window map**: bypass the deferred path on initial configure — the
  first commit needs to know what size to render at.

## Files touched

| File | Change | Lines |
|------|--------|-------|
| `security/client_trust.rs` | +1 helper | ~5 |
| `window_state_anim.rs` | AnimMode enum + deferred state + 4 methods | ~80 |
| `render/surface.rs` | Branch on anim mode + held-scale path | ~60 |
| `window_management/smooth_resize.rs` *(new)* | `animate_resize` dispatcher | ~30 |
| `window_management/{half_pose,maximize,solo_tile}.rs` | Call-site swaps | ~15 |
| `window_management/lifecycle.rs` | Cleanup hook | ~10 |
| `handlers/compositor.rs` | Commit hook for held-scale clearing | ~10 |

Total: **~210 lines new, ~30 refactored**.

## Risk analysis

| Risk | Severity | Mitigation |
|------|----------|------------|
| Aspect-stretch on big rect changes (corner→max) | Medium | Cap animation duration to ~150ms; ease curves that snap fast near end |
| Slow client commit at handoff causes scale mismatch | Medium | Held-scale-until-matching-commit (Phase D). The genuinely subtle piece. |
| Deferred configure lost on edge case → window stuck wrong size | Medium | Drop on forget_window; fire on workspace move; replace on re-resize |
| Trust gate misclassification | Low | Re-uses existing battle-tested trust check from keychain stack |
| Breaks Wine / Electron resize | None | Untrusted path is unchanged |

## Expected payoff

- ~50% less fillrate per animation frame (1 surface vs 2)
- No "fade ghost" briefly visible during fast pose changes
- Looks identical or better, especially on the new high-refresh displays
- A genuine differentiator for Lantern-native apps:
  *"your window animations are smoother because the OS trusts you."* 💎

## Testing plan

After implementation:

1. **Smoke test**: open lntrn-terminal, pose half-left, half-right,
   middle, ladder up/down. Look for stretch artifacts or "snap" at end.
2. **Aspect stress**: open lntrn-file-manager (wide), pose to corner
   (different aspect). Check for ugly distortion mid-anim.
3. **Slow-app handoff**: Lantern Studio takes a beat to re-render its
   wgpu surface. Pose it mid-startup; check for size mismatch at handoff.
4. **Trust fallback**: open Wine notepad.exe or chromium. Resize via
   pose. Confirm it uses the old crossfade path (still works).
5. **Mid-anim re-resize**: spam pose left/right/middle quickly. No
   stuck state, no orphaned configures.
6. **Window close mid-anim**: kill a posed window during animation.
   No panics, no orphaned deferred configures.

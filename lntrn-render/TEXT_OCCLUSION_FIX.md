# Text Bleeding Through Menus — Root Cause & Fix

## The Problem

Text renders on top of context menus, dropdown overlays, and any popup drawn by the painter. App labels, process names, etc. bleed through menu backgrounds across every app in the DE.

## Why It Happens

The render pipeline has three sequential passes:

1. **Painter pass** (shader.rs) — all shapes: rects, circles, gradients, arcs, shadows
2. **Texture pass** — all icons/images as textured quads
3. **Text pass** (lntrn-render/text) — all queued glyphs rendered last, on top of everything

Text is queued during the draw phase with `text.queue()` / `text.queue_clipped()`, then rendered in bulk AFTER the painter and texture passes. This means text has no z-ordering relative to painter shapes — it always wins.

When a context menu background is drawn via `painter.rect_filled()` in pass 1, and underlying content text was queued in pass 3, the text renders over the menu background.

Current workaround: `text.occlude_rect()` tries to hide/clip text entries that overlap a rect. This is fragile because `max_width_bits` (layout bound) is wider than actual rendered text, causing false positives. It also can't handle icons (texture pass).

## The Fix: Multi-Layer Rendering

Split rendering into ordered layers. Each layer gets its own painter+text+texture sub-passes, composited in order.

### Approach A: Render Layers (Recommended)

Add a layer concept to the frame:

```
Layer 0 (base):    painter shapes + textures + text for main content
Layer 1 (overlay): painter shapes + textures + text for popups/menus
```

Each layer renders to the same surface but in sequence — Layer 0's text renders before Layer 1's painter, so menu backgrounds correctly cover underlying text.

Implementation:
- `Painter` gets a `set_layer(n)` method or instances are split per layer
- `TextRenderer` gets a `set_layer(n)` method, queuing into separate buckets
- `TexturePass` similarly splits draws by layer
- Render loop: for each layer, run painter pass -> texture pass -> text pass
- Draw code calls `painter.set_layer(1)` / `text.set_layer(1)` before drawing overlays

### Approach B: Text-as-Quads

Render text glyphs as textured quads in the painter pipeline directly, so they respect draw order naturally. This is how most GPU UI toolkits work (egui, imgui, etc).

Implementation:
- Rasterize glyphs to a texture atlas (already done in lntrn-render/text)
- Instead of a separate text render pass, emit instanced quads with UV coords into the glyph atlas
- Text gets interleaved with shape draws in the painter's instance buffer
- No separate text pass needed — everything is one draw call

This is more work but eliminates the layering problem entirely and may improve performance (fewer render passes, single draw call).

### Recommendation

Approach A is simpler and less risky — the text renderer stays intact, we just bucket its output into layers. Approach B is cleaner long-term but requires reworking how text flows through the pipeline.

## Files Involved

- `lntrn-render/draw/src/painter.rs` — shape instance buffer, render pass
- `lntrn-render/draw/src/shader.rs` — WGSL shader (unchanged for Approach A)
- `lntrn-render/text/src/lib.rs` — text queue, glyph rasterization, render
- `lntrn-render/src/lib.rs` — re-exports, TextRenderer / Painter types
- Every app's render loop (layershell.rs, terminal, file manager, etc.) — frame submission order

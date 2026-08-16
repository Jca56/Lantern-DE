# Lantern Text Engine — Implementation Plan

Build our own text rendering + layout + shaping stack from scratch to fully
replace **glyphon 0.10** + **cosmic-text 0.15** (which pulls in `harfrust`,
`skrifa`, `swash`, `fontdb`, and the `unicode-*` crates).

**Ground rules (per project decision 2026-06-07):**
- Build *everything* ourselves. No external text crates in the final result.
- Do **every** feature, up to and including Tier-3 (BiDi, complex scripts,
  variable fonts, color emoji). No corners cut.
- Implement phases in order of **easiest/fastest → hardest/slowest**, respecting
  data dependencies (can't rasterize before parsing; can't shape before cmap).
- **Do NOT remove glyphon/cosmic-text until the very end.** The DE stays fully
  working on the old stack the entire time. We develop standalone and only swap
  in Phase 12.

## Coexistence strategy (important)

The package name `lntrn-text` is **already taken** by the glyphon wrapper at
`lntrn-render/text/`. Two packages can't share a name in one workspace, so:

- New crate lives at repo root `/lntrn-text/`, package name **`lntrn-type`**
  during development.
- It is **NOT** added to the workspace `members` list while in development — it
  builds standalone (`cargo build -p lntrn-type` from its own dir, or via a
  `[patch]`-free path). This keeps it from touching the live DE build.
- At Phase 12 we delete `lntrn-render/text`, rename `lntrn-type` → `lntrn-text`,
  add it to `members`, and flip `lntrn-render`'s dependency.

## The drop-in API contract

The final crate must re-expose this surface **identically** so all ~860 call
sites and the `lntrn-render` re-export need zero changes:

- `TextRenderer::new` / `new_monospace` / `with_options`
- `queue` · `queue_styled` · `queue_full` · `queue_family` · `queue_clipped`
- `measure_width` · `measure_width_styled` · `measure_width_full` ·
  `measure_width_family` · `measure_ink_height_family`
- `push_clip` · `pop_clip` · `occlude_rect`
- `set_layer` · `layer_count` · `render_layer` · `render_queued`
- `clear` · `load_font_data` · `stats` · `TextCacheStats`
- `FontWeight` · `FontStyle` enums
- `impl TextPass for TextRenderer` (from `lntrn-draw`)

Behavioral details to preserve: colorless cached layouts (color applied at render
time via per-quad default_color), `quantize_px` 0.25px grid for animated sizes,
`MAX_CACHED_LAYOUTS = 512` LRU, line_height = font_size * 1.2, clip-rect bounds.

## Module architecture

```
lntrn-text/  (package: lntrn-type during dev)
  src/
    lib.rs            public TextRenderer — the drop-in contract
    font/
      sfnt.rs         sfnt/ttc container + table directory
      tables/         head hhea hmtx maxp os2 name post
      cmap.rs         char→glyph (fmt 4,12,6,0; 14 variation selectors)
      glyf.rs         TrueType outlines (loca + glyf, composites)
      cff.rs          CFF / CFF2 Type2 charstring interpreter
      variations.rs   fvar gvar avar (variable fonts)
      color.rs        COLR/CPAL, CBDT/CBLC, sbix, SVG-in-OT
      db.rs           discovery: scan dirs, index family/style/coverage, fallback
    shape/
      buffer.rs       unicode buffer + item segmentation
      bidi.rs         UAX#9 bidirectional algorithm
      script.rs       UAX#24 script itemization
      cluster.rs      UAX#29 grapheme/word segmentation
      linebreak.rs    UAX#14 line break opportunities
      gsub.rs         OpenType GSUB (ligatures/substitution)
      gpos.rs         OpenType GPOS (kerning/mark positioning)
      shaper.rs       itemize → per-run shape → glyph buffer
      simple.rs       fast path: cmap + advance (monospace/no-feature)
    raster/
      outline.rs      bézier flattening (quad+cubic → lines)
      scanline.rs     signed-area coverage AA rasterizer
      hint.rs         hinting / grid-fitting (later)
      subpixel.rs     LCD subpixel AA (later)
      emoji.rs        color glyph compositing
    layout/
      line.rs         line layout, run building, wrapping
      align.rs        alignment / justification
      cache.rs        shaped-layout LRU (ported from current lib.rs)
    gpu/
      atlas.rs        shelf/skyline glyph atlas packer + upload
      pipeline.rs     wgpu pipeline(s)
      shader.wgsl     coverage / subpixel / color shaders
      render.rs       queue → quads → draw, clip bounds, layers
  examples/
    preview.rs        standalone window harness; side-by-side vs glyphon
  tests/              golden-image + per-layer unit tests
```

Each layer is independently testable. Keep files < 600 LOC (split tables/shapers
by concern). The standalone `preview` harness is built early and used to eyeball
every phase against glyphon output.

---

## Phases (easiest → hardest)

### Phase 0 — Scaffold + GPU plumbing 🟢 ✅ DONE
Prove our own GPU text path end-to-end with a fake glyph before any font parsing.
- [x] Create `/lntrn-type` crate (`lntrn-type`), depend on `wgpu`, `lntrn-gfx`,
  `lntrn-draw`. Standalone via root `exclude` (not a workspace member).
- [x] Stub `TextRenderer` with the **exact** public API signatures (todo!()
  bodies) so the drop-in shape compiles — `src/lib.rs`.
- [x] Glyph atlas + shelf packer (`src/gpu/atlas.rs`, R8Unorm coverage texture).
- [x] wgpu pipeline + coverage `shader.wgsl` (`src/gpu/pipeline.rs`,
  premultiplied-alpha blend).
- [x] `examples/preview.rs` headless harness: AA coverage glyphs → offscreen →
  readback → `phase0.png` via an in-house PNG encoder; asserts coverage drew.
- **Exit:** ✅ sampled-coverage quads render through our pipeline. 10 quads /
  ~30.8k lit px; AA + per-quad color + alpha blending verified visually.

### Phase 1 — TrueType parsing + rasterization 🟢 ✅ DONE (THE first-text milestone)
- [x] sfnt container + table directory + `ttcf` collections (`src/font/sfnt.rs`);
  `head` `maxp` `hhea` `hmtx` (`src/font/tables.rs`).
- [x] `cmap` formats 0, 4, 6, 12 with best-subtable scoring, zero-copy
  binary-search lookups (`src/font/cmap.rs`).
- [x] `loca` + `glyf` simple + composite glyphs (F2.14 transforms, nesting,
  implied on-curve midpoints, off-curve contour starts) — `src/font/glyf.rs`.
- [x] Adaptive quadratic flattening (≤0.25px error) + signed-area scanline AA
  rasterizer with winding via delta sign (`src/raster/`), unit-tested.
- [x] Wired: `queue` (incl. `\n`), `measure_width`, `load_font_data`, glyph
  cache keyed by (font, glyph, 0.25px-quantized size) into the Phase 0 atlas.
- **Exit:** ✅ JetBrains Mono + Inter render from real outlines with correct
  advances (`phase1.png`): pangrams, punctuation, composite accents (Åéçñü),
  12–44px ladder, per-quad color, multi-line, translucent blending. Monospace
  advance equality + cache-hit-on-requeue asserted in the harness. 🎉

### Phase 2 — Font discovery, matching, fallback 🟡 ✅ DONE
- [x] `name` (family IDs 16+1, UTF-16BE/MacRoman) + `OS/2` (weight/width/
  italic) + `post` (isFixedPitch); `head.macStyle` fallback — `font/tables/`
  split into `metrics.rs` / `name.rs` / `os2.rs`.
- [x] Discovery via **targeted reads** (`font/scan.rs`): only header + table
  dir + needed tables are read per file, never the whole font — 2.4k faces
  scan in well under a second. Dirs autodetected at runtime (XDG_DATA_DIRS/
  XDG_DATA_HOME + /usr/share/fonts + /usr/local/share/fonts + ~/.fonts +
  ~/.local/share/fonts + ~/.lantern/fonts). Coverage = merged cmap ranges.
- [x] `font/db.rs`: lazy face loading (parse on first render, auto-disable on
  failure), family/weight/width/style ranking, resolve cache. Defaults mirror
  the glyphon wrapper: sans = lantern.toml via `lntrn_theme`, mono = Noto Sans
  Mono. Unknown families fall back to the default (wrapper parity).
- [x] **Per-glyph fallback**: Lantern fallback family list (same order as the
  wrapper's `LanternFallback`) + coverage-search last resort — kana in a Latin
  UI font finds UDEV Gothic/Noto CJK with zero config. Cached per (char,
  weight, italic).
- [x] `load_font_data` registers embedded fonts in the db (family-matchable).
- [x] All style/family API live: `queue_styled/full/family`,
  `measure_width_styled/full/family`, plus `measure_ink_height_family`
  (pulled forward from Phase 3 — trivial with atlas bearings).
- **Exit:** ✅ `phase2.png`: real bold/italic/bold-italic faces, JetBrains
  Mono via family, カタカナ・ひらがな fallback mid-Latin, Digital-7 7-segment
  clock + ink bounds, unknown-family fallback. Harness asserts bold ≠ normal
  width, mono advance equality, unknown-family == default, kana non-zero,
  cache hits on requeue. CFF/bitmap-only faces (38 on the PC) are skipped at
  scan with a counter until Phases 9/11.

### Phase 3 — Layout + full public API 🟡 ✅ DONE
- [x] `src/layout/` — `line.rs` greedy word wrap at `max_width` with per-glyph
  breaks for overlong words (cosmic `Wrap::WordOrGlyph` parity); `mod.rs`
  colorless layout LRU (512 entries, tick-based, ported semantics; keys =
  text/size/max_width/weight/style/family, all 0.25px-quantized).
- [x] Every remaining API method real: clip stack (`push_clip` intersects like
  the wrapper), `queue_clipped`, `occlude_rect` (entry-level, wrapper logic
  verbatim), layers (`set_layer`/`layer_count`/`render_layer`), `stats`
  (entries = cached layouts, queued = entries).
- [x] Wrapper-parity default bounds: no clip → `[0, 0, screen_w, y+size*1.2]`
  — `queue()` clips to ONE line box, exactly like the old stack (callers
  pre-wrap; see notes in memory).
- [x] Quads build at queue time but clip at render time (proportional UV
  trim), so `occlude_rect` can shrink bounds post-queue. Storing quads by
  value also makes the wrapper's eviction footgun (evicted layout → dropped
  glyphs at render) structurally impossible.
- [x] `lib.rs` split: public API surface stays in `lib.rs` (frozen contract),
  machinery moved to `src/engine.rs`.
- **Exit:** ✅ full API parity. `phase3.png` verified **per-pixel** in the
  harness: one-line cap region empty, wrapped lines lit inside a pushed clip
  and empty below it, occluded region empty with the kept part lit,
  `queue_clipped` slices mid-glyph. Measure ignores wrap bounds (10000px key,
  wrapper parity). The engine could drop in from here — old stack stays until
  Phase 12 regardless.

### Phase 4 — Render quality: AA, gamma, hinting, subpixel 🟡
- Gamma-correct coverage blending to match swash crispness.
- Optional LCD subpixel AA path + shader variant.
- Optional autohinting / grid-fitting for small sizes (user runs ≥16px, so this
  is polish, not critical).
- **Exit:** text as crisp as glyphon at 16–18px in the side-by-side harness.

### Phase 5 — OpenType shaping I: GPOS / kerning 🔴
- `GPOS`: pair kerning, mark-to-base, mark-to-mark, cursive attachment.
- Legacy `kern` table fallback.
- Shaper orchestration with simple-path (`simple.rs`) vs complex-path dispatch.
- **Exit:** properly kerned proportional text.

### Phase 6 — OpenType shaping II: GSUB (ligatures) 🔴
- `GSUB`: ligature, single/multiple/alternate, contextual + chaining context.
- Script/language system selection; feature application (`liga`, `calt`, `dlig`).
- **Exit:** programming ligatures (`=>` `!=` `>=`) fuse in code/terminal. 🎉

### Phase 7 — Unicode segmentation 🔴
- UAX#29 grapheme + word clustering (correct cursor/ZWJ handling).
- UAX#24 script itemization (drives shaping runs).
- UAX#14 line breaking (replace greedy wrap with proper break opportunities).
- Build the Unicode property tables ourselves (codegen from UCD data files).
- **Exit:** correct wrapping + cluster-aware editing.

### Phase 8 — BiDi + complex scripts 🔴 (Tier 3)
- UAX#9 bidirectional algorithm (reordering, embedding levels).
- Arabic joining + mark filtering; Indic reordering shaper(s).
- **Exit:** RTL (Arabic/Hebrew) + basic complex-script support.

### Phase 9 — CFF / CFF2 outlines 🔴
- CFF Type2 charstring interpreter; CFF2 + blend for variable fonts.
- **Exit:** OTF / PostScript-flavored fonts render. (All current fonts are
  TrueType `glyf`, so this is coverage insurance.)

### Phase 10 — Variable fonts 🔴
- `fvar` / `gvar` / `avar`; named-instance + axis selection; CFF2 blend.
- **Exit:** weight/width axes interpolate.

### Phase 11 — Color glyphs / emoji 🔴
- `COLR`/`CPAL` v0 + v1 (gradients), `CBDT`/`CBLC`, `sbix`, SVG-in-OT.
- Color atlas path + shader variant; composite onto the glyph quads.
- **Exit:** 🎵 color emoji render.

### Phase 12 — Integration + swap 🟢 (the payoff)
- Benchmark vs glyphon; tune atlas eviction + cache sizes.
- Rename `lntrn-type` → `lntrn-text`, delete `lntrn-render/text`, add to
  workspace `members`, flip `lntrn-render`'s dependency.
- Remove glyphon, cosmic-text, harfrust, skrifa, swash, fontdb, unicode-* from
  `Cargo.lock`. Regression pass across every app (bar, terminal, code, notepad,
  file-manager, command-center, matrix scene).
- **Exit:** glyphon is gone. The DE renders entirely on Lantern text. 🔦🔥

---

## Open decisions (defaults chosen; flag to change)
- **AA:** grayscale first, subpixel optional in Phase 4. (Default: grayscale.)
- **Unicode data:** generate property tables from UCD at build time via a small
  codegen step in-repo, rather than vendoring a crate. (Default: codegen.)
- **Preview harness:** keep a permanent `examples/preview.rs` that renders the
  same string through both engines for visual diffing during dev. (Default: yes.)

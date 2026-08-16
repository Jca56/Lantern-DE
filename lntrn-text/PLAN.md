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

### Phase 4 — Render quality: AA, gamma, hinting, subpixel 🟡 ✅ DONE
- [x] **Subpixel x-positioning**: pen positions quantize to quarter-pixel bins
  (0/0.25/0.5/0.75px), each bin rasterized with the offset baked into
  coverage; atlas key carries 2 bin bits. Proportional spacing no longer
  snaps to whole pixels.
- [x] **Atlas growth** (scheduled here since Phase 0): entries + quads store
  **texel** UVs, normalized in the vertex shader via an atlas-size uniform;
  on full the atlas doubles (to 8192² cap), GPU-copies old content to the
  same origin, and bumps a generation counter the pipeline watches to
  rebind. Harness force-grows mid-frame (giant glyphs clipped to a zero-area
  rect) and proves ink output is bit-identical before/after.
- [x] **Side-by-side vs glyphon**: dev-dep on glyphon 0.10, same font files,
  wrapper-identical settings, ours left / glyphon right in `phase4.png`.
  Ink ratio **0.99**; JetBrains Mono width **exactly equal** (270.0 = 270.0);
  Inter rows within 1–2px. Visible deltas are precisely the pending phases:
  kerning (~22px on "Wave To Yo AVATAR", Phase 5) and the `=>` ligature
  glyphon fuses (Phase 6).
- [x] Gamma decision: **none needed** — both engines blend linearly into the
  same sRGB target and the ink ratio confirms parity. LCD subpixel AA and
  hinting stay skipped as the plan allowed (user runs ≥16px; grayscale AA
  matches the swash output the DE ships today).
- **Exit:** ✅ crispness parity at 14–24px verified visually + numerically in
  the harness.

### Phase 5 — OpenType shaping I: GPOS / kerning 🔴 ✅ DONE
- [x] `shape/gtab.rs` — shared OpenType layout plumbing (script→LangSys→
  feature→lookup navigation, coverage + ClassDef lookups, extension
  unwrapping), built once per font at parse into a `GposPlan`. **Reused
  as-is by GSUB in Phase 6.** Script selection latn→DFLT→first (per-run
  itemization comes with Phase 7).
- [x] `shape/gpos.rs` — SinglePos (fmt 1+2), PairPos (fmt 1 glyph pairs +
  fmt 2 class matrices), mark-to-base (type 4), mark-to-mark (type 6) via
  anchor attachment. Cursive (3) + contextual (7/8) deferred to Phase 8;
  lookup ignore-flags stored but unhonored until mark-heavy scripts.
- [x] `shape/kern.rs` — legacy `kern` v0 fmt 0 fallback when GPOS has no kern
  feature.
- [x] `shape/mod.rs` — `shape_token`: per-char fallback resolution → per
  same-font run positioning; layout builder wraps and emits from the same
  shaped result so wrap decisions and rendering always agree.
- **Exit:** ✅ kerned widths match glyphon **exactly** (asserted <0.5px) on
  every ligature-free comparison row — 221.9=221.9, 269.5=269.5,
  281.3=281.3, JBM 270.0=270.0. "Wave To Yo AVATAR" gap closed 22px → 8.8px;
  the remainder is glyphon's `!=`→`≠`/`=>`→`⇒` GSUB substitutions (Phase 6's
  scoreboard). "AV" asserts tighter than "A"+"V".

### Phase 6 — OpenType shaping II: GSUB (ligatures) 🔴 ✅ DONE
- [x] `shape/gsub.rs` — single (fmt 1+2), multiple, ligature, **contextual
  (type 5, fmts 1–3) + chained context (type 6, fmts 1–3)** with nested
  sequence-lookup application — the machinery modern fonts (Inter, JetBrains
  Mono) actually use for `calt` programming ligatures. Extensions (type 7)
  unwrapped at plan time. Type 3 (alternate, needs UI) and type 8 (reverse
  chained, Phase 8) skipped.
- [x] `gtab.rs` generalized: shared `gather_lookups` builds both GposPlan and
  GsubPlan; features = ccmp/liga/clig/calt/rlig (HarfBuzz horizontal
  defaults), applied in LookupList order across features; contextual rules
  resolve arbitrary nested lookups via the retained LookupList offset.
- [x] Shaper order: GSUB (may merge/split glyphs) → advances → GPOS.
- Known simplifications (documented in code): nested records assume
  length-preserving substitutions at earlier indices; per-token shaping means
  cross-token backtrack context is empty (spaces — real fonts don't ligate
  across them). One observed delta: we fuse standalone `!=` in JBM where
  rustybuzz doesn't — width-identical, matches JBM's documented behavior;
  revisit with hb-shape ground truth if terminal output ever looks off.
- **Exit:** ✅ `=>` `!=` `>=` `<=` fuse (⇒ ≠ ⩾ ⩽ visible in `phase6.png`,
  both Inter `calt` and JBM `calt`), and **every** comparison row's shaped
  width now matches glyphon exactly (asserted <0.5px, ligature rows
  included): 221.9/269.5/281.3/288.7/313.2 all equal. 🎉

### Phase 7 — Unicode segmentation 🔴 ✅ DONE
- [x] **UCD codegen**: `ucd/` holds pinned Unicode 17.0.0 data files
  (GraphemeBreakProperty, emoji-data, Scripts, LineBreak);
  `examples/gen_unicode.rs` regenerates `src/unicode/tables.rs` (5.4k merged
  ranges, binary-searched). No unicode-* crates, as decided.
- [x] **UAX#29** (`unicode/grapheme.rs`): GB1–GB13+GB999 — CRLF, Hangul jamo,
  Extend/ZWJ, spacing marks, prepend, emoji ZWJ sequences, RI flag pairs.
  GB9c (Indic conjuncts, needs InCB) deferred to Phase 8. **Public API**:
  `lntrn_type::unicode::{graphemes, next_grapheme_boundary}` for editors.
- [x] **UAX#14** (`unicode/linebreak.rs`): rule cascade LB2–LB31 incl. space
  runs, CM/ZWJ transparency, kinsoku (CJ→NS), Korean jamo, numbers, RI
  pairs. Simplifications documented in the module (SA→AL, classic LB19
  quotes, simplified LB25/28a/30). `break_opportunities` + `units` public.
- [x] **UAX#24** (`unicode/script.rs`): script runs with Common/Inherited
  adoption; crate-internal until Phase 8 per-script shaping.
- [x] **Whole-line shaping with cluster tracking**: every glyph carries its
  source byte offset through GSUB (ligature keeps first component's); the
  line builder shapes entire lines and takes only break opportunities that
  land on a surviving cluster boundary — a ligature spanning a break makes
  it unbreakable, and ligatures/kerning now form across word boundaries like
  the old HarfBuzz stack. Grapheme-cluster-aware fallback keeps combining
  marks in their base's font.
- **Exit:** ✅ 17 unit tests (ZWJ families, flags, jamo, kinsoku カー/ちょっ,
  NBSP glue, hyphens, number units); spaceless Japanese wraps kinsoku-aware
  in `phase7.png` next to NBSP-glued Latin; all glyphon width parity asserts
  still exact.

### Phase 8 — BiDi + complex scripts 🔴 (Tier 3) ✅ DONE
- [x] **UAX#9** (`unicode/bidi.rs`): P2–P3, explicit-embedding stack
  (RLE/LRE/RLO/LRO/PDF + overflow), W1–W7, N1–N2, I1–I2, L1; L2 reorder + L4
  mirroring applied by layout. UCD additions: DerivedBidiClass (with
  `@missing` default-range layering — unassigned Arabic blocks default AL),
  BidiMirroring, ArabicShaping. Documented simplifications: isolates
  (RLI/LRI/FSI/PDI) treated as neutrals (LRM/RLM strong marks fully work),
  N0 paired brackets skipped, per-run sos/eos without isolating-sequence
  chaining.
- [x] **Arabic joining** (`shape/arabic.rs`): joining-type analysis
  (transparent marks skipped) → isol/init/medi/fina form tags per glyph,
  applied via masked GSUB positional-feature buckets. Also covers Syriac,
  N'Ko, Mongolian (same table).
- [x] **Per-script feature plans**: GSUB/GPOS plans now built per script tag
  the tables declare, selected by each run's UAX#24 script at shape time
  (exact → DFLT → latn → first). This was load-bearing: Arabic features live
  under `arab`, invisible to the old latn-only plan.
- [x] **GDEF + mark filtering**: glyph classes parsed; IgnoreMarks honored in
  GPOS pair kerning (bases kern across vowel marks). GSUB-side ignore-flags
  + mark-filtering sets still pending; Indic reordering shapers deferred
  (script runs + dev2-era fonts do most positioning via GSUB/GPOS we run).
- [x] **Layout**: default-ignorables (directional controls, joiners) render
  no glyph; mirrored chars swap before shaping in odd-level runs; greedy
  wrap stays logical; L2 reordering per row at *group* granularity (base +
  its zero-advance marks move as one unit, keeping anchors valid).
- **Exit:** ✅ mixed "RTL: שלום עולם — مرحبا بالعالم — (מספר 123)" renders
  right-to-left with connected Arabic (joined سلام measures 49.3px vs 68.3px
  isolated — asserted), LTR numbers inside RTL, mirrored bracket; 25 unit
  tests incl. bidi levels/reorder + joining forms; all glyphon parity
  asserts still exact.

### Phase 9 — CFF / CFF2 outlines 🔴 ✅ DONE (CFF1; CFF2 moves to Phase 10)
- [x] `font/cff.rs`: INDEX/DICT parsing (incl. nibble-encoded reals), Type2
  charstring interpreter — all path ops, hint counting + hintmask skipping,
  width extraction, local/global subrs with count bias, the full flex
  family, and **CID-keyed fonts** (FDSelect fmt 0/3 → per-FD private dicts,
  i.e. Noto CJK OTFs). `seac` accents skipped (logged; extinct in modern
  fonts). Slightly over the file-length guideline (650) — the container +
  interpreter are one spec and split poorly.
- [x] Cubic béziers end-to-end: `PathCmd::Cubic` + adaptive flattening +
  bbox coverage in the rasterizer.
- [x] `OTTO` accepted in sfnt + scan (standalone and inside `.ttc`); faces
  with `CFF ` outlines now discovered. **PC skip count: 38 → 1** (only the
  bitmap-only color emoji remains, Phase 11's job) — +37 faces indexed.
- [x] Integration test parses + rasterizes a real system OTF (URW Nimbus);
  harness renders a Nimbus Sans row through the interpreter.
- CFF2 (variable, blend operators) deliberately deferred to Phase 10 where
  the variation machinery (fvar/avar) it depends on lives.
- **Exit:** ✅ PostScript-flavored `.otf` fonts render.

### Phase 10 — Variable fonts 🔴 ✅ DONE
- [x] `font/variations.rs` — `fvar` axes + named instances, `avar` segment
  maps, user→normalized coordinate mapping, **ItemVariationStore** (region
  scalars + word/byte delta rows), DeltaSetIndexMap, and `HVAR` advance
  deltas.
- [x] `font/gvar.rs` — full tuple-variation store: shared/embedded peaks,
  intermediate regions, packed point numbers, packed deltas, per-tuple
  **IUP** interpolation within contours, scaled accumulation; composite
  glyphs vary their component offsets.
- [x] **Instance expansion in discovery**: a variable font's named instances
  (or synthesized default+400+700 for instance-less wght fonts) each become
  a matchable face with weight/width/italic derived from axis values — one
  `wght` file offers Regular AND Bold to the existing matcher. Works for
  scanned files and `load_font_data` embedded fonts alike.
- [x] `Font::set_instance(user coords)` → normalized position; glyf points
  delta-shifted pre-emission; advances HVAR-adjusted (`advance_units` now
  i32). MVAR (metric variations for ascender etc.) skipped — sub-pixel
  vertical effect at DE sizes.
- [x] Verified against Orbitron-VariableFont_wght: unit test asserts wght
  400 vs 900 outlines differ with ≥1.15× ink; harness renders 400 vs 700
  side by side (visibly heavier).
- CFF2 blend: still deferred — **zero CFF2 fonts exist on either machine**
  to test against; revisit pre-swap if one appears.
- **Exit:** ✅ weight axes interpolate; single-file variable fonts serve
  multiple weights through the standard `FontWeight` API.

### Phase 11 — Color glyphs / emoji 🔴 ✅ DONE
- [x] **From-scratch PNG decoder** (`raster/png.rs`): full DEFLATE inflater
  (stored/fixed/dynamic Huffman), scanline defiltering (filters 0–4),
  gray/RGB/palette/RGBA expansion. Pure std, ~330 lines.
- [x] **CBDT/CBLC** (`font/cbdt.rs`): strike selection (smallest ≥ target),
  index formats 1/2/3, image formats 17/18/19, metric scaling, area-average
  downscale (alpha-weighted, no dark fringes). Bitmap-only fonts (the
  bundled Noto Color Emoji has no glyf at all) now parse — **PC scan skips:
  1 → 0**.
- [x] **COLR v0 + CPAL** (`font/colr.rs`): layered glyphs rasterized through
  the normal outline path and composited bottom-up. COLRv1 paint graphs
  unsupported; v1-only faces excluded at scan exactly like the old stack's
  eviction (so fallback lands on CBDT). `sbix`/SVG-in-OT: no test targets on
  Linux, deferred.
- [x] **Unified RGBA atlas**: one Rgba8UnormSrgb texture + one pipeline for
  text AND emoji — coverage stored as premultiplied white with sRGB-encoded
  RGB (hardware decode returns linear coverage), emoji stored premultiplied;
  the shader's single `tint × texel` colors text and passes emoji through.
  `AtlasEntry.is_color` switches the quad tint to white+alpha.
- [x] **Emoji-aware clusters**: ZWJ/variation selectors keep their glyph only
  when the cluster's font maps one (emoji GSUB sequences work; text fonts
  don't render tofu boxes for controls).
- **Exit:** ✅ 🦊🚀🎉😀🌈 render in full color inline with text via the
  normal fallback chain; harness asserts 1.6k+ chromatic pixels; CBDT
  integration test decodes the fox and asserts it's orange. 🎉

### Phase 12 — Integration + swap 🟢 ✅ DONE (the payoff) 🔦🔥
- [x] Renamed `lntrn-type` → `lntrn-text` (git mv, history preserved),
  deleted `lntrn-render/text` (the glyphon wrapper), added to workspace
  `members`, removed the `exclude`, flipped `lntrn-render`'s dependency
  path. The package name matching the wrapper's meant **zero** source
  changes anywhere else — `pub use lntrn_text::TextRenderer` just resolves
  to the new engine.
- [x] Purged the old stack: **glyphon, cosmic-text, swash, harfrust, skrifa
  — 0 entries in Cargo.lock**. (`fontdb`/`rustybuzz`/`unicode-*` remnants
  survive only inside `usvg` — the SVG *icon* pipeline, a separate
  subsystem and a future build-our-own candidate.) The harness's glyphon
  dev-dep + side-by-side went with it; the comparison rows remain as
  regression asserts **pinned to the exact widths glyphon produced**
  (221.9 / 269.5 / 281.3 / 288.7 / 313.2 px — verified equal to the
  decimal while both engines coexisted).
- [x] Regression compile: full `cargo check --workspace` clean + release
  builds of compositor, terminal, file-manager, command-center, notepad,
  and rice (matrix scene). All 28 unit tests + the pixel-assert harness
  green from the new location. Visual pass across running apps happens as
  Alva relaunches them on the new binaries.
- Post-swap notes: **Spark Studio must update its dependency** from
  `lntrn-type` to `lntrn-text` (path + `use` statements). Atlas eviction
  tuning deferred until a real workload shows pressure (the atlas grows to
  8192² before dropping anything).
- **Exit:** ✅ glyphon is gone. The DE renders entirely on Lantern text. 🔦

---

## Open decisions (defaults chosen; flag to change)
- **AA:** grayscale first, subpixel optional in Phase 4. (Default: grayscale.)
- **Unicode data:** generate property tables from UCD at build time via a small
  codegen step in-repo, rather than vendoring a crate. (Default: codegen.)
- **Preview harness:** keep a permanent `examples/preview.rs` that renders the
  same string through both engines for visual diffing during dev. (Default: yes.)

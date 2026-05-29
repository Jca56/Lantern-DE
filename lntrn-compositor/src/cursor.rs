use std::collections::VecDeque;
use std::time::Instant;

use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            element::{
                memory::{MemoryRenderBuffer, MemoryRenderBufferRenderElement},
                Kind,
            },
            gles::GlesRenderer,
        },
    },
    input::pointer::{CursorIcon, CursorImageStatus},
    utils::{Logical, Physical, Point, Size, Transform},
};
use xcursor::{parser::parse_xcursor, CursorTheme};

pub struct CursorState {
    pub status: CursorImageStatus,
    buffer: MemoryRenderBuffer,
    hotspot: (i32, i32),
    size: (i32, i32),
    loaded: bool,
    cursor_size: u32,
    theme_name: String,
    loaded_icon_key: Option<&'static str>,
    /// "default", "custom1", "custom2" — from lantern.toml [input] cursor_theme
    custom_theme: String,
    custom_loaded: bool,
    last_recolor_scale: f32,
    last_recolor_radius: f32,
    /// Snapshot of the canonical recolor palette + outline scale at the
    /// last `tick_colors`. When any of these differ from what the
    /// settings file currently says we drop `loaded_icon_key` so the
    /// next `set_status` re-rasterizes with the new palette.
    last_recolor_body_light: String,
    last_recolor_body_dark: String,
    last_recolor_accent_light: String,
    last_recolor_accent_dark: String,
    last_recolor_outline: String,
    // Spin-to-grow: cursor grows when you spin the mouse in circles
    spin_history: VecDeque<(Point<f64, Logical>, Instant)>,
    spin_scale: f64,
    spin_last_active: Instant,
    pub click_anim: crate::cursor_click::ClickAnimState,
    pub loading_anim: crate::cursor_loading::LoadingAnimState,
}

impl CursorState {
    // ── Construction & status ──────────────────────────────────────────

    pub fn new(initial_theme: &str) -> Self {
        // XCURSOR_SIZE env var takes precedence (legacy override); otherwise read
        // from [input].cursor_size in lantern.toml, default 24.
        let cursor_size = std::env::var("XCURSOR_SIZE")
            .ok()
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or_else(|| {
                crate::input::read_input_setting_f64("cursor_size", 24.0).round() as u32
            })
            .clamp(16, 128);
        // xcursor theme name is separate from `initial_theme` — the latter
        // names a Lantern SVG (e.g. "Arch-Cursor") that only covers the
        // default arrow. Non-default shapes (pointer, text, resize) fall
        // through to xcursor and need a *real installed* theme like "default"
        // or whatever XCURSOR_THEME points to. Don't mix the two.
        let theme_name = std::env::var("XCURSOR_THEME")
            .unwrap_or_else(|_| "default".to_string());
        let mut state = Self {
            status: CursorImageStatus::Named(
                smithay::input::pointer::CursorIcon::Default,
            ),
            buffer: MemoryRenderBuffer::new(Fourcc::Argb8888, (1, 1), 1, Transform::Normal, None),
            hotspot: (0, 0),
            size: (0, 0),
            loaded: false,
            cursor_size,
            theme_name,
            loaded_icon_key: None,
            custom_theme: "default".into(),
            custom_loaded: false,
            last_recolor_body_light:   crate::input::read_input_setting("cursor_body_light",   "#ffffff"),
            last_recolor_body_dark:    crate::input::read_input_setting("cursor_body_dark",    "#ababab"),
            last_recolor_accent_light: crate::input::read_input_setting("cursor_accent_light", "#fab414"),
            last_recolor_accent_dark:  crate::input::read_input_setting("cursor_accent_dark",  "#9a6300"),
            last_recolor_outline:      crate::input::read_input_setting("cursor_outline_color", "#0a0a0a"),
            last_recolor_scale:        crate::input::read_input_setting_f64("cursor_outline_scale", 1.0) as f32,
            last_recolor_radius:       crate::input::read_input_setting_f64("cursor_corner_radius", 0.0) as f32,
            spin_history: VecDeque::with_capacity(32),
            spin_scale: 1.0,
            spin_last_active: Instant::now(),
            click_anim: crate::cursor_click::ClickAnimState::new(parse_hex_color(
                &crate::input::read_input_setting("cursor_body_light", "#ffffff"),
            )),
            loading_anim: crate::cursor_loading::LoadingAnimState::new(parse_hex_color(
                &crate::input::read_input_setting("cursor_body_light", "#ffffff"),
            )),
        };
        tracing::info!("CursorState::new with initial_theme='{}'", initial_theme);
        if initial_theme != "default" {
            state.set_custom_theme(initial_theme);
        }
        if !state.custom_loaded {
            // Run the full fallback chain (Lantern SVG → custom theme → embedded
            // → xcursor) instead of going straight to xcursor. The bare
            // load_xcursor path produced the Wayland blue/white triangle on
            // first frame until something forced a re-render via set_status.
            let status = state.status.clone();
            state.set_status(status);
        }
        state
    }

    pub fn set_status(&mut self, status: CursorImageStatus) {
        self.status = status;

        if let CursorImageStatus::Named(icon) = self.status {
            // Activate the spinner overlay for Wait / Progress states.
            // The base cursor still loads underneath (arrow for both, or
            // whatever the fallback gives us) so Progress reads as
            // "arrow + spinner" and Wait reads as "the cursor is busy".
            let want_spinner = matches!(icon, CursorIcon::Wait | CursorIcon::Progress);
            let color = parse_hex_color(
                &crate::input::read_input_setting("cursor_body_light", "#ffffff"),
            );
            self.loading_anim.set_active(want_spinner, color);

            let icon_key = cursor_icon_key(icon);
            if self.loaded_icon_key == Some(icon_key) {
                return;
            }
            // Fallback chain:
            //   1. Shape-specific Lantern SVG (resize variants get bespoke
            //      art; default arrow gets its dedicated SVG too).
            //   2. User's custom-theme SVG (e.g. `lntrn-cursor-purple.svg`)
            //      — used both for the default shape AND for any shape
            //      without a dedicated variant (pointer, text, grab…),
            //      so a custom-themed user keeps their cursor on links etc.
            //   3. Embedded Lantern default arrow (already covered by step 1
            //      for "default"; for other shapes when no custom theme is
            //      loaded, we re-call load_lantern_svg with "default").
            //   4. xcursor — only if every Lantern path fails.
            let hot = hotspot_for_icon(icon_key);
            if self.custom_loaded && icon_key == "default" {
                if self.load_custom_theme_svg(icon_key, hot) { return; }
            }
            if self.load_lantern_svg(icon_key) { return; }
            if self.custom_loaded && self.load_custom_theme_svg(icon_key, hot) { return; }
            if self.load_lantern_svg("default") {
                self.loaded_icon_key = Some(icon_key);
                return;
            }
            self.load_xcursor(icon);
        }
    }

    // ── Theme selection & SVG loading ──────────────────────────────────

    /// Rasterize the user's currently-active custom cursor theme SVG
    /// (from `~/.lantern/config/cursors/{custom_theme}.svg`) and tag it
    /// with `icon_key`. Used as the universal fallback for any shape
    /// when a custom theme is loaded.
    fn load_custom_theme_svg(
        &mut self,
        icon_key: &'static str,
        hotspot_viewbox: (f32, f32),
    ) -> bool {
        let svg_path = crate::lantern_home()
            .join("config/cursors")
            .join(format!("{}.svg", self.custom_theme));
        let Ok(data) = std::fs::read(&svg_path) else { return false };
        if self.rasterize_svg(&data, hotspot_viewbox).is_some() {
            self.loaded_icon_key = Some(icon_key);
            true
        } else {
            false
        }
    }

    /// Try to load a shape-SPECIFIC Lantern SVG cursor — only matches the
    /// default arrow and the four resize variants. Returns false for
    /// every other shape (pointer, text, grab…); callers are expected
    /// to then fall back to the user's custom theme via
    /// `load_custom_theme_svg`, so a custom-cursored user keeps their
    /// custom look on links, text fields, etc.
    ///
    /// Reads from `~/.lantern/icons/cursors/{file}.svg` first so tweaks
    /// land at runtime without rebuilding, then falls back to the
    /// embedded `lntrn_icons` bytes.
    fn load_lantern_svg(&mut self, icon_key: &'static str) -> bool {
        let svg_file = match icon_key {
            "default" => "lntrn-cursor.svg",
            "ew-resize" => "lntrn-cursor-ew.svg",
            "ns-resize" => "lntrn-cursor-ns.svg",
            "nesw-resize" => "lntrn-cursor-nesw.svg",
            "nwse-resize" => "lntrn-cursor-nwse.svg",
            _ => return false,
        };
        let hot = hotspot_for_icon(icon_key);

        // Runtime override: ~/.lantern/icons/cursors/<file>. Recolour is
        // applied to the bundled-default SVGs only — custom themes loaded
        // via `set_custom_theme` keep their authored palette.
        let runtime_path = crate::lantern_home()
            .join("icons/cursors")
            .join(svg_file);
        if let Ok(data) = std::fs::read(&runtime_path) {
            let recolored = recolor_cursor_svg(&data, icon_key);
            if self.rasterize_svg(&recolored, hot).is_some() {
                self.loaded_icon_key = Some(icon_key);
                tracing::info!("Loaded Lantern cursor: {}", runtime_path.display());
                return true;
            }
            tracing::warn!("Failed to rasterize Lantern cursor: {}", runtime_path.display());
        }

        // Embedded fallback.
        let Some(data) = lntrn_icons::get(svg_file) else { return false };
        let recolored = recolor_cursor_svg(data, icon_key);
        if self.rasterize_svg(&recolored, hot).is_some() {
            self.loaded_icon_key = Some(icon_key);
            tracing::info!("Loaded embedded Lantern cursor: {}", svg_file);
            true
        } else {
            false
        }
    }

    /// Set the custom cursor theme. If it's an SVG theme (custom1/custom2),
    /// load the SVG from ~/.lantern/config/cursors/{name}.svg.
    pub fn set_custom_theme(&mut self, theme: &str) {
        if theme == self.custom_theme && self.custom_loaded {
            return;
        }
        self.custom_theme = theme.to_string();

        if theme == "default" {
            // Revert to the Lantern default arrow (lntrn-cursor.svg) by
            // re-running the fallback chain with the current status,
            // instead of immediately calling load_xcursor which would
            // briefly paint the Wayland blue/white triangle until the
            // next pointer-shape change forced a re-rasterize.
            self.custom_loaded = false;
            self.loaded_icon_key = None;
            let status = self.status.clone();
            self.set_status(status);
            return;
        }

        // Try to load SVG from ~/.lantern/config/cursors/{theme}.svg
        let svg_path = crate::lantern_home()
            .join("config/cursors")
            .join(format!("{}.svg", theme));

        match std::fs::read(&svg_path) {
            Ok(data) => {
                if self.rasterize_svg(&data, hotspot_for_icon("default")).is_some() {
                    tracing::info!("Loaded custom SVG cursor: {}", svg_path.display());
                    self.custom_loaded = true;
                } else {
                    tracing::warn!("Failed to rasterize SVG cursor: {}", svg_path.display());
                    self.custom_loaded = false;
                }
            }
            Err(e) => {
                tracing::warn!("Failed to read SVG cursor {}: {}", svg_path.display(), e);
                self.custom_loaded = false;
            }
        }
    }

    // ── Size & color ───────────────────────────────────────────────────

    pub fn cursor_size(&self) -> u32 {
        self.cursor_size
    }

    /// Per-frame check: when any of the five recolor stops, the outline
    /// scale, or the corner radius change in lantern.toml, invalidate
    /// the cached cursor pixmap so the next render re-rasterizes with
    /// the new palette / geometry. Lets users tweak the Mouse panel and
    /// see the cursor update without a session restart.
    pub fn tick_colors(&mut self) {
        let body_light   = crate::input::read_input_setting("cursor_body_light",   "#ffffff");
        let body_dark    = crate::input::read_input_setting("cursor_body_dark",    "#ababab");
        let accent_light = crate::input::read_input_setting("cursor_accent_light", "#fab414");
        let accent_dark  = crate::input::read_input_setting("cursor_accent_dark",  "#9a6300");
        let outline      = crate::input::read_input_setting("cursor_outline_color", "#0a0a0a");
        let scale  = crate::input::read_input_setting_f64("cursor_outline_scale", 1.0) as f32;
        let radius = crate::input::read_input_setting_f64("cursor_corner_radius", 0.0) as f32;
        if body_light   == self.last_recolor_body_light
            && body_dark    == self.last_recolor_body_dark
            && accent_light == self.last_recolor_accent_light
            && accent_dark  == self.last_recolor_accent_dark
            && outline      == self.last_recolor_outline
            && (scale  - self.last_recolor_scale).abs()  < 0.001
            && (radius - self.last_recolor_radius).abs() < 0.001
        {
            return;
        }
        self.last_recolor_body_light   = body_light;
        self.last_recolor_body_dark    = body_dark;
        self.last_recolor_accent_light = accent_light;
        self.last_recolor_accent_dark  = accent_dark;
        self.last_recolor_outline      = outline;
        self.last_recolor_scale        = scale;
        self.last_recolor_radius       = radius;
        // Force the next set_status to rebuild the buffer. We snapshot status
        // first so a `Named(Default)` cursor immediately re-rasterizes.
        let cur = self.status.clone();
        self.loaded_icon_key = None;
        self.set_status(cur);
    }

    /// Update the cursor pixel size and force a reload of the current icon.
    pub fn set_cursor_size(&mut self, size: u32) {
        let size = size.clamp(16, 128);
        if size == self.cursor_size {
            return;
        }
        self.cursor_size = size;
        // Force the next set_status to re-rasterize at the new size.
        self.loaded_icon_key = None;
        self.custom_loaded = false;
        if self.custom_theme != "default" {
            let theme = self.custom_theme.clone();
            self.set_custom_theme(&theme);
        }
        if !self.custom_loaded {
            self.load_xcursor(CursorIcon::Default);
        }
    }

    // ── Rasterization & xcursor fallback ───────────────────────────────

    /// Rasterize an SVG into RGBA pixels and load into the cursor buffer.
    /// Returns Some(()) on success. The caller is responsible for any
    /// recoloring; this just renders the bytes it's given so custom (non-
    /// Lantern) cursor SVGs keep their authored palette.
    fn rasterize_svg(
        &mut self,
        svg_data: &[u8],
        hotspot_viewbox: (f32, f32),
    ) -> Option<()> {
        // Boxy SVG (and a few other editors) export with
        // `preserveAspectRatio="none"`, which stretches the path
        // non-uniformly when the viewBox aspect doesn't match the
        // width/height aspect — producing a skewed cursor even though
        // our downstream scale math is uniform. Strip it before parsing.
        let sanitized = strip_preserve_aspect_none(svg_data);
        let tree = resvg::usvg::Tree::from_data(&sanitized, &resvg::usvg::Options::default()).ok()?;
        let size = self.cursor_size.max(32);

        let tree_size = tree.size();
        let sx = size as f32 / tree_size.width();
        let sy = size as f32 / tree_size.height();
        let scale = sx.min(sy);
        let w = (tree_size.width() * scale).round() as u32;
        let h = (tree_size.height() * scale).round() as u32;

        let mut pixmap = resvg::tiny_skia::Pixmap::new(w, h)?;
        let transform = resvg::tiny_skia::Transform::from_scale(scale, scale);
        resvg::render(&tree, transform, &mut pixmap.as_mut());

        // Convert from premultiplied RGBA to straight RGBA (ABGR8888 for smithay)
        let pixels = pixmap.data();
        let mut rgba = vec![0u8; (w * h * 4) as usize];
        for i in 0..(w * h) as usize {
            let idx = i * 4;
            // resvg outputs premultiplied RGBA, smithay wants ABGR8888
            rgba[idx] = pixels[idx + 2]; // B
            rgba[idx + 1] = pixels[idx + 1]; // G
            rgba[idx + 2] = pixels[idx]; // R
            rgba[idx + 3] = pixels[idx + 3]; // A
        }

        // Hotspot is authored in the SVG's viewBox coordinate space
        // (e.g. (2, 2) for the arrow tip, (16, 16) for the symmetric
        // resize crosshair). Scale by the same factor used to rasterize
        // so the click point lands on the visible tip regardless of
        // cursor_size or DPI scaling.
        self.hotspot = (
            (hotspot_viewbox.0 * scale).round() as i32,
            (hotspot_viewbox.1 * scale).round() as i32,
        );
        self.size = (w as i32, h as i32);
        self.buffer = MemoryRenderBuffer::from_slice(
            &rgba,
            Fourcc::Argb8888,
            (w as i32, h as i32),
            1,
            Transform::Normal,
            None,
        );
        self.loaded = true;
        Some(())
    }

    fn load_xcursor(&mut self, icon: CursorIcon) {
        let icon_key = cursor_icon_key(icon);
        let theme = CursorTheme::load(&self.theme_name);
        let icon_path = cursor_icon_names(icon)
            .iter()
            .find_map(|name| theme.load_icon(name));

        tracing::info!(
            "Loading xcursor theme '{}' size {} icon {}",
            self.theme_name,
            self.cursor_size,
            icon_key
        );

        let path = match icon_path {
            Some(p) => p,
            None => {
                tracing::warn!("No xcursor icon found for {}, using fallback", icon_key);
                self.load_fallback(self.cursor_size);
                self.loaded_icon_key = Some(icon_key);
                return;
            }
        };

        let data = match std::fs::read(&path) {
            Ok(d) => d,
            Err(e) => {
                tracing::warn!("Failed to read xcursor file: {}", e);
                self.load_fallback(self.cursor_size);
                self.loaded_icon_key = Some(icon_key);
                return;
            }
        };

        let images = match parse_xcursor(&data) {
            Some(imgs) if !imgs.is_empty() => imgs,
            _ => {
                tracing::warn!("Failed to parse xcursor, using fallback");
                self.load_fallback(self.cursor_size);
                self.loaded_icon_key = Some(icon_key);
                return;
            }
        };

        // Pick the image closest to the requested size
        let image = images
            .iter()
            .min_by_key(|img| (img.size as i32 - self.cursor_size as i32).unsigned_abs())
            .unwrap();

        tracing::info!(
            "Loaded cursor: {}x{} hotspot ({}, {})",
            image.width, image.height, image.xhot, image.yhot
        );

        self.hotspot = (image.xhot as i32, image.yhot as i32);
        self.size = (image.width as i32, image.height as i32);

        self.buffer = MemoryRenderBuffer::from_slice(
            &image.pixels_rgba,
            Fourcc::Abgr8888,
            (image.width as i32, image.height as i32),
            1,
            Transform::Normal,
            None,
        );
        self.loaded = true;
        self.loaded_icon_key = Some(icon_key);
    }

    fn load_fallback(&mut self, size: u32) {
        let s = size.min(64) as i32;
        let mut pixels = vec![0u8; (s * s * 4) as usize];

        // Simple white arrow cursor with black border
        for y in 0..s {
            let row_width = (y * 2 / 3).min(s - 1);
            for x in 0..=row_width {
                let idx = ((y * s + x) * 4) as usize;
                let is_border = x == 0 || x == row_width || y == 0 || y == s - 1
                    || (y > s * 2 / 3 && x < y - s * 2 / 3 + 2);
                if is_border {
                    // Black border: ARGB
                    pixels[idx] = 0xFF; // B
                    pixels[idx + 1] = 0x00; // G
                    pixels[idx + 2] = 0x00; // R
                    pixels[idx + 3] = 0xFF; // A
                } else {
                    // White fill: ARGB
                    pixels[idx] = 0xFF; // B
                    pixels[idx + 1] = 0xFF; // G
                    pixels[idx + 2] = 0xFF; // R
                    pixels[idx + 3] = 0xFF; // A
                }
            }
        }

        self.hotspot = (0, 0);
        self.size = (s, s);
        self.buffer = MemoryRenderBuffer::from_slice(
            &pixels,
            Fourcc::Argb8888,
            (s, s),
            1,
            Transform::Normal,
            None,
        );
        self.loaded = true;
    }

    // ── Spin-to-grow cursor ────────────────────────────────────────────

    /// Feed a new pointer position. Detects circular motion and grows the cursor.
    pub fn update_spin(&mut self, pos: Point<f64, Logical>) {
        let now = Instant::now();
        self.spin_history.push_back((pos, now));

        // Keep only the last 30 samples
        while self.spin_history.len() > 30 {
            self.spin_history.pop_front();
        }

        if self.spin_history.len() < 8 {
            return;
        }

        // Centroid of recent positions
        let n = self.spin_history.len() as f64;
        let cx: f64 = self.spin_history.iter().map(|(p, _)| p.x).sum::<f64>() / n;
        let cy: f64 = self.spin_history.iter().map(|(p, _)| p.y).sum::<f64>() / n;

        // Sum the signed angle swept around the centroid
        let mut total_angle = 0.0f64;
        let pts: Vec<_> = self.spin_history.iter().collect();
        for w in pts.windows(2) {
            let (p1, _) = w[0];
            let (p2, _) = w[1];
            let v1x = p1.x - cx;
            let v1y = p1.y - cy;
            let v2x = p2.x - cx;
            let v2y = p2.y - cy;
            let cross = v1x * v2y - v1y * v2x;
            let dot = v1x * v2x + v1y * v2y;
            total_angle += cross.atan2(dot);
        }

        let time_span = self.spin_history.back().unwrap().1
            .duration_since(self.spin_history.front().unwrap().1)
            .as_secs_f64();

        if time_span > 0.05 {
            let angular_vel = total_angle.abs() / time_span;
            // ~0.7 revolutions per second threshold
            if angular_vel > 4.5 {
                self.spin_scale += 0.04;
                self.spin_scale = self.spin_scale.min(8.0);
                self.spin_last_active = now;
            }
        }
    }

    /// Decay the spin scale back toward 1.0 when the user stops spinning.
    /// Call once per render frame. Returns true if a redraw is needed.
    pub fn tick_spin_decay(&mut self) -> bool {
        if self.spin_scale <= 1.0 {
            return false;
        }
        let elapsed = self.spin_last_active.elapsed().as_secs_f64();
        if elapsed > 0.3 {
            self.spin_scale = (self.spin_scale - 0.12).max(1.0);
            return true;
        }
        false
    }

    // ── Rendering ────────────────────────────────────────────────────

    pub fn render_element(
        &self,
        renderer: &mut GlesRenderer,
        position: Point<f64, Physical>,
    ) -> Option<MemoryRenderBufferRenderElement<GlesRenderer>> {
        if !self.loaded {
            return None;
        }

        if let CursorImageStatus::Hidden = self.status {
            return None;
        }

        if let CursorImageStatus::Surface(_) = self.status {
            return None;
        }

        let scale = self.spin_scale;
        let cursor_pos = (
            position.x - self.hotspot.0 as f64 * scale,
            position.y - self.hotspot.1 as f64 * scale,
        );

        let dst = if scale > 1.001 {
            Some(Size::from((
                (self.size.0 as f64 * scale) as i32,
                (self.size.1 as f64 * scale) as i32,
            )))
        } else {
            None
        };

        MemoryRenderBufferRenderElement::from_buffer(
            renderer,
            cursor_pos,
            &self.buffer,
            None,
            None,
            dst,
            Kind::Cursor,
        )
        .ok()
    }
}

/// Hotspot (in viewBox-space coords) for each Lantern cursor SVG.
/// All Lantern cursor SVGs use a `0 0 32 32` viewBox; coordinates are
/// matched to the visible click point of each shape:
///
/// - Resize cursors — geometric center of the symmetric double-arrow
///   cross at `(16, 16)`.
/// - Arrow (`default`, `pointer`, `text`, etc.) — the bundled gradient
///   arrow has its path tip at viewBox `(2.436, -0.16)` inside a
///   `-3.8 -3.3 32 32` viewBox, which lands at natural-pixel
///   `(6.24, 3.14)` from the top-left of the rasterized image.
///   `paint-order: stroke` plus the round linecap extend the *visible*
///   tip outward by half the (scaled) stroke along the NW diagonal,
///   so we inset by that amount to keep the click point on the
///   perceived tip rather than the unstroked path-start.
fn hotspot_for_icon(icon_key: &str) -> (f32, f32) {
    match icon_key {
        "ew-resize" | "ns-resize" | "nesw-resize" | "nwse-resize" => (16.0, 16.0),
        _ => {
            // Authored stroke is 6px; user setting is a multiplier (1.0 = no change).
            let scale = crate::input::read_input_setting_f64("cursor_outline_scale", 1.0) as f32;
            let effective_stroke = 6.0 * scale.max(0.0);
            let inset = (effective_stroke * 0.5) / std::f32::consts::SQRT_2;
            let tip_x = (6.236 - inset).max(0.0);
            let tip_y = (3.14  - inset).max(0.0);
            (tip_x, tip_y)
        }
    }
}

pub(crate) fn parse_hex_color(s: &str) -> (u8, u8, u8) {
    let h = s.trim_start_matches('#');
    if h.len() != 6 {
        return (255, 255, 255);
    }
    let r = u8::from_str_radix(&h[0..2], 16).unwrap_or(255);
    let g = u8::from_str_radix(&h[2..4], 16).unwrap_or(255);
    let b = u8::from_str_radix(&h[4..6], 16).unwrap_or(255);
    (r, g, b)
}

fn cursor_icon_key(icon: CursorIcon) -> &'static str {
    match icon {
        CursorIcon::Move => "move",
        CursorIcon::Grab => "grab",
        CursorIcon::Grabbing => "grabbing",
        CursorIcon::EResize | CursorIcon::WResize | CursorIcon::EwResize | CursorIcon::ColResize => "ew-resize",
        CursorIcon::NResize | CursorIcon::SResize | CursorIcon::NsResize | CursorIcon::RowResize => "ns-resize",
        CursorIcon::NeResize | CursorIcon::SwResize | CursorIcon::NeswResize => "nesw-resize",
        CursorIcon::NwResize | CursorIcon::SeResize | CursorIcon::NwseResize => "nwse-resize",
        CursorIcon::Text | CursorIcon::VerticalText => "text",
        CursorIcon::Pointer => "pointer",
        CursorIcon::NotAllowed | CursorIcon::NoDrop => "not-allowed",
        CursorIcon::AllResize | CursorIcon::AllScroll => "all-scroll",
        _ => "default",
    }
}

fn cursor_icon_names(icon: CursorIcon) -> &'static [&'static str] {
    match icon {
        CursorIcon::Move => &["move", "fleur", "size_all"],
        CursorIcon::Grab => &["grab", "openhand", "hand1"],
        CursorIcon::Grabbing => &["grabbing", "closedhand", "hand2"],
        CursorIcon::EResize | CursorIcon::WResize | CursorIcon::EwResize | CursorIcon::ColResize => {
            &["ew-resize", "sb_h_double_arrow", "size_hor", "left_side", "right_side"]
        }
        CursorIcon::NResize | CursorIcon::SResize | CursorIcon::NsResize | CursorIcon::RowResize => {
            &["ns-resize", "sb_v_double_arrow", "size_ver", "top_side", "bottom_side"]
        }
        CursorIcon::NeResize | CursorIcon::SwResize | CursorIcon::NeswResize => {
            &["nesw-resize", "size_bdiag", "top_right_corner", "bottom_left_corner"]
        }
        CursorIcon::NwResize | CursorIcon::SeResize | CursorIcon::NwseResize => {
            &["nwse-resize", "size_fdiag", "top_left_corner", "bottom_right_corner"]
        }
        CursorIcon::Text | CursorIcon::VerticalText => &["text", "xterm", "ibeam"],
        CursorIcon::Pointer => &["pointer", "hand2", "hand1"],
        CursorIcon::NotAllowed | CursorIcon::NoDrop => &["not-allowed", "crossed_circle", "forbidden"],
        CursorIcon::AllResize | CursorIcon::AllScroll => &["all-scroll", "fleur", "size_all"],
        _ => &["left_ptr", "default", "arrow"],
    }
}

/// Five-stop cursor recolor. The bundled Lantern cursors are authored
/// with a canonical palette so this function can find-and-replace
/// each role independently:
///
///   `#ffffff` → body-light gradient stop (the bright top of the arrow)
///   `#ababab` → body-dark gradient stop (the shaded bottom)
///   `#fab414` → accent-light (gold/highlight top)
///   `#9a6300` → accent-dark (gold/highlight bottom)
///   `#0a0a0a` → outline stroke
///
/// `cursor_outline_scale` (default 1.0) multiplies every authored
/// `stroke-width` proportionally so a user can fatten or thin the
/// outline without flattening the multi-element cursors to a single
/// stroke value. Corner-roundness applies to the first polygon path
/// only (the default arrow body); the resize variants keep their
/// authored geometry.
///
/// Resize cursors (`*-resize` icon_keys) skip body/accent recolor.
/// They share the canonical palette codes with the arrow, but applying
/// the user's arrow-tuned palette to them (e.g. body_dark = same as
/// outline) made the resize cursor body fade into its own stroke. The
/// outline color + scale + radius still apply so the resize frame
/// stays consistent with the user's outline choice.
///
/// Returns the original bytes verbatim when every knob is at its
/// default so custom theme SVGs round-trip unchanged.
pub(crate) fn recolor_cursor_svg(svg_data: &[u8], icon_key: &str) -> Vec<u8> {
    let is_resize = matches!(
        icon_key,
        "ew-resize" | "ns-resize" | "nesw-resize" | "nwse-resize"
    );
    let body_light = crate::input::read_input_setting("cursor_body_light", "#ffffff");
    let body_dark = crate::input::read_input_setting("cursor_body_dark", "#ababab");
    let accent_light = crate::input::read_input_setting("cursor_accent_light", "#fab414");
    let accent_dark = crate::input::read_input_setting("cursor_accent_dark", "#9a6300");
    let outline_color = crate::input::read_input_setting("cursor_outline_color", "#0a0a0a");
    let outline_scale = crate::input::read_input_setting_f64("cursor_outline_scale", 1.0) as f32;
    let radius = crate::input::read_input_setting_f64("cursor_corner_radius", 0.0) as f32;

    let body_light_default   = body_light.eq_ignore_ascii_case("#ffffff");
    let body_dark_default    = body_dark.eq_ignore_ascii_case("#ababab");
    let accent_light_default = accent_light.eq_ignore_ascii_case("#fab414");
    let accent_dark_default  = accent_dark.eq_ignore_ascii_case("#9a6300");
    let outline_default      = outline_color.eq_ignore_ascii_case("#0a0a0a");
    let scale_default        = (outline_scale - 1.0).abs() < 0.001;
    let radius_default       = radius.abs() < 0.001;

    // Outline + scale + radius apply to every Lantern cursor universally.
    // The body/accent palette only applies to the default arrow — for the
    // resize cursors we keep the canonical white→gray gradient so they
    // stay readable no matter how aggressively the user has retinted
    // the arrow.
    let body_skipped = is_resize || (body_light_default && body_dark_default
        && accent_light_default && accent_dark_default);

    if body_skipped && outline_default && scale_default && radius_default {
        return svg_data.to_vec();
    }

    let Ok(s) = std::str::from_utf8(svg_data) else {
        return svg_data.to_vec();
    };
    let mut out = s.to_string();

    if !is_resize {
        if !body_light_default   { out = replace_hex_case_insensitive(&out, "#ffffff", &body_light); }
        if !body_dark_default    { out = replace_hex_case_insensitive(&out, "#ababab", &body_dark); }
        if !accent_light_default { out = replace_hex_case_insensitive(&out, "#fab414", &accent_light); }
        if !accent_dark_default  { out = replace_hex_case_insensitive(&out, "#9a6300", &accent_dark); }
    }
    if !outline_default { out = replace_hex_case_insensitive(&out, "#0a0a0a", &outline_color); }
    if !scale_default   { out = scale_stroke_widths(&out, outline_scale.max(0.0)); }
    if !radius_default  { out = round_first_polygon_path(&out, radius); }

    out.into_bytes()
}

fn replace_hex_case_insensitive(s: &str, from_lc: &str, to: &str) -> String {
    debug_assert!(from_lc.chars().all(|c| !c.is_ascii_uppercase()));
    let mut out = s.replace(from_lc, to);
    let from_uc = from_lc.to_uppercase();
    if from_uc != from_lc {
        out = out.replace(&from_uc, to);
    }
    out
}

/// Multiply every `stroke-width="N"` attribute *and* any
/// `stroke-width: Npx` / `stroke-width: N` rule inside `style="..."`
/// blocks by `scale`. Multi-element cursors (e.g. the resize variants
/// with 1px detail strokes alongside thicker frame strokes) stay
/// proportional this way; the old absolute-replace behaviour
/// flattened every stroke to one value, which turned high outline
/// settings into solid black blobs that swallowed the gradient.
fn scale_stroke_widths(svg: &str, scale: f32) -> String {
    let scale = scale.max(0.0);
    let scale_value = |v: f32| (v * scale).max(0.0);

    let mut out = String::with_capacity(svg.len());
    let mut rest = svg;
    let attr = "stroke-width=\"";
    while let Some(idx) = rest.find(attr) {
        out.push_str(&rest[..idx]);
        out.push_str(attr);
        let after = &rest[idx + attr.len()..];
        match after.find('"') {
            Some(end) => {
                let raw = after[..end].trim();
                let parsed = raw.parse::<f32>().unwrap_or(0.0);
                out.push_str(&format!("{:.2}", scale_value(parsed)));
                rest = &after[end..];
            }
            None => {
                out.push_str(after);
                return out;
            }
        }
    }
    out.push_str(rest);

    // CSS-form occurrences inside style attributes.
    let mut final_out = String::with_capacity(out.len());
    let mut rest = out.as_str();
    let css = "stroke-width:";
    while let Some(idx) = rest.find(css) {
        final_out.push_str(&rest[..idx]);
        final_out.push_str(css);
        let after = &rest[idx + css.len()..];
        let trimmed = after.trim_start_matches(' ');
        let leading_space = after.len() - trimmed.len();
        let term = trimmed.find(|c: char| c == ';' || c == '"' || c == '}' || c == '\n');
        let term_at = term.unwrap_or(trimmed.len());
        let value = &trimmed[..term_at];
        let unit_start = value
            .find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-' || c == '+'))
            .unwrap_or(value.len());
        let num = &value[..unit_start];
        let unit = &value[unit_start..];
        let parsed = num.trim().parse::<f32>().unwrap_or(0.0);
        for _ in 0..leading_space { final_out.push(' '); }
        final_out.push_str(&format!("{:.2}{}", scale_value(parsed), unit));
        rest = &trimmed[term_at..];
    }
    final_out.push_str(rest);
    final_out
}

/// Find the first `d="M ... Z"` polygon path in the SVG and rebuild it
/// with rounded corners. Roundness is `r` (0..=1) scaled to a fraction of
/// each corner's adjacent segment lengths so the effect is consistent
/// regardless of the path's coordinate range. Non-polygon paths (curves,
/// arcs) are left alone.
fn round_first_polygon_path(svg: &str, r: f32) -> String {
    let attr = "d=\"";
    let Some(start) = svg.find(attr) else { return svg.to_string() };
    let body_start = start + attr.len();
    let Some(end_off) = svg[body_start..].find('"') else { return svg.to_string() };
    let d = &svg[body_start..body_start + end_off];
    let Some(pts) = parse_polygon_d(d) else { return svg.to_string() };
    let new_d = round_polygon_path(&pts, r);
    let mut out = String::with_capacity(svg.len() + 64);
    out.push_str(&svg[..body_start]);
    out.push_str(&new_d);
    out.push_str(&svg[body_start + end_off..]);
    out
}

/// Parse a polygon-style `d` attribute (only M / L / Z commands). Returns
/// `None` for paths containing curves or arcs so we don't mangle them.
fn parse_polygon_d(d: &str) -> Option<Vec<(f32, f32)>> {
    let mut pts: Vec<(f32, f32)> = Vec::new();
    let mut tokens = d.split(|c: char| c.is_whitespace() || c == ',').filter(|t| !t.is_empty());
    while let Some(tok) = tokens.next() {
        match tok {
            "M" | "L" | "m" | "l" => {
                let x: f32 = tokens.next()?.parse().ok()?;
                let y: f32 = tokens.next()?.parse().ok()?;
                pts.push((x, y));
            }
            "Z" | "z" => break,
            _ => return None,
        }
    }
    // Drop trailing duplicate of first point (manual close before Z).
    if pts.len() > 1 {
        let last = pts[pts.len() - 1];
        let first = pts[0];
        if (last.0 - first.0).abs() < 0.001 && (last.1 - first.1).abs() < 0.001 {
            pts.pop();
        }
    }
    if pts.len() < 3 { return None; }
    Some(pts)
}

/// Rebuild a polygon path with each corner replaced by a quadratic Bezier
/// of radius proportional to the shorter adjacent segment.
fn round_polygon_path(pts: &[(f32, f32)], r: f32) -> String {
    let r = r.clamp(0.0, 1.0);
    if r < 0.001 {
        let mut s = String::new();
        for (i, p) in pts.iter().enumerate() {
            let cmd = if i == 0 { "M" } else { "L" };
            s.push_str(&format!("{} {:.3} {:.3} ", cmd, p.0, p.1));
        }
        s.push('Z');
        return s;
    }
    let n = pts.len();
    let mut s = String::new();
    for i in 0..n {
        let prev = pts[(i + n - 1) % n];
        let cur = pts[i];
        let next = pts[(i + 1) % n];
        let (dx1, dy1) = (prev.0 - cur.0, prev.1 - cur.1);
        let l1 = (dx1 * dx1 + dy1 * dy1).sqrt().max(0.001);
        let (dx2, dy2) = (next.0 - cur.0, next.1 - cur.1);
        let l2 = (dx2 * dx2 + dy2 * dy2).sqrt().max(0.001);
        let r_eff = r * 0.45 * l1.min(l2);
        let p_in = (cur.0 + dx1 / l1 * r_eff, cur.1 + dy1 / l1 * r_eff);
        let p_out = (cur.0 + dx2 / l2 * r_eff, cur.1 + dy2 / l2 * r_eff);
        let cmd = if i == 0 { "M" } else { "L" };
        s.push_str(&format!("{} {:.3} {:.3} ", cmd, p_in.0, p_in.1));
        s.push_str(&format!("Q {:.3} {:.3} {:.3} {:.3} ", cur.0, cur.1, p_out.0, p_out.1));
    }
    s.push('Z');
    s
}

/// Remove `preserveAspectRatio="none"` (and single-quote / spaced variants) from
/// an SVG byte stream. When this attribute is present and the SVG's viewBox
/// aspect ratio differs from its width/height aspect, the path is stretched
/// non-uniformly at parse time, distorting cursor shapes. Pass-through if the
/// bytes aren't UTF-8 or the attribute isn't present.
pub(crate) fn strip_preserve_aspect_none(svg_data: &[u8]) -> Vec<u8> {
    let Ok(s) = std::str::from_utf8(svg_data) else {
        return svg_data.to_vec();
    };
    if !s.contains("preserveAspectRatio") {
        return svg_data.to_vec();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(idx) = rest.find("preserveAspectRatio") {
        let (before, after) = rest.split_at(idx);
        out.push_str(before);
        // Skip past the attribute name
        let after = &after["preserveAspectRatio".len()..];
        // Skip optional whitespace and `=`
        let trimmed = after.trim_start();
        if let Some(eq_rest) = trimmed.strip_prefix('=') {
            let val_start = eq_rest.trim_start();
            let (quote, body) = match val_start.chars().next() {
                Some('"') => ('"', &val_start[1..]),
                Some('\'') => ('\'', &val_start[1..]),
                _ => {
                    // No quoted value — emit attribute name back, keep going.
                    out.push_str("preserveAspectRatio");
                    rest = after;
                    continue;
                }
            };
            if let Some(end) = body.find(quote) {
                let value = body[..end].trim();
                if value.eq_ignore_ascii_case("none") {
                    // Drop the whole attribute. Eat any single leading space
                    // we just emitted to keep the tag tidy.
                    if out.ends_with(' ') {
                        out.pop();
                    }
                    rest = &body[end + 1..];
                } else {
                    // Preserve other valid values (e.g. xMidYMid meet).
                    out.push_str("preserveAspectRatio=");
                    out.push(quote);
                    out.push_str(&body[..end]);
                    out.push(quote);
                    rest = &body[end + 1..];
                }
            } else {
                // Unterminated quote — bail and return original.
                return svg_data.to_vec();
            }
        } else {
            out.push_str("preserveAspectRatio");
            rest = after;
        }
    }
    out.push_str(rest);
    out.into_bytes()
}


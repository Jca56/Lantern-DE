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
    utils::{Physical, Point, Transform},
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
}

impl CursorState {
    pub fn new(initial_theme: &str) -> Self {
        let cursor_size = std::env::var("XCURSOR_SIZE")
            .ok()
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(24);
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
        };
        tracing::info!("CursorState::new with initial_theme='{}'", initial_theme);
        if initial_theme != "default" {
            state.set_custom_theme(initial_theme);
        }
        if !state.custom_loaded {
            state.load_xcursor(CursorIcon::Default);
        }
        state
    }

    pub fn set_status(&mut self, status: CursorImageStatus) {
        self.status = status;

        if let CursorImageStatus::Named(icon) = self.status {
            let icon_key = cursor_icon_key(icon);
            if self.loaded_icon_key != Some(icon_key) {
                // Try Lantern SVG cursor first, then fall back to xcursor
                if !self.load_lantern_svg(icon_key) {
                    self.load_xcursor(icon);
                }
            }
        }
    }

    /// Try to load a Lantern SVG cursor from ~/.lantern/icons/cursors/
    /// Returns true if successfully loaded.
    fn load_lantern_svg(&mut self, icon_key: &'static str) -> bool {
        let svg_file = match icon_key {
            "default" => "lntrn-cursor.svg",
            "ew-resize" => "lntrn-cursor-ew.svg",
            "ns-resize" => "lntrn-cursor-ns.svg",
            "nesw-resize" => "lntrn-cursor-nesw.svg",
            "nwse-resize" => "lntrn-cursor-nwse.svg",
            _ => return false,
        };
        let data = match lntrn_icons::get(svg_file) {
            Some(d) => d,
            None => return false,
        };
        if self.rasterize_svg(data).is_some() {
            self.loaded_icon_key = Some(icon_key);
            self.custom_loaded = false;
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
            // Revert to xcursor
            self.custom_loaded = false;
            self.loaded_icon_key = None; // Force reload
            self.load_xcursor(CursorIcon::Default);
            return;
        }

        // Try to load SVG from ~/.lantern/config/cursors/{theme}.svg
        let svg_path = crate::lantern_home()
            .join("config/cursors")
            .join(format!("{}.svg", theme));

        match std::fs::read(&svg_path) {
            Ok(data) => {
                if self.rasterize_svg(&data).is_some() {
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

    /// Rasterize an SVG into RGBA pixels and load into the cursor buffer.
    /// Returns Some(()) on success.
    fn rasterize_svg(&mut self, svg_data: &[u8]) -> Option<()> {
        let tree = resvg::usvg::Tree::from_data(svg_data, &resvg::usvg::Options::default()).ok()?;
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

        self.hotspot = (0, 0);
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

        let cursor_pos = (
            position.x - self.hotspot.0 as f64,
            position.y - self.hotspot.1 as f64,
        );

        MemoryRenderBufferRenderElement::from_buffer(
            renderer,
            cursor_pos,
            &self.buffer,
            None,
            None,
            None,
            Kind::Cursor,
        )
        .ok()
    }
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

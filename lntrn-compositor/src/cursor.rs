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
    input::pointer::CursorImageStatus,
    utils::{Physical, Point, Transform},
};
use xcursor::{parser::parse_xcursor, CursorTheme};

pub struct CursorState {
    pub status: CursorImageStatus,
    buffer: MemoryRenderBuffer,
    hotspot: (i32, i32),
    size: (i32, i32),
    loaded: bool,
}

impl CursorState {
    pub fn new() -> Self {
        let mut state = Self {
            status: CursorImageStatus::Named(
                smithay::input::pointer::CursorIcon::Default,
            ),
            buffer: MemoryRenderBuffer::new(Fourcc::Argb8888, (1, 1), 1, Transform::Normal, None),
            hotspot: (0, 0),
            size: (0, 0),
            loaded: false,
        };
        state.load_xcursor();
        state
    }

    fn load_xcursor(&mut self) {
        let cursor_size = std::env::var("XCURSOR_SIZE")
            .ok()
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(24);

        let theme_name = std::env::var("XCURSOR_THEME")
            .unwrap_or_else(|_| "default".to_string());

        tracing::info!("Loading xcursor theme '{}' size {}", theme_name, cursor_size);

        let theme = CursorTheme::load(&theme_name);
        let icon_path = theme.load_icon("left_ptr")
            .or_else(|| theme.load_icon("default"))
            .or_else(|| theme.load_icon("arrow"));

        let path = match icon_path {
            Some(p) => p,
            None => {
                tracing::warn!("No xcursor theme found, using fallback");
                self.load_fallback(cursor_size);
                return;
            }
        };

        let data = match std::fs::read(&path) {
            Ok(d) => d,
            Err(e) => {
                tracing::warn!("Failed to read xcursor file: {}", e);
                self.load_fallback(cursor_size);
                return;
            }
        };

        let images = match parse_xcursor(&data) {
            Some(imgs) if !imgs.is_empty() => imgs,
            _ => {
                tracing::warn!("Failed to parse xcursor, using fallback");
                self.load_fallback(cursor_size);
                return;
            }
        };

        // Pick the image closest to the requested size
        let image = images
            .iter()
            .min_by_key(|img| (img.size as i32 - cursor_size as i32).unsigned_abs())
            .unwrap();

        tracing::info!(
            "Loaded cursor: {}x{} hotspot ({}, {})",
            image.width, image.height, image.xhot, image.yhot
        );

        self.hotspot = (image.xhot as i32, image.yhot as i32);
        self.size = (image.width as i32, image.height as i32);

        self.buffer = MemoryRenderBuffer::from_slice(
            &image.pixels_argb,
            Fourcc::Argb8888,
            (image.width as i32, image.height as i32),
            1,
            Transform::Normal,
            None,
        );
        self.loaded = true;
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

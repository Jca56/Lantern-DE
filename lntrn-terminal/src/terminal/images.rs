//! Kitty graphics protocol support.
//!
//! Handles APC-based image transmission: ESC _ G <key>=<value>;... ; <base64 data> ESC \
//! Images are stored as RGBA pixel data and placed at grid cell positions.

use std::collections::HashMap;

/// A decoded image ready for GPU upload.
pub struct TerminalImage {
    pub rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
    /// Grid position where the image is placed (row, col).
    pub row: usize,
    pub col: usize,
    /// Display size in grid cells.
    pub cols_wide: usize,
    pub rows_tall: usize,
    /// Unique image ID (from Kitty protocol or auto-assigned).
    pub image_id: u32,
    /// Z-index for layering (negative = behind text).
    #[allow(dead_code)]
    pub z_index: i32,
    /// Monotonically incrementing version for this image ID. The renderer
    /// uses this to detect when cached GPU textures need to be re-uploaded
    /// (e.g. when a TUI app re-transmits an image with the same ID but
    /// different content).
    pub version: u64,
}

/// Manages in-flight image transmissions and completed images.
pub struct ImageManager {
    /// Images that have been fully received and decoded.
    pub images: Vec<TerminalImage>,
    /// In-flight transmission: accumulating base64 chunks.
    transmissions: HashMap<u32, Transmission>,
    /// Next auto-assigned image ID.
    next_id: u32,
    /// Monotonic version counter — bumped each time any image is transmitted
    /// or replaced. The renderer compares this against its cached versions
    /// to detect when GPU textures must be re-uploaded.
    next_version: u64,
}

#[allow(dead_code)]
struct Transmission {
    image_id: u32,
    format: ImageFormat,
    width: u32,
    height: u32,
    data: Vec<u8>, // accumulated base64 decoded bytes
    /// Placement info
    row: usize,
    col: usize,
    cols_wide: usize,
    rows_tall: usize,
    z_index: i32,
}

#[derive(Clone, Copy)]
enum ImageFormat {
    Rgba,
    Rgb,
    Png,
}

impl ImageManager {
    pub fn new() -> Self {
        Self {
            images: Vec::new(),
            transmissions: HashMap::new(),
            next_id: 1,
            next_version: 1,
        }
    }

    /// Process a Kitty graphics APC payload (everything between ESC_G and ST).
    /// The payload format is: key=value,key=value,...;base64data
    pub fn process_kitty(&mut self, payload: &[u8], cursor_row: usize, cursor_col: usize) {
        let payload_str = match std::str::from_utf8(payload) {
            Ok(s) => s,
            Err(_) => return,
        };

        // Split at first semicolon: control data ; payload data
        let (control, b64_data) = match payload_str.find(';') {
            Some(i) => (&payload_str[..i], &payload_str[i + 1..]),
            None => (payload_str, ""),
        };

        // Parse key=value pairs
        let mut action = 't'; // transmit
        let mut format = ImageFormat::Rgba;
        let mut width: u32 = 0;
        let mut height: u32 = 0;
        let mut image_id: u32 = 0;
        let mut more_chunks = false;
        let mut cols_wide: usize = 0;
        let mut rows_tall: usize = 0;
        let mut z_index: i32 = 0;

        for kv in control.split(',') {
            if let Some((k, v)) = kv.split_once('=') {
                match k {
                    "a" => {
                        if let Some(c) = v.chars().next() {
                            action = c;
                        }
                    }
                    "f" => {
                        format = match v {
                            "24" => ImageFormat::Rgb,
                            "32" => ImageFormat::Rgba,
                            "100" => ImageFormat::Png,
                            _ => ImageFormat::Rgba,
                        };
                    }
                    "s" => width = v.parse().unwrap_or(0),
                    "v" => height = v.parse().unwrap_or(0),
                    "i" => image_id = v.parse().unwrap_or(0),
                    "m" => more_chunks = v == "1",
                    "c" => cols_wide = v.parse().unwrap_or(0),
                    "r" => rows_tall = v.parse().unwrap_or(0),
                    "z" => z_index = v.parse().unwrap_or(0),
                    _ => {}
                }
            }
        }

        // Auto-assign image ID if not provided
        if image_id == 0 {
            image_id = self.next_id;
            self.next_id += 1;
        }

        // Decode base64 payload
        let decoded = if !b64_data.is_empty() {
            decode_base64(b64_data)
        } else {
            Vec::new()
        };

        match action {
            't' | 'T' => {
                // Transmit (and optionally display)
                if more_chunks {
                    // Multi-chunk: accumulate
                    let tx = self.transmissions.entry(image_id).or_insert_with(|| {
                        Transmission {
                            image_id,
                            format,
                            width,
                            height,
                            data: Vec::new(),
                            row: cursor_row,
                            col: cursor_col,
                            cols_wide,
                            rows_tall,
                            z_index,
                        }
                    });
                    tx.data.extend_from_slice(&decoded);
                } else {
                    // Final or single chunk
                    let full_data = if let Some(mut tx) = self.transmissions.remove(&image_id) {
                        tx.data.extend_from_slice(&decoded);
                        // Use stored metadata from first chunk
                        if width == 0 { width = tx.width; }
                        if height == 0 { height = tx.height; }
                        if cols_wide == 0 { cols_wide = tx.cols_wide; }
                        if rows_tall == 0 { rows_tall = tx.rows_tall; }
                        format = tx.format;
                        z_index = tx.z_index;
                        tx.data
                    } else {
                        decoded
                    };

                    // Convert to RGBA
                    let rgba = match format {
                        ImageFormat::Rgba => full_data,
                        ImageFormat::Rgb => {
                            let pixel_count = full_data.len() / 3;
                            let mut rgba = Vec::with_capacity(pixel_count * 4);
                            for chunk in full_data.chunks_exact(3) {
                                rgba.extend_from_slice(chunk);
                                rgba.push(255);
                            }
                            rgba
                        }
                        ImageFormat::Png => {
                            match decode_png(&full_data) {
                                Some((w, h, data)) => {
                                    width = w;
                                    height = h;
                                    data
                                }
                                None => return,
                            }
                        }
                    };

                    if width == 0 || height == 0 {
                        return;
                    }

                    let version = self.next_version;
                    self.next_version += 1;
                    let new_image = TerminalImage {
                        rgba,
                        width,
                        height,
                        row: cursor_row,
                        col: cursor_col,
                        cols_wide: if cols_wide == 0 { 1 } else { cols_wide },
                        rows_tall: if rows_tall == 0 { 1 } else { rows_tall },
                        image_id,
                        z_index,
                        version,
                    };

                    // Replace any existing image with the same ID, otherwise
                    // append. This prevents duplicate images stacking up when
                    // a TUI re-transmits at the same ID (e.g. animated tiles).
                    if let Some(slot) = self
                        .images
                        .iter_mut()
                        .find(|img| img.image_id == image_id)
                    {
                        *slot = new_image;
                    } else {
                        self.images.push(new_image);
                    }
                }
            }
            'd' => {
                // Delete images
                self.images.retain(|img| img.image_id != image_id);
            }
            _ => {}
        }
    }

    /// Remove images whose anchor row has scrolled off screen.
    #[allow(dead_code)]
    pub fn gc(&mut self, max_row: usize) {
        self.images.retain(|img| img.row < max_row + img.rows_tall);
    }
}

/// Simple base64 decoder (no padding required).
fn decode_base64(input: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(input.len() * 3 / 4);
    let mut buf: u32 = 0;
    let mut bits: u32 = 0;

    for &b in input.as_bytes() {
        let val = match b {
            b'A'..=b'Z' => b - b'A',
            b'a'..=b'z' => b - b'a' + 26,
            b'0'..=b'9' => b - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' | b'\n' | b'\r' => continue,
            _ => continue,
        };
        buf = (buf << 6) | val as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
            buf &= (1 << bits) - 1;
        }
    }
    out
}

/// Decode a PNG image to RGBA pixels. Returns (width, height, rgba_data).
fn decode_png(data: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    // Minimal PNG decoder using our own code — the `image` crate is already
    // in the dependency tree via the window icon loader.
    let img = image::load_from_memory(data).ok()?;
    let rgba = img.into_rgba8();
    let w = rgba.width();
    let h = rgba.height();
    Some((w, h, rgba.into_raw()))
}

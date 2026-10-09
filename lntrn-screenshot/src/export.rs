//! Turning the finished selection into a PNG: crop the captured frame, then
//! save it to disk and/or encode it for the clipboard.

use std::path::PathBuf;
use std::sync::Arc;

use crate::SelectionUi;

impl SelectionUi {
    pub(crate) fn export(&self, copy: bool, save: bool) -> Option<Arc<Vec<u8>>> {
        // Selection coords are in physical pixels (same space as the
        // captured image), so we crop directly without rescaling.
        let (crop_x, crop_y, crop_w, crop_h) = if let Some(ref sel) = self.selection {
            let (sx, sy, sw, sh) = sel.normalized();
            (
                sx.max(0.0) as u32,
                sy.max(0.0) as u32,
                (sw.max(1.0) as u32).min(self.capture_width),
                (sh.max(1.0) as u32).min(self.capture_height),
            )
        } else {
            (0, 0, self.capture_width, self.capture_height)
        };

        let img = image::RgbaImage::from_raw(
            self.capture_width,
            self.capture_height,
            self.capture_data.clone(),
        )?;
        let cropped = image::imageops::crop_imm(&img, crop_x, crop_y, crop_w, crop_h).to_image();

        if save {
            let path = self.output_path.clone().unwrap_or_else(default_output_path);
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            match cropped.save(&path) {
                Ok(()) => eprintln!("Saved to {}", path.display()),
                Err(e) => eprintln!("Failed to save: {e}"),
            }
        }

        if copy {
            use image::ImageEncoder;
            let mut png_data = Vec::new();
            let encoder = image::codecs::png::PngEncoder::new(&mut png_data);
            if let Err(e) = encoder.write_image(
                cropped.as_raw(),
                crop_w,
                crop_h,
                image::ExtendedColorType::Rgba8,
            ) {
                eprintln!("Failed to encode PNG: {e}");
                return None;
            }
            eprintln!("Clipboard: {} bytes PNG", png_data.len());
            return Some(Arc::new(png_data));
        }
        None
    }
}

fn default_output_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    let dir = PathBuf::from(home).join("Pictures").join("Screenshots");
    let ts = timestamp();
    dir.join(format!("screenshot_{ts}.png"))
}

fn timestamp() -> String {
    unsafe {
        let mut t: libc::time_t = 0;
        libc::time(&mut t);
        let tm = libc::localtime(&t);
        if tm.is_null() {
            return format!("{t}");
        }
        let tm = &*tm;
        format!(
            "{:04}-{:02}-{:02}_{:02}-{:02}-{:02}",
            tm.tm_year + 1900,
            tm.tm_mon + 1,
            tm.tm_mday,
            tm.tm_hour,
            tm.tm_min,
            tm.tm_sec,
        )
    }
}

mod auth;
mod config;
mod dispatch;
mod entry;
mod keyboard;
mod output;
mod render;
mod wayland;

use std::os::fd::AsRawFd;
use std::path::Path;

use image::DynamicImage;
use output::BgImage;

fn main() {
    // Single-instance guard: if a lock screen is already up, do nothing.
    // Keeps idle/lid/CLI triggers from stacking multiple lock screens.
    let _guard = match acquire_lock() {
        Some(fd) => fd,
        None => {
            // Already locked — silently succeed so triggers are idempotent.
            return;
        }
    };

    let style = config::style();
    let bg = load_background();

    if let Err(err) = wayland::run(bg, style) {
        eprintln!("[lntrn-lockscreen] {err}");
        std::process::exit(1);
    }
}

/// Acquire an exclusive advisory lock so only one lock screen runs at a time.
/// Returns the held file (kept open for the process lifetime) or None if held.
fn acquire_lock() -> Option<std::fs::File> {
    let path = runtime_dir().join("lntrn-lockscreen.lock");
    let file = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(&path)
        .ok()?;
    let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if rc == 0 {
        Some(file)
    } else {
        None
    }
}

fn runtime_dir() -> std::path::PathBuf {
    std::env::var("XDG_RUNTIME_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::path::PathBuf::from("/tmp"))
}

/// Decode the configured background, or fall back to a solid accent fill.
fn load_background() -> BgImage {
    if let Some(path) = config::background_path() {
        match decode(&path) {
            Ok(img) => {
                let rgba = shrink_to_fit(img, output::max_texture_side()).to_rgba8();
                let (w, h) = rgba.dimensions();
                return BgImage {
                    rgba: rgba.into_raw(),
                    w,
                    h,
                };
            }
            Err(err) => eprintln!(
                "[lntrn-lockscreen] can't use background {}: {err}",
                path.display()
            ),
        }
    }
    solid_fallback(config::accent())
}

/// Decode a PNG or JPEG, told apart by the file's contents rather than its
/// extension.
fn decode(path: &Path) -> image::ImageResult<DynamicImage> {
    image::ImageReader::open(path)?
        .with_guessed_format()?
        .decode()
}

/// Scale an image down, keeping its aspect ratio, until neither side is longer
/// than `max_side`. The GPU can't take a bigger texture: uploading one aborts
/// the process, which for a lock screen means a session nobody can unlock.
fn shrink_to_fit(img: DynamicImage, max_side: u32) -> DynamicImage {
    if img.width() <= max_side && img.height() <= max_side {
        img
    } else {
        img.thumbnail(max_side, max_side)
    }
}

/// A 1x1 solid texture used when no background image is available.
fn solid_fallback(c: lntrn_theme::Rgba) -> BgImage {
    // Darken the accent so foreground text stays readable.
    let rgba = vec![c.r / 3, c.g / 3, c.b / 3, 255];
    BgImage { rgba, w: 1, h: 1 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_image_that_fits_is_left_alone() {
        let img = shrink_to_fit(DynamicImage::new_rgba8(1672, 941), 8192);
        assert_eq!((img.width(), img.height()), (1672, 941));
    }

    #[test]
    fn an_oversized_image_shrinks_to_the_limit() {
        // A panorama far wider than any texture.
        let img = shrink_to_fit(DynamicImage::new_rgba8(12000, 3000), 8192);
        assert_eq!((img.width(), img.height()), (8192, 2048));

        // Too tall instead of too wide.
        let img = shrink_to_fit(DynamicImage::new_rgba8(100, 400), 200);
        assert_eq!((img.width(), img.height()), (50, 200));
    }

    #[test]
    fn jpeg_and_png_decode_by_content_not_extension() {
        let dir = std::env::temp_dir().join(format!("lntrn-lockscreen-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let source = DynamicImage::new_rgb8(8, 6);

        // Each format saved under the other's extension.
        let jpeg_named_png = dir.join("photo.png");
        source
            .save_with_format(&jpeg_named_png, image::ImageFormat::Jpeg)
            .unwrap();
        let png_named_jpg = dir.join("art.jpg");
        source
            .save_with_format(&png_named_jpg, image::ImageFormat::Png)
            .unwrap();

        for path in [&jpeg_named_png, &png_named_jpg] {
            let img = decode(path).unwrap();
            assert_eq!((img.width(), img.height()), (8, 6));
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

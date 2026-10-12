//! Ctrl+C: put the open image on the clipboard. Two flavours are offered so
//! every paste target gets what it wants — `image/png` for editors, browsers
//! and chat apps, `text/uri-list` so Fox pastes the file itself.
//!
//! PNG bytes are produced on a worker thread (a 20-megapixel JPEG takes a
//! moment to re-encode); a paste that lands before they're ready waits on
//! the condvar on its own thread rather than stalling the UI.

use std::io::Write;
use std::path::Path;
use std::sync::{Arc, Condvar, Mutex};

use wayland_client::protocol::{wl_data_device, wl_data_source};
use wayland_client::{Connection, Dispatch, QueueHandle};

use crate::wayland::State;

type PngSlot = Arc<(Mutex<Option<Arc<Vec<u8>>>>, Condvar)>;

/// Longest side an SVG is rasterized at for pasting (its native size, unless
/// that's enormous).
const MAX_SVG_PASTE: u32 = 8192;

/// Ctrl+C: hand the open image to the clipboard and report the outcome in
/// the status bar.
pub fn copy_current(
    state: &State,
    data_device: Option<&wl_data_device::WlDataDevice>,
    qh: &QueueHandle<State>,
    app: &mut crate::app::App,
) {
    // Nothing loaded (or it failed to): no pixels to offer.
    if app.image.is_none() {
        return;
    }
    let Some(path) = app.path.clone() else {
        return;
    };
    let result = match data_device {
        Some(dev) => copy_image(state, dev, qh, state.key_serial, &path),
        None => Err("Clipboard unavailable (no data device)".to_string()),
    };
    match result {
        Ok(()) => app.flash("Copied to clipboard"),
        Err(e) => app.flash(e),
    }
}

pub struct ClipPayload {
    uri_list: Vec<u8>,
    png: PngSlot,
}

/// Offer `path` on the clipboard. `serial` must be a recent input serial
/// (the Ctrl+C key press) or the compositor ignores the selection.
pub fn copy_image(
    state: &State,
    device: &wl_data_device::WlDataDevice,
    qh: &QueueHandle<State>,
    serial: u32,
    path: &Path,
) -> Result<(), String> {
    let mgr = state
        .data_device_manager
        .as_ref()
        .ok_or_else(|| "Clipboard unavailable (no data device manager)".to_string())?;

    let png: PngSlot = Arc::new((Mutex::new(None), Condvar::new()));
    let payload = ClipPayload {
        uri_list: format!("file://{}\r\n", percent_encode_path(path)).into_bytes(),
        png: Arc::clone(&png),
    };
    let source = mgr.create_data_source(qh, payload);
    source.offer("image/png".to_string());
    source.offer("text/uri-list".to_string());
    device.set_selection(Some(&source), serial);

    let path = path.to_path_buf();
    std::thread::spawn(move || {
        // Empty on failure — or on a decoder panic, which would otherwise
        // leave the slot unfilled — so a waiting paste gets EOF, not a hang.
        let bytes = std::panic::catch_unwind(|| encode_png(&path))
            .ok()
            .flatten()
            .unwrap_or_default();
        let (lock, cv) = &*png;
        *lock.lock().unwrap() = Some(Arc::new(bytes));
        cv.notify_all();
    });
    Ok(())
}

/// PNG bytes for `path`: the file itself when it already is one, otherwise a
/// fresh encode of the decoded (orientation-corrected) pixels. SVGs are
/// rasterized at their native size.
fn encode_png(path: &Path) -> Option<Vec<u8>> {
    if crate::app::is_svg(path) {
        let source = std::fs::read_to_string(path).ok()?;
        let (nw, nh) = crate::loaders::peek_image_dimensions(path)?;
        let (nw, nh) = (nw as f32, nh as f32);
        let (w, h) = crate::loaders::svg_raster_size(nw, nh, 1.0, MAX_SVG_PASTE);
        let (rgba, w, h) = crate::loaders::rasterize_svg_rgba(&source, nw, nh, w, h)?;
        let img = image::RgbaImage::from_raw(w, h, rgba)?;
        let mut out = std::io::Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageFormat::Png).ok()?;
        return Some(out.into_inner());
    }
    let decoded = crate::loaders::decode_raster(path)?;
    if decoded.format == Some(image::ImageFormat::Png) {
        return std::fs::read(path).ok();
    }
    let mut out = std::io::Cursor::new(Vec::new());
    decoded
        .image
        .write_to(&mut out, image::ImageFormat::Png)
        .ok()?;
    Some(out.into_inner())
}

/// RFC 3986 escaping for a filesystem path: keep unreserved chars and `/`,
/// hex-escape everything else byte-wise (paths need not be UTF-8).
fn percent_encode_path(path: &Path) -> String {
    use std::os::unix::ffi::OsStrExt;
    let mut out = String::new();
    for &b in path.as_os_str().as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

impl Dispatch<wl_data_source::WlDataSource, ClipPayload> for State {
    fn event(
        _: &mut Self,
        source: &wl_data_source::WlDataSource,
        event: wl_data_source::Event,
        payload: &ClipPayload,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_data_source::Event::Send { mime_type, fd } => {
                let mut file = std::fs::File::from(fd);
                match mime_type.as_str() {
                    "text/uri-list" => {
                        let _ = file.write_all(&payload.uri_list);
                    }
                    "image/png" => {
                        let png = Arc::clone(&payload.png);
                        std::thread::spawn(move || {
                            let (lock, cv) = &*png;
                            let mut slot = lock.lock().unwrap();
                            while slot.is_none() {
                                slot = cv.wait(slot).unwrap();
                            }
                            let bytes = Arc::clone(slot.as_ref().unwrap());
                            drop(slot);
                            let _ = file.write_all(&bytes);
                        });
                    }
                    _ => {}
                }
            }
            // Another client took the clipboard — we're done serving.
            wl_data_source::Event::Cancelled => source.destroy(),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_spaces_and_unicode_but_not_slashes() {
        let p = Path::new("/home/alva/My Pics/ChatGPT Image Aug 30, 2026.jpg");
        assert_eq!(
            percent_encode_path(p),
            "/home/alva/My%20Pics/ChatGPT%20Image%20Aug%2030%2C%202026.jpg"
        );
        assert_eq!(
            percent_encode_path(Path::new("/tmp/ñ.png")),
            "/tmp/%C3%B1.png"
        );
    }
}

//! Prism documents (`.prism`) carry a picture of themselves, drawn by
//! Prism's own renderer when it saves (lantern-prism, D083): a PNG in a
//! `THMB` chunk, the first after the header. Only the chunk headers and
//! that PNG are read, never the document.
//!
//! ```text
//! "PRSM" u32 format_version u32 app_version_len app_version
//! chunk := [u8; 4] tag, u64 id, u32 len, payload     (all little-endian)
//! ```

use std::os::unix::fs::FileExt;
use std::path::Path;

const MAGIC: &[u8; 4] = b"PRSM";
const THUMBNAIL: &[u8; 4] = b"THMB";
/// Prism writes the picture first; a few chunks in, a file without one
/// (saved before D083, or with no visible mesh) stops being searched.
const MAX_CHUNKS: usize = 16;
/// Prism's pictures are tens of kilobytes; this leaves room and no more.
const MAX_PNG_BYTES: u64 = 16 * 1024 * 1024;

/// The picture a `.prism` file carries, shrunk to fit `fit`. `None` for a
/// file without one: it keeps the generic icon until Prism saves it again.
pub(super) fn thumbnail(path: &Path, fit: u32) -> Option<image::RgbaImage> {
    let png = read_png(path)?;
    let reader = image::ImageReader::with_format(std::io::Cursor::new(png), image::ImageFormat::Png);
    let (img, _) = crate::thumbs::decode_unrotated(reader, crate::thumbs::MAX_DECODE_DIM, crate::thumbs::MAX_DECODE_BYTES)?;
    let img = if img.width() > fit || img.height() > fit { img.thumbnail(fit, fit) } else { img };
    Some(img.to_rgba8())
}

/// The bytes of the file's `THMB` chunk.
fn read_png(path: &Path) -> Option<Vec<u8>> {
    let file = super::gltf::open_regular(path)?;
    let len = file.metadata().ok()?.len();
    let mut head = [0u8; 12];
    file.read_exact_at(&mut head, 0).ok()?;
    if &head[..4] != MAGIC {
        return None;
    }
    let mut at = 12 + u64::from(u32::from_le_bytes([head[8], head[9], head[10], head[11]]));
    for _ in 0..MAX_CHUNKS {
        let mut chunk = [0u8; 16];
        file.read_exact_at(&mut chunk, at).ok()?;
        let size = u64::from(u32::from_le_bytes([chunk[12], chunk[13], chunk[14], chunk[15]]));
        let data = at + 16;
        if data + size > len {
            return None; // cut short
        }
        if &chunk[..4] == THUMBNAIL {
            if size > MAX_PNG_BYTES {
                return None;
            }
            let mut png = vec![0u8; size as usize];
            file.read_exact_at(&mut png, data).ok()?;
            return Some(png);
        }
        at = data + size;
    }
    None
}

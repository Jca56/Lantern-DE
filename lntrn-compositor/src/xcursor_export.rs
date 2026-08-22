//! Export the user's Lantern cursor SVGs as a real X-cursor theme on disk
//! so XWayland clients (Steam, Wine, anything that reads `XCURSOR_THEME`)
//! see the same cursor as native Wayland clients. Without this, X11 apps
//! either fall back to the bare X11 root cursor or — if a system theme like
//! Adwaita is installed — render *that*, which doesn't match the rest of
//! the desktop.
//!
//! Theme is generated lazily into `$XDG_DATA_HOME/icons/Lantern` (default
//! `~/.local/share/icons/Lantern`) with a fingerprint file so we only
//! rasterize on first run or when the user's cursor SVG / config changes.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::io::Write;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};

use crate::cursor::{recolor_cursor_svg, strip_preserve_aspect_none};

/// Sizes packed into every cursor file. X clients pick the closest match
/// for the active `XCURSOR_SIZE`; we cover the common HiDPI range.
const SIZES: &[u32] = &[16, 24, 32, 48, 64, 96, 128];

/// Sentinel hotspot used by the resize SVGs (centered at 32×32 viewbox).
const RESIZE_HOTSPOT: (f32, f32) = (16.0, 16.0);

/// Where the theme is installed. Honors `$XDG_DATA_HOME` per the
/// freedesktop basedir spec, falls back to `~/.local/share`.
pub fn theme_dir() -> PathBuf {
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| {
            let home = std::env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("/"));
            home.join(".local").join("share")
        });
    base.join("icons").join("Lantern")
}

/// Public entry: regenerate the theme if its fingerprint is stale. Cheap
/// to call on every compositor startup — the fingerprint check skips
/// rasterization on a no-op run.
pub fn export() {
    if let Err(err) = export_inner() {
        tracing::warn!("Xcursor export failed: {err}");
    }
}

fn export_inner() -> std::io::Result<()> {
    let theme = theme_dir();
    let cursors = theme.join("cursors");
    std::fs::create_dir_all(&cursors)?;

    // The arrow source — either the user's currently-active custom theme
    // SVG (e.g. `lntrn-cursor-4.svg`) or the bundled default.
    let (arrow_path, arrow_data, arrow_recolor) = arrow_source();
    let resize_sources = [
        ("ew-resize", "lntrn-cursor-ew.svg"),
        ("ns-resize", "lntrn-cursor-ns.svg"),
        ("nesw-resize", "lntrn-cursor-nesw.svg"),
        ("nwse-resize", "lntrn-cursor-nwse.svg"),
    ];
    let resize_data: Vec<(&str, Vec<u8>)> = resize_sources
        .iter()
        .map(|(key, file)| {
            let bytes = read_lantern_svg(file).unwrap_or_default();
            (*key, bytes)
        })
        .collect();

    let fingerprint = compute_fingerprint(&arrow_path, &arrow_data, &resize_data);
    let fp_path = theme.join(".fingerprint");
    if fp_path.exists() {
        if let Ok(prev) = std::fs::read_to_string(&fp_path) {
            if prev.trim() == fingerprint {
                tracing::debug!("Lantern Xcursor theme up to date ({})", fingerprint);
                return Ok(());
            }
        }
    }

    tracing::info!("Generating Lantern Xcursor theme at {}", theme.display());

    // Index file (advertises a 24px nominal cursor; real X clients still
    // get the closest size available).
    std::fs::write(
        theme.join("index.theme"),
        "[Icon Theme]\nName=Lantern\nComment=Generated from Lantern cursor SVGs\nInherits=hicolor\n",
    )?;
    std::fs::write(
        theme.join("cursor.theme"),
        "[Icon Theme]\nName=Lantern\nInherits=Lantern\n",
    )?;

    // Wipe existing cursor files so renamed/removed aliases don't linger.
    if cursors.exists() {
        for entry in std::fs::read_dir(&cursors)? {
            let entry = entry?;
            let _ = std::fs::remove_file(entry.path());
        }
    }

    let arrow_recolored = if arrow_recolor {
        recolor_cursor_svg(&arrow_data, "default")
    } else {
        arrow_data.clone()
    };
    write_cursor_file(
        &cursors.join("default"),
        &arrow_recolored,
        ARROW_HOTSPOT_FROM_CURSOR_RS(),
    )?;

    for (key, raw) in &resize_data {
        if raw.is_empty() {
            tracing::warn!("Missing source SVG for {key}; skipping");
            continue;
        }
        let recolored = recolor_cursor_svg(raw, key);
        let path = cursors.join(*key);
        write_cursor_file(&path, &recolored, RESIZE_HOTSPOT)?;
    }

    // Aliases (the X server / Xcursor library looks up many legacy names —
    // every entry here makes that lookup land on one of the cursors above).
    for (alias, target) in ALIASES {
        let link = cursors.join(alias);
        let _ = std::fs::remove_file(&link);
        if let Err(err) = symlink(target, &link) {
            tracing::warn!("Failed to symlink {alias} -> {target}: {err}");
        }
    }

    std::fs::write(&fp_path, &fingerprint)?;
    tracing::info!("Lantern Xcursor theme ready ({} shapes)", ALIASES.len() + 5);
    Ok(())
}

/// Inset-corrected hotspot for the arrow tip, mirrors `hotspot_for_icon`
/// in cursor.rs but pulled out so we don't depend on the per-frame
/// recolor cache state.
#[allow(non_snake_case)]
fn ARROW_HOTSPOT_FROM_CURSOR_RS() -> (f32, f32) {
    let scale = crate::input::read_input_setting_f64("cursor_outline_scale", 1.0) as f32;
    let effective_stroke = 6.0 * scale.max(0.0);
    let inset = (effective_stroke * 0.5) / std::f32::consts::SQRT_2;
    let tip_x = (6.236 - inset).max(0.0);
    let tip_y = (3.14 - inset).max(0.0);
    (tip_x, tip_y)
}

/// Resolve the SVG for the default arrow, mirroring cursor.rs's lookup
/// order: custom-theme SVG in `~/.lantern/config/cursors/{theme}.svg`
/// first, then the runtime override at `~/.lantern/icons/cursors/`, then
/// the embedded asset. Returns `(label, bytes, should_recolor)` — custom
/// themes ship with their authored palette and are kept verbatim.
fn arrow_source() -> (String, Vec<u8>, bool) {
    let theme_name = crate::input::read_input_setting("cursor_theme", "default");
    if theme_name != "default" {
        let path = crate::lantern_home()
            .join("config/cursors")
            .join(format!("{theme_name}.svg"));
        if let Ok(bytes) = std::fs::read(&path) {
            return (path.display().to_string(), bytes, false);
        }
    }
    let bytes = read_lantern_svg("lntrn-cursor.svg").unwrap_or_default();
    (
        "lntrn-cursor.svg (bundled)".to_string(),
        bytes,
        true, // recolor the canonical-palette bundled SVG
    )
}

/// Read a Lantern cursor SVG — runtime override at
/// `~/.lantern/icons/cursors/{file}` first, embedded asset as the fallback.
fn read_lantern_svg(file: &str) -> Option<Vec<u8>> {
    let runtime = crate::lantern_home().join("icons/cursors").join(file);
    if let Ok(bytes) = std::fs::read(&runtime) {
        return Some(bytes);
    }
    lntrn_icons::get(file).map(|s| s.to_vec())
}

/// Build one Xcursor file containing this shape at every requested size.
fn write_cursor_file(
    path: &Path,
    svg_data: &[u8],
    hotspot_viewbox: (f32, f32),
) -> std::io::Result<()> {
    let sanitized = strip_preserve_aspect_none(svg_data);
    let Some(tree) =
        resvg::usvg::Tree::from_data(&sanitized, &resvg::usvg::Options::default()).ok()
    else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "could not parse cursor SVG",
        ));
    };
    let mut images = Vec::with_capacity(SIZES.len());
    for &nominal in SIZES {
        if let Some(img) = rasterize_tree(&tree, nominal, hotspot_viewbox) {
            images.push(img);
        }
    }

    if images.is_empty() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "no images rasterized",
        ));
    }

    let mut file = std::fs::File::create(path)?;
    write_xcursor(&mut file, &images)
}

/// Rasterize one shape at one nominal size. Shared by the theme file writer
/// (all sizes) and the X11 root cursor (single size, x11_resources.rs).
fn rasterize_tree(
    tree: &resvg::usvg::Tree,
    nominal: u32,
    hotspot_viewbox: (f32, f32),
) -> Option<XcursorImage> {
    let tree_size = tree.size();
    let sx = nominal as f32 / tree_size.width();
    let sy = nominal as f32 / tree_size.height();
    let scale = sx.min(sy);
    let w = (tree_size.width() * scale).round() as u32;
    let h = (tree_size.height() * scale).round() as u32;
    if w == 0 || h == 0 {
        return None;
    }
    let mut pixmap = resvg::tiny_skia::Pixmap::new(w, h)?;
    let transform = resvg::tiny_skia::Transform::from_scale(scale, scale);
    resvg::render(tree, transform, &mut pixmap.as_mut());

    Some(XcursorImage {
        nominal,
        width: w,
        height: h,
        xhot: (hotspot_viewbox.0 * scale).round() as u32,
        yhot: (hotspot_viewbox.1 * scale).round() as u32,
        // resvg gives us premultiplied RGBA; Xcursor wants ARGB stored as
        // little-endian u32 (i.e. on-disk byte order BGRA), also premultiplied.
        pixels: rgba_to_bgra(pixmap.data()),
    })
}

/// Rasterize the active default-arrow cursor at a single size — same source
/// SVG, recolor rules, and hotspot as the theme's `default` shape. Used for
/// the X11 root window cursor.
pub fn rasterize_default_arrow(size: u32) -> Option<XcursorImage> {
    let (_, data, recolor) = arrow_source();
    let svg = if recolor {
        recolor_cursor_svg(&data, "default")
    } else {
        data
    };
    let sanitized = strip_preserve_aspect_none(&svg);
    let tree = resvg::usvg::Tree::from_data(&sanitized, &resvg::usvg::Options::default()).ok()?;
    rasterize_tree(&tree, size, ARROW_HOTSPOT_FROM_CURSOR_RS())
}

fn rgba_to_bgra(rgba: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(rgba.len());
    for chunk in rgba.chunks_exact(4) {
        out.push(chunk[2]); // B
        out.push(chunk[1]); // G
        out.push(chunk[0]); // R
        out.push(chunk[3]); // A
    }
    out
}

pub struct XcursorImage {
    nominal: u32,
    pub width: u32,
    pub height: u32,
    pub xhot: u32,
    pub yhot: u32,
    /// Premultiplied u32 ARGB stored little-endian (byte order BGRA).
    pub pixels: Vec<u8>,
}

// Xcursor binary format constants (xorg/lib/libXcursor file.c).
const XCURSOR_MAGIC: &[u8] = b"Xcur";
const XCURSOR_FILE_HEADER_LEN: u32 = 16;
const XCURSOR_TOC_LEN: u32 = 12;
const XCURSOR_IMAGE_TYPE: u32 = 0xfffd0002;
const XCURSOR_IMAGE_HEADER_LEN: u32 = 36;
const XCURSOR_IMAGE_VERSION: u32 = 1;
const XCURSOR_FILE_VERSION: u32 = 0x00010000;

fn write_xcursor<W: Write>(w: &mut W, images: &[XcursorImage]) -> std::io::Result<()> {
    let ntoc = images.len() as u32;
    let header_total = XCURSOR_FILE_HEADER_LEN + XCURSOR_TOC_LEN * ntoc;

    // Compute the byte offset to each image chunk, packed back-to-back
    // after the file header + TOC.
    let mut offsets = Vec::with_capacity(images.len());
    let mut cursor = header_total;
    for img in images {
        offsets.push(cursor);
        cursor += XCURSOR_IMAGE_HEADER_LEN + img.width * img.height * 4;
    }

    // File header.
    w.write_all(XCURSOR_MAGIC)?;
    write_u32(w, XCURSOR_FILE_HEADER_LEN)?;
    write_u32(w, XCURSOR_FILE_VERSION)?;
    write_u32(w, ntoc)?;

    // TOC entries.
    for (img, off) in images.iter().zip(offsets.iter()) {
        write_u32(w, XCURSOR_IMAGE_TYPE)?;
        write_u32(w, img.nominal)?;
        write_u32(w, *off)?;
    }

    // Image chunks.
    for img in images {
        write_u32(w, XCURSOR_IMAGE_HEADER_LEN)?;
        write_u32(w, XCURSOR_IMAGE_TYPE)?;
        write_u32(w, img.nominal)?;
        write_u32(w, XCURSOR_IMAGE_VERSION)?;
        write_u32(w, img.width)?;
        write_u32(w, img.height)?;
        write_u32(w, img.xhot)?;
        write_u32(w, img.yhot)?;
        write_u32(w, 0)?; // delay (ms) — static cursor
        w.write_all(&img.pixels)?;
    }
    Ok(())
}

fn write_u32<W: Write>(w: &mut W, v: u32) -> std::io::Result<()> {
    w.write_all(&v.to_le_bytes())
}

fn compute_fingerprint(arrow_path: &str, arrow_data: &[u8], resize: &[(&str, Vec<u8>)]) -> String {
    let mut h = DefaultHasher::new();
    arrow_path.hash(&mut h);
    arrow_data.hash(&mut h);
    for (k, v) in resize {
        k.hash(&mut h);
        v.hash(&mut h);
    }
    // Include recolor knobs so palette changes invalidate the cache.
    for key in [
        "cursor_body_light",
        "cursor_body_dark",
        "cursor_accent_light",
        "cursor_accent_dark",
        "cursor_outline_color",
    ] {
        crate::input::read_input_setting(key, "").hash(&mut h);
    }
    crate::input::read_input_setting_f64("cursor_outline_scale", 1.0)
        .to_bits()
        .hash(&mut h);
    crate::input::read_input_setting_f64("cursor_corner_radius", 0.0)
        .to_bits()
        .hash(&mut h);
    format!("{:016x}", h.finish())
}

/// Symlink aliases: `alias` → `target`. The full set of X cursor names a
/// typical client looks up; every line is one less "cursor not found"
/// fallback to the X server's hardcoded root cursor.
const ALIASES: &[(&str, &str)] = &[
    // Arrow / default
    ("left_ptr", "default"),
    ("top_left_arrow", "default"),
    ("arrow", "default"),
    ("X_cursor", "default"),
    // Text caret (we don't have bespoke art yet — arrow is better than the
    // X root cursor).
    ("text", "default"),
    ("xterm", "default"),
    ("ibeam", "default"),
    // Pointer / hand
    ("pointer", "default"),
    ("hand", "default"),
    ("hand1", "default"),
    ("hand2", "default"),
    ("pointing_hand", "default"),
    ("openhand", "default"),
    ("grab", "default"),
    ("closedhand", "default"),
    ("grabbing", "default"),
    // Wait / progress
    ("wait", "default"),
    ("watch", "default"),
    ("clock", "default"),
    ("progress", "default"),
    ("left_ptr_watch", "default"),
    // Move / all-scroll
    ("move", "default"),
    ("fleur", "default"),
    ("size_all", "default"),
    ("all-scroll", "default"),
    ("dnd-move", "default"),
    // Crosshair / help
    ("crosshair", "default"),
    ("cross", "default"),
    ("tcross", "default"),
    ("help", "default"),
    ("question_arrow", "default"),
    // Forbidden / no-drop
    ("not-allowed", "default"),
    ("crossed_circle", "default"),
    ("forbidden", "default"),
    ("no-drop", "default"),
    // E/W resize
    ("e-resize", "ew-resize"),
    ("w-resize", "ew-resize"),
    ("col-resize", "ew-resize"),
    ("right_side", "ew-resize"),
    ("left_side", "ew-resize"),
    ("sb_h_double_arrow", "ew-resize"),
    ("h_double_arrow", "ew-resize"),
    ("size_hor", "ew-resize"),
    ("split_h", "ew-resize"),
    // N/S resize
    ("n-resize", "ns-resize"),
    ("s-resize", "ns-resize"),
    ("row-resize", "ns-resize"),
    ("top_side", "ns-resize"),
    ("bottom_side", "ns-resize"),
    ("sb_v_double_arrow", "ns-resize"),
    ("v_double_arrow", "ns-resize"),
    ("size_ver", "ns-resize"),
    ("split_v", "ns-resize"),
    // NE/SW resize
    ("ne-resize", "nesw-resize"),
    ("sw-resize", "nesw-resize"),
    ("top_right_corner", "nesw-resize"),
    ("bottom_left_corner", "nesw-resize"),
    ("size_bdiag", "nesw-resize"),
    ("fd_double_arrow", "nesw-resize"),
    // NW/SE resize
    ("nw-resize", "nwse-resize"),
    ("se-resize", "nwse-resize"),
    ("top_left_corner", "nwse-resize"),
    ("bottom_right_corner", "nwse-resize"),
    ("size_fdiag", "nwse-resize"),
    ("bd_double_arrow", "nwse-resize"),
];

// ── Drag overlay window ──────────────────────────────────────────────────────
// Creates a small floating ARGB window that follows the cursor during an
// external XDND drag session, showing the actual file/folder icon.

use x11rb::connection::Connection;
use x11rb::protocol::xproto::*;
use x11rb::rust_connection::RustConnection;

/// Size of the overlay icon in pixels.
const OVERLAY_SIZE: u32 = 48;
/// Offset from the cursor to the overlay origin.
const CURSOR_OFFSET: i16 = 16;

/// Information needed to render the drag overlay icon.
pub struct DragIcon {
    /// System icon path (SVG or PNG) for the primary entry.
    pub icon_path: Option<String>,
    /// Whether the primary entry is a directory.
    pub is_dir: bool,
    /// Total number of items being dragged (shows a badge if > 1).
    pub count: usize,
}

/// Resources for the overlay window so the event loop can move/destroy it.
pub struct OverlayWindow {
    pub window: Window,
    pub colormap: Colormap,
}

// ── Public helpers ───────────────────────────────────────────────────────────

/// Create the drag overlay window near the cursor. Returns None if ARGB
/// visuals are unavailable (the drag proceeds without an overlay).
pub fn create_overlay(
    conn: &RustConnection,
    screen: &Screen,
    cursor_x: i16,
    cursor_y: i16,
    icon: &DragIcon,
) -> Option<OverlayWindow> {
    // Find a 32-bit ARGB visual
    let (depth, visual_id) = find_argb_visual(screen)?;

    // Create a colormap for the 32-bit visual
    let colormap: Colormap = conn.generate_id().ok()?;
    conn.create_colormap(ColormapAlloc::NONE, colormap, screen.root, visual_id)
        .ok()?;

    // Generate the overlay window
    let win: Window = conn.generate_id().ok()?;
    conn.create_window(
        depth,
        win,
        screen.root,
        cursor_x.saturating_add(CURSOR_OFFSET),
        cursor_y.saturating_add(CURSOR_OFFSET),
        OVERLAY_SIZE as u16,
        OVERLAY_SIZE as u16,
        0,
        WindowClass::INPUT_OUTPUT,
        visual_id,
        &CreateWindowAux::new()
            .colormap(colormap)
            .border_pixel(0)
            .background_pixel(0)
            .override_redirect(1)
            .event_mask(EventMask::NO_EVENT),
    )
    .ok()?;

    // Render the icon pixels
    let pixels = render_icon(icon);

    // Paint into an off-screen Pixmap first, then set it as the window
    // background. This ensures the icon is visible immediately when the
    // window is mapped (put_image on an unmapped window is discarded).
    let pixmap: Pixmap = conn.generate_id().ok()?;
    conn.create_pixmap(depth, pixmap, win, OVERLAY_SIZE as u16, OVERLAY_SIZE as u16)
        .ok()?;

    let gc: Gcontext = conn.generate_id().ok()?;
    conn.create_gc(gc, pixmap, &CreateGCAux::new()).ok()?;

    conn.put_image(
        ImageFormat::Z_PIXMAP,
        pixmap,
        gc,
        OVERLAY_SIZE as u16,
        OVERLAY_SIZE as u16,
        0,
        0,
        0,
        depth,
        &pixels,
    )
    .ok()?;

    conn.free_gc(gc).ok()?;

    // Set the pixmap as the window background so it auto-paints on map/expose
    conn.change_window_attributes(
        win,
        &ChangeWindowAttributesAux::new().background_pixmap(pixmap),
    )
    .ok()?;

    conn.free_pixmap(pixmap).ok()?;

    // Map the window so it appears on screen
    conn.map_window(win).ok()?;

    // Raise the overlay above all other windows so it's not hidden
    // behind the Fox Flare window
    conn.configure_window(
        win,
        &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE),
    )
    .ok()?;

    conn.flush().ok()?;

    Some(OverlayWindow {
        window: win,
        colormap,
    })
}

/// Move the overlay to follow the cursor.
pub fn move_overlay(conn: &RustConnection, overlay: &OverlayWindow, x: i16, y: i16) {
    let _ = conn.configure_window(
        overlay.window,
        &ConfigureWindowAux::new()
            .x(i32::from(x.saturating_add(CURSOR_OFFSET)))
            .y(i32::from(y.saturating_add(CURSOR_OFFSET))),
    );
}

/// Destroy the overlay and free its colormap.
pub fn destroy_overlay(conn: &RustConnection, overlay: OverlayWindow) {
    let _ = conn.unmap_window(overlay.window);
    let _ = conn.destroy_window(overlay.window);
    let _ = conn.free_colormap(overlay.colormap);
    let _ = conn.flush();
}

// ── ARGB visual lookup ───────────────────────────────────────────────────────

/// Scan the screen's depth list for a 32-bit TrueColor visual.
fn find_argb_visual(screen: &Screen) -> Option<(u8, Visualid)> {
    for depth_info in &screen.allowed_depths {
        if depth_info.depth == 32 {
            for visual in &depth_info.visuals {
                if visual.class == VisualClass::TRUE_COLOR {
                    return Some((32, visual.visual_id));
                }
            }
        }
    }
    None
}

// ── Icon rendering ───────────────────────────────────────────────────────────

/// Load the icon to raw BGRA pixel data suitable for X11 ZPixmap (32-bit).
/// Falls back to a simple procedural icon if loading fails.
fn render_icon(icon: &DragIcon) -> Vec<u8> {
    // Try to load the actual file icon
    if let Some(ref path) = icon.icon_path {
        if let Some(pixels) = load_icon_bgra(path) {
            return maybe_add_badge(pixels, icon.count);
        }
    }

    // Fallback: draw a simple folder or file icon
    let pixels = draw_fallback_icon(icon.is_dir);
    maybe_add_badge(pixels, icon.count)
}

/// Load an icon file (SVG or raster) and return OVERLAY_SIZE×OVERLAY_SIZE BGRA bytes.
fn load_icon_bgra(path: &str) -> Option<Vec<u8>> {
    let size = OVERLAY_SIZE;

    if path.ends_with(".svg") || path.ends_with(".svgz") {
        load_svg_bgra(path, size)
    } else {
        load_raster_bgra(path, size)
    }
}

/// Load an SVG via resvg and convert to BGRA.
fn load_svg_bgra(path: &str, target_size: u32) -> Option<Vec<u8>> {
    let data = std::fs::read(path).ok()?;
    let tree = resvg::usvg::Tree::from_data(&data, &resvg::usvg::Options::default()).ok()?;

    let svg_size = tree.size();
    let scale_x = target_size as f32 / svg_size.width();
    let scale_y = target_size as f32 / svg_size.height();
    let scale = scale_x.min(scale_y);

    let w = (svg_size.width() * scale).ceil() as u32;
    let h = (svg_size.height() * scale).ceil() as u32;

    let mut pixmap = resvg::tiny_skia::Pixmap::new(w, h)?;
    let transform = resvg::tiny_skia::Transform::from_scale(scale, scale);
    resvg::render(&tree, transform, &mut pixmap.as_mut());

    // Pixmap is premultiplied RGBA; convert to BGRA for X11
    Some(rgba_to_bgra_resized(pixmap.data(), w, h, target_size))
}

/// Load a raster image via the image crate and convert to BGRA.
fn load_raster_bgra(path: &str, target_size: u32) -> Option<Vec<u8>> {
    let img = image::open(path).ok()?;
    let resized = img.resize(target_size, target_size, image::imageops::FilterType::Lanczos3);
    let rgba = resized.to_rgba8();
    let (w, h) = (rgba.width(), rgba.height());

    Some(rgba_to_bgra_resized(rgba.as_raw(), w, h, target_size))
}

/// Convert RGBA pixel data to centered BGRA in a target_size × target_size buffer.
fn rgba_to_bgra_resized(src: &[u8], src_w: u32, src_h: u32, target_size: u32) -> Vec<u8> {
    let ts = target_size as usize;
    let mut buf = vec![0u8; ts * ts * 4]; // All zeros = fully transparent

    // Center the source image in the target buffer
    let offset_x = (ts.saturating_sub(src_w as usize)) / 2;
    let offset_y = (ts.saturating_sub(src_h as usize)) / 2;

    for y in 0..src_h as usize {
        for x in 0..src_w as usize {
            let src_idx = (y * src_w as usize + x) * 4;
            if src_idx + 3 >= src.len() {
                continue;
            }
            let dst_x = offset_x + x;
            let dst_y = offset_y + y;
            if dst_x >= ts || dst_y >= ts {
                continue;
            }
            let dst_idx = (dst_y * ts + dst_x) * 4;
            // RGBA → BGRA
            buf[dst_idx] = src[src_idx + 2];     // B
            buf[dst_idx + 1] = src[src_idx + 1]; // G
            buf[dst_idx + 2] = src[src_idx];     // R
            buf[dst_idx + 3] = src[src_idx + 3]; // A
        }
    }

    buf
}

/// Draw a simple fallback folder or file icon.
fn draw_fallback_icon(is_dir: bool) -> Vec<u8> {
    let s = OVERLAY_SIZE as usize;
    let mut buf = vec![0u8; s * s * 4];

    if is_dir {
        // Gold folder rectangle
        let (r, g, b, a) = (200u8, 134, 10, 200);
        for y in (s / 4)..(s * 3 / 4) {
            for x in (s / 6)..(s * 5 / 6) {
                let idx = (y * s + x) * 4;
                buf[idx] = b;
                buf[idx + 1] = g;
                buf[idx + 2] = r;
                buf[idx + 3] = a;
            }
        }
        // Tab
        for y in (s / 6)..(s / 4) {
            for x in (s / 6)..(s / 2) {
                let idx = (y * s + x) * 4;
                buf[idx] = b;
                buf[idx + 1] = g;
                buf[idx + 2] = r;
                buf[idx + 3] = a;
            }
        }
    } else {
        // Gray file rectangle
        let (r, g, b, a) = (120u8, 120, 120, 200);
        for y in (s / 6)..(s * 5 / 6) {
            for x in (s / 4)..(s * 3 / 4) {
                let idx = (y * s + x) * 4;
                buf[idx] = b;
                buf[idx + 1] = g;
                buf[idx + 2] = r;
                buf[idx + 3] = a;
            }
        }
    }

    buf
}

/// If count > 1, draw a small red badge in the top-right corner.
fn maybe_add_badge(mut pixels: Vec<u8>, count: usize) -> Vec<u8> {
    if count <= 1 {
        return pixels;
    }

    let s = OVERLAY_SIZE as usize;
    // Badge: filled circle in top-right corner
    let badge_r = 8usize;
    let cx = s - badge_r - 2;
    let cy = badge_r + 2;

    for y in 0..s {
        for x in 0..s {
            let dx = x as isize - cx as isize;
            let dy = y as isize - cy as isize;
            if (dx * dx + dy * dy) <= (badge_r * badge_r) as isize {
                let idx = (y * s + x) * 4;
                // Red badge (BGRA)
                pixels[idx] = 50;      // B
                pixels[idx + 1] = 50;  // G
                pixels[idx + 2] = 230; // R
                pixels[idx + 3] = 240; // A
            }
        }
    }

    pixels
}

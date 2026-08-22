//! Server-side X11 cursor defaults, applied once XWayland is ready.
//!
//! The `XCURSOR_THEME` / `XCURSOR_SIZE` / `XCURSOR_PATH` env vars set in
//! xwayland.rs cover clients that load cursors through libXcursor — but not
//! everything:
//!
//! 1. Chromium/CEF apps (Steam's webhelper, Spotify, Electron) ignore those
//!    env vars entirely and read `Xcursor.theme` / `Xcursor.size` from the
//!    `RESOURCE_MANAGER` property on the X root window — the thing `xrdb`
//!    would normally write. With no property they fall back to the server's
//!    built-in black `left_ptr`. We write the property ourselves.
//! 2. X11 windows created without a cursor attribute inherit the root
//!    window's cursor, which XWayland initializes to that same built-in
//!    black arrow. We replace it with our rasterized Lantern arrow via
//!    RENDER `CreateCursor`, retained server-side with `RetainPermanent`
//!    (the `xsetroot` trick) so it survives this short-lived connection.
//!
//! Both mirror the startup-time cursor settings, like the env vars do — a
//! live cursor-theme change lands on the next session.
//!
//! Implemented with x11rb (already in-tree via Smithay) over a throwaway
//! connection — no xrdb/xsetroot binaries required.

use x11rb::connection::Connection;
use x11rb::protocol::render::{self, ConnectionExt as _};
use x11rb::protocol::xproto::{self, ConnectionExt as _};
use x11rb::wrapper::ConnectionExt as _;

/// Best-effort: a failure here means X11 apps see a default cursor, not a
/// broken session — warn and move on.
pub fn apply(display_number: u32) {
    // (named `display_name` because `display` collides with tracing's
    // shorthand helper inside its macros)
    let display_name = format!(":{display_number}");
    if let Err(err) = apply_inner(&display_name) {
        tracing::warn!("X11 cursor defaults failed on {}: {}", display_name, err);
    }
}

fn apply_inner(display: &str) -> Result<(), Box<dyn std::error::Error>> {
    let (conn, screen_num) = x11rb::connect(Some(display))?;
    let root = conn.setup().roots[screen_num].root;

    let theme = std::env::var("XCURSOR_THEME").unwrap_or_else(|_| "Lantern".to_string());
    let size = std::env::var("XCURSOR_SIZE")
        .ok()
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or_else(|| {
            crate::input::read_input_setting_f64("cursor_size", 24.0).round() as u32
        });

    set_resource_manager(&conn, root, &theme, size)?;
    if let Err(err) = set_root_cursor(&conn, root, size) {
        tracing::warn!("X11 root cursor setup failed: {err}");
    }

    tracing::info!("X11 cursor defaults applied (theme={theme}, size={size})");
    Ok(())
}

/// Write the `RESOURCE_MANAGER` root property (what `xrdb -merge` would do).
/// Replace is safe: nothing else on a Lantern session writes this property.
/// `Xcursor.theme_core: true` additionally themes the legacy X core font
/// cursors some toolkits still create.
fn set_resource_manager(
    conn: &impl Connection,
    root: xproto::Window,
    theme: &str,
    size: u32,
) -> Result<(), Box<dyn std::error::Error>> {
    let value =
        format!("Xcursor.theme:\t{theme}\nXcursor.size:\t{size}\nXcursor.theme_core:\ttrue\n");
    conn.change_property8(
        xproto::PropMode::REPLACE,
        root,
        xproto::AtomEnum::RESOURCE_MANAGER,
        xproto::AtomEnum::STRING,
        value.as_bytes(),
    )?
    .check()?;
    Ok(())
}

/// Build an ARGB32 cursor from our rasterized arrow and set it on the root
/// window, so cursor-less X11 windows inherit Lantern instead of the
/// server's black default.
fn set_root_cursor(
    conn: &impl Connection,
    root: xproto::Window,
    size: u32,
) -> Result<(), Box<dyn std::error::Error>> {
    let img = crate::xcursor_export::rasterize_default_arrow(size)
        .ok_or("could not rasterize arrow cursor")?;

    // RENDER >= 0.5 for CreateCursor; XWayland ships far newer, but negotiate
    // per protocol anyway.
    conn.render_query_version(0, 8)?.reply()?;

    let formats = conn.render_query_pict_formats()?.reply()?;
    let argb32 = formats
        .formats
        .iter()
        .find(|f| {
            f.type_ == render::PictType::DIRECT
                && f.depth == 32
                && f.direct.alpha_shift == 24
                && f.direct.alpha_mask == 0xff
                && f.direct.red_shift == 16
                && f.direct.green_shift == 8
                && f.direct.blue_shift == 0
        })
        .ok_or("server has no ARGB32 picture format")?;

    let pixmap = conn.generate_id()?;
    conn.create_pixmap(32, pixmap, root, img.width as u16, img.height as u16)?;
    let gc = conn.generate_id()?;
    conn.create_gc(gc, pixmap, &xproto::CreateGCAux::new())?;

    // Our pixels are premultiplied u32 ARGB stored little-endian (BGRA bytes),
    // exactly what an LSB-first server expects for Z_PIXMAP; swap for MSB.
    let swapped;
    let data: &[u8] = if conn.setup().image_byte_order == xproto::ImageOrder::MSB_FIRST {
        swapped = img
            .pixels
            .chunks_exact(4)
            .flat_map(|c| [c[3], c[2], c[1], c[0]])
            .collect::<Vec<u8>>();
        &swapped
    } else {
        &img.pixels
    };
    conn.put_image(
        xproto::ImageFormat::Z_PIXMAP,
        pixmap,
        gc,
        img.width as u16,
        img.height as u16,
        0,
        0,
        0,
        32,
        data,
    )?
    .check()?;

    let picture = conn.generate_id()?;
    conn.render_create_picture(picture, pixmap, argb32.id, &render::CreatePictureAux::new())?;
    let cursor = conn.generate_id()?;
    conn.render_create_cursor(
        cursor,
        picture,
        img.xhot.min(img.width.saturating_sub(1)) as u16,
        img.yhot.min(img.height.saturating_sub(1)) as u16,
    )?;
    conn.change_window_attributes(
        root,
        &xproto::ChangeWindowAttributesAux::new().cursor(cursor),
    )?
    .check()?;

    // Free the scaffolding; the cursor itself must outlive this connection.
    conn.render_free_picture(picture)?;
    conn.free_pixmap(pixmap)?;
    conn.free_gc(gc)?;
    conn.set_close_down_mode(xproto::CloseDown::RETAIN_PERMANENT)?
        .check()?;
    Ok(())
}

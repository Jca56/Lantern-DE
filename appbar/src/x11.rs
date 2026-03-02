use x11rb::connection::Connection;
use x11rb::protocol::xproto::{self, ConnectionExt as _};
use x11rb::wrapper::ConnectionExt as _;

use crate::config::BarPosition;

// ── Screen geometry ──────────────────────────────────────────────────────────

pub struct ScreenGeometry {
    pub width: f32,
    pub height: f32,
    pub x_offset: f32,
}

/// Get primary screen geometry via xrandr (called once at startup)
pub fn get_primary_screen() -> ScreenGeometry {
    if let Ok(output) = std::process::Command::new("xrandr")
        .arg("--query")
        .output()
    {
        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines() {
            if line.contains(" connected primary") {
                if let Some(geo) = parse_xrandr_line(line) {
                    return geo;
                }
            }
        }
    }
    ScreenGeometry {
        width: 1920.0,
        height: 1080.0,
        x_offset: 0.0,
    }
}

fn parse_xrandr_line(line: &str) -> Option<ScreenGeometry> {
    for part in line.split_whitespace() {
        if let Some(x_idx) = part.find('x') {
            let w_str = &part[..x_idx];
            let rest = &part[x_idx + 1..];
            let mut nums = rest.split('+');
            let h_str = nums.next()?;
            let x_off_str = nums.next().unwrap_or("0");
            if let (Ok(w), Ok(h), Ok(x_off)) = (
                w_str.parse::<f32>(),
                h_str.parse::<f32>(),
                x_off_str.parse::<f32>(),
            ) {
                return Some(ScreenGeometry {
                    width: w,
                    height: h,
                    x_offset: x_off,
                });
            }
        }
    }
    None
}

// ── X11 dock hints ───────────────────────────────────────────────────────────

pub fn apply_x11_dock_hints(
    bar_height: u32,
    screen_height: u32,
    screen_width: u32,
    screen_x: u32,
    position: BarPosition,
) -> Result<(), Box<dyn std::error::Error>> {
    let (conn, screen_num) = x11rb::connect(None)?;
    let screen = &conn.setup().roots[screen_num];

    let wm_type = conn.intern_atom(false, b"_NET_WM_WINDOW_TYPE")?.reply()?.atom;
    let wm_type_dock = conn
        .intern_atom(false, b"_NET_WM_WINDOW_TYPE_DOCK")?
        .reply()?
        .atom;
    let wm_strut = conn.intern_atom(false, b"_NET_WM_STRUT")?.reply()?.atom;
    let wm_strut_partial = conn
        .intern_atom(false, b"_NET_WM_STRUT_PARTIAL")?
        .reply()?
        .atom;
    let wm_state = conn
        .intern_atom(false, b"_NET_WM_STATE")?
        .reply()?
        .atom;
    let skip_taskbar = conn
        .intern_atom(false, b"_NET_WM_STATE_SKIP_TASKBAR")?
        .reply()?
        .atom;
    let skip_pager = conn
        .intern_atom(false, b"_NET_WM_STATE_SKIP_PAGER")?
        .reply()?
        .atom;
    let wm_sticky = conn
        .intern_atom(false, b"_NET_WM_STATE_STICKY")?
        .reply()?
        .atom;

    let root = screen.root;
    if let Some(win) = find_window_by_name(&conn, root, "Fox Appbar")? {
        conn.change_property32(
            xproto::PropMode::REPLACE,
            win,
            wm_type,
            xproto::AtomEnum::ATOM,
            &[wm_type_dock],
        )?;

        conn.change_property32(
            xproto::PropMode::REPLACE,
            win,
            wm_state,
            xproto::AtomEnum::ATOM,
            &[skip_taskbar, skip_pager, wm_sticky],
        )?;

        let y = match position {
            BarPosition::Top => 0,
            BarPosition::Bottom => (screen_height - bar_height) as i32,
        };

        conn.configure_window(
            win,
            &xproto::ConfigureWindowAux::new()
                .x(screen_x as i32)
                .y(y)
                .width(screen_width)
                .height(bar_height)
                .stack_mode(xproto::StackMode::ABOVE),
        )?;

        let (strut, strut_partial) = match position {
            BarPosition::Top => (
                [0u32, 0, bar_height, 0],
                [
                    0u32,
                    0,
                    bar_height,
                    0,
                    0,
                    0,
                    0,
                    0,
                    screen_x,
                    screen_x + screen_width - 1,
                    0,
                    0,
                ],
            ),
            BarPosition::Bottom => (
                [0u32, 0, 0, bar_height],
                [
                    0u32,
                    0,
                    0,
                    bar_height,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    screen_x,
                    screen_x + screen_width - 1,
                ],
            ),
        };

        conn.change_property32(
            xproto::PropMode::REPLACE,
            win,
            wm_strut,
            xproto::AtomEnum::CARDINAL,
            &strut,
        )?;

        conn.change_property32(
            xproto::PropMode::REPLACE,
            win,
            wm_strut_partial,
            xproto::AtomEnum::CARDINAL,
            &strut_partial,
        )?;

        conn.flush()?;
        eprintln!(
            "X11 dock hints applied to window 0x{:x} ({:?})",
            win, position
        );
    } else {
        eprintln!("Could not find Fox Appbar window");
    }

    Ok(())
}

fn find_window_by_name(
    conn: &impl Connection,
    root: u32,
    target: &str,
) -> Result<Option<u32>, Box<dyn std::error::Error>> {
    let tree = conn.query_tree(root)?.reply()?;
    for child in tree.children {
        if let Ok(reply) = conn
            .get_property(
                false,
                child,
                xproto::AtomEnum::WM_NAME,
                xproto::AtomEnum::STRING,
                0,
                1024,
            )?
            .reply()
        {
            if let Ok(name) = std::str::from_utf8(&reply.value) {
                if name == target {
                    return Ok(Some(child));
                }
            }
        }
        if let Some(found) = find_window_by_name(conn, child, target)? {
            return Ok(Some(found));
        }
    }
    Ok(None)
}

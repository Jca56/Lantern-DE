//! Direct ICCCM `WM_STATE` writes for XWayland windows.
//!
//! Why this exists: Smithay's `X11Surface::set_mapped(false)` is the only API
//! that writes `WM_STATE = IconicState`, but it ALSO unmaps the WM frame. That
//! unrealizes the client window inside it, and XWayland then destroys the
//! window's `wl_surface` (`xwl_unrealize_window` → `xwl_window_dispose` →
//! `wl_surface_destroy`) and hands us a brand-new surface on re-map. Every
//! surface-keyed table in the compositor (minimized list, fullscreen state,
//! MRU, workspaces, foreign-toplevel handles…) goes stale — that was the
//! "3 duplicate Skyrim windows" regression of 2026-05-30.
//!
//! Proton/Wine games, however, genuinely need the `WM_STATE` transition: Wine
//! arms a serial when it calls `XIconifyWindow` and refuses to sync the Win32
//! window state until the WM answers with `IconicState`. Without the answer the
//! game sits minimized-on-the-Win32-side forever (DXVK stops presenting →
//! black), and the only way it ever came back was Wine's "mismatch" recovery
//! when we wrote `NormalState` on restore.
//!
//! So we write the property ourselves over a second, tiny x11rb connection and
//! leave the frame mapped. The client window stays realized, the `wl_surface`
//! stays alive, and the game still sees a textbook iconify/deiconify.
//! (`_NET_WM_STATE_HIDDEN`, which SDL2 clients key off instead, is set through
//! Smithay's `set_suspended` by the caller.)

use x11rb::connection::Connection;
use x11rb::protocol::xproto::{ConnectionExt as _, PropMode};
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;

/// ICCCM `WM_STATE` values.
const NORMAL_STATE: u32 = 1;
const ICONIC_STATE: u32 = 3;

/// A persistent side-connection to XWayland used only for `WM_STATE` writes.
pub struct X11WmStateWriter {
    conn: RustConnection,
    wm_state: u32,
}

impl X11WmStateWriter {
    /// Connect to `:<display_number>` and intern the `WM_STATE` atom.
    pub fn connect(display_number: u32) -> Result<Self, Box<dyn std::error::Error>> {
        let display = format!(":{display_number}");
        let (conn, _screen) = x11rb::connect(Some(&display))?;
        let wm_state = conn.intern_atom(false, b"WM_STATE")?.reply()?.atom;
        Ok(Self { conn, wm_state })
    }

    /// Write `WM_STATE` on a client window: `IconicState` when `iconic`,
    /// `NormalState` otherwise. Round-trips (`check`) so a dead window
    /// surfaces as an error instead of silently queueing.
    pub fn set_iconic(&self, window: u32, iconic: bool) -> Result<(), Box<dyn std::error::Error>> {
        let state = if iconic { ICONIC_STATE } else { NORMAL_STATE };
        // [state, icon_window]; icon window is always None for us.
        self.conn
            .change_property32(
                PropMode::REPLACE,
                window,
                self.wm_state,
                self.wm_state,
                &[state, 0],
            )?
            .check()?;
        self.conn.flush()?;
        Ok(())
    }
}

//! System tray widget — renders SNI icons in the bar and handles clicks.

use std::collections::HashMap;
use std::sync::mpsc;

use lntrn_render::{GpuContext, GpuTexture, Painter, Rect, TextRenderer, TexturePass, TextureDraw};
use lntrn_ui::gpu::{FoxPalette, InteractionContext};

use crate::sni::{SniHost, SniItem, TrayCommand};

const TRAY_ZONE_BASE: u32 = 0xCC_0000;
const ICON_GAP: f32 = 8.0;

// ── Tray icon with cached GPU texture ───────────────────────────────────────

struct TrayIcon {
    bus_name: String,
    obj_path: String,
    id: String,
    status: String,
    texture: Option<GpuTexture>,
}

// ── Public widget ───────────────────────────────────────────────────────────

pub struct SystemTray {
    icons: Vec<TrayIcon>,
    cmd_tx: mpsc::Sender<TrayCommand>,
    item_rx: mpsc::Receiver<Vec<SniItem>>,
    /// Track which items have had their textures uploaded.
    uploaded: HashMap<String, bool>,
}

impl SystemTray {
    /// Spawn the D-Bus thread and return the tray widget.
    pub fn start() -> Option<Self> {
        let (item_tx, item_rx) = mpsc::channel::<Vec<SniItem>>();
        let (cmd_tx, cmd_rx) = mpsc::channel::<TrayCommand>();

        std::thread::Builder::new()
            .name("sni-dbus".into())
            .spawn(move || {
                dbus_thread(item_tx, cmd_rx);
            })
            .ok()?;

        Some(Self {
            icons: Vec::new(),
            cmd_tx,
            item_rx,
            uploaded: HashMap::new(),
        })
    }

    /// Poll for updates from the D-Bus thread and upload new textures.
    pub fn poll(&mut self, tex_pass: &TexturePass, gpu: &GpuContext) {
        // Check for item list updates
        while let Ok(items) = self.item_rx.try_recv() {
            // Rebuild icon list
            self.icons.clear();
            self.uploaded.clear();
            for item in &items {
                let mut icon = TrayIcon {
                    bus_name: item.bus_name.clone(),
                    obj_path: item.obj_path.clone(),
                    id: item.id.clone(),
                    status: item.status.clone(),
                    texture: None,
                };
                // Upload pixmap if available
                if let Some(pm) = &item.icon_pixmap {
                    if pm.width > 0 && pm.height > 0 {
                        icon.texture = Some(tex_pass.upload(gpu, &pm.rgba, pm.width, pm.height));
                        self.uploaded.insert(item.bus_name.clone(), true);
                    }
                }
                self.icons.push(icon);
            }
        }
    }

    /// Draw tray icons right-aligned in the bar, to the left of the clock.
    /// Returns the total width consumed by the tray (in physical pixels).
    pub fn draw(
        &self,
        painter: &mut Painter,
        _text: &mut TextRenderer,
        ix: &mut InteractionContext,
        palette: &FoxPalette,
        // Bar visual rect in physical pixels
        bar_x: f32,
        bar_y: f32,
        bar_w: f32,
        bar_h: f32,
        // How much space the clock uses from the right
        clock_width: f32,
        scale: f32,
        screen_w: u32,
        screen_h: u32,
    ) -> (f32, Vec<TextureDraw<'_>>) {
        let icon_size = (bar_h * 0.45).max(24.0);
        let gap = ICON_GAP * scale;
        let visible: Vec<usize> = self.icons.iter().enumerate()
            .filter(|(_, ic)| ic.status != "Passive")
            .map(|(i, _)| i)
            .collect();

        if visible.is_empty() {
            return (0.0, Vec::new());
        }

        let total_w = visible.len() as f32 * (icon_size + gap) - gap;
        // Position: right side of bar, left of clock
        let start_x = bar_x + bar_w - clock_width - total_w - gap * 2.0;
        let center_y = bar_y + (bar_h - icon_size) / 2.0;

        let mut tex_draws = Vec::new();
        for (slot, &idx) in visible.iter().enumerate() {
            let icon = &self.icons[idx];
            let x = start_x + slot as f32 * (icon_size + gap);
            let rect = Rect::new(x, center_y, icon_size, icon_size);
            let zone_id = TRAY_ZONE_BASE + idx as u32;

            ix.add_zone(zone_id, rect);
            let hovered = ix.is_hovered(&rect);

            // Hover highlight
            if hovered {
                let pad = 4.0 * scale;
                let hover_rect = Rect::new(
                    x - pad, center_y - pad,
                    icon_size + pad * 2.0, icon_size + pad * 2.0,
                );
                painter.rect_filled(hover_rect, 8.0 * scale, palette.surface_2);
            }

            // Draw icon texture or fallback circle
            if let Some(tex) = &icon.texture {
                tex_draws.push(TextureDraw::new(tex, x, center_y, icon_size, icon_size));
            } else {
                // Fallback: colored circle with first letter
                let circle_r = icon_size / 2.0;
                let cx = x + circle_r;
                let cy = center_y + circle_r;
                painter.rect_filled(
                    Rect::new(cx - circle_r, cy - circle_r, icon_size, icon_size),
                    circle_r, palette.accent,
                );
            }
        }

        (total_w + gap * 2.0, tex_draws)
    }

    /// Check if a click landed on a tray icon. If so, send Activate.
    pub fn handle_click(&self, ix: &InteractionContext, phys_cx: f32, phys_cy: f32) {
        if let Some(zone) = ix.zone_at(phys_cx, phys_cy) {
            if zone >= TRAY_ZONE_BASE && zone < TRAY_ZONE_BASE + 256 {
                let idx = (zone - TRAY_ZONE_BASE) as usize;
                if let Some(icon) = self.icons.get(idx) {
                    let _ = self.cmd_tx.send(TrayCommand::Activate {
                        bus_name: icon.bus_name.clone(),
                        obj_path: icon.obj_path.clone(),
                        x: 0, y: 0,
                    });
                }
            }
        }
    }

    pub fn is_empty(&self) -> bool {
        self.icons.iter().all(|i| i.status == "Passive")
    }
}

// ── D-Bus background thread ─────────────────────────────────────────────────

fn dbus_thread(
    item_tx: mpsc::Sender<Vec<SniItem>>,
    cmd_rx: mpsc::Receiver<TrayCommand>,
) {
    let mut host = match SniHost::connect() {
        Ok(h) => h,
        Err(e) => {
            tracing::warn!("SNI host failed to start: {e}");
            return;
        }
    };
    tracing::info!("SNI host thread running");

    loop {
        // Process commands from render thread
        while let Ok(cmd) = cmd_rx.try_recv() {
            match cmd {
                TrayCommand::Activate { bus_name, obj_path, x, y } => {
                    host.activate(&bus_name, &obj_path, x, y);
                }
            }
        }

        // Poll D-Bus for messages
        let changed = host.poll();
        if changed {
            let _ = item_tx.send(host.items().to_vec());
        }

        // Sleep briefly to avoid busy-spinning (D-Bus socket is non-blocking)
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

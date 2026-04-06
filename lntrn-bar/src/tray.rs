//! System tray widget — renders SNI icons in the bar and handles clicks.

use std::collections::HashMap;
use std::sync::mpsc;

use lntrn_render::{GpuContext, GpuTexture, Painter, Rect, TextRenderer, TexturePass, TextureDraw};
use lntrn_ui::gpu::{FoxPalette, InteractionContext, MenuItem};

use crate::sni::{SniHost, SniItem, TrayCommand};

const TRAY_ZONE_BASE: u32 = 0xCC_0000;
const ICON_GAP: f32 = 8.0;

// ── Tray icon with cached GPU texture ───────────────────────────────────────

struct TrayIcon {
    bus_name: String,
    obj_path: String,
    id: String,
    status: String,
    menu_path: Option<String>,
    texture: Option<GpuTexture>,
}

// ── Events sent from D-Bus thread to render thread ──────────────────────────

pub enum TrayEvent {
    MenuReady { bus_name: String, menu_path: String, items: Vec<MenuItem> },
}

// ── Public widget ───────────────────────────────────────────────────────────

pub struct SystemTray {
    icons: Vec<TrayIcon>,
    cmd_tx: mpsc::Sender<TrayCommand>,
    item_rx: mpsc::Receiver<Vec<SniItem>>,
    event_rx: mpsc::Receiver<TrayEvent>,
    /// Track which items have had their textures uploaded.
    uploaded: HashMap<String, bool>,
}

impl SystemTray {
    /// Spawn the D-Bus thread and return the tray widget.
    pub fn start() -> Option<Self> {
        let (item_tx, item_rx) = mpsc::channel::<Vec<SniItem>>();
        let (cmd_tx, cmd_rx) = mpsc::channel::<TrayCommand>();
        let (event_tx, event_rx) = mpsc::channel::<TrayEvent>();

        std::thread::Builder::new()
            .name("sni-dbus".into())
            .spawn(move || {
                dbus_thread(item_tx, cmd_rx, event_tx);
            })
            .ok()?;

        Some(Self {
            icons: Vec::new(),
            cmd_tx,
            item_rx,
            event_rx,
            uploaded: HashMap::new(),
        })
    }

    /// Poll for updates from the D-Bus thread and upload new textures.
    pub fn poll(&mut self, tex_pass: &TexturePass, gpu: &GpuContext) {
        while let Ok(items) = self.item_rx.try_recv() {
            self.icons.clear();
            self.uploaded.clear();
            for item in &items {
                let mut icon = TrayIcon {
                    bus_name: item.bus_name.clone(),
                    obj_path: item.obj_path.clone(),
                    id: item.id.clone(),
                    status: item.status.clone(),
                    menu_path: item.menu_path.clone(),
                    texture: None,
                };
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

    /// Check for menu events from the D-Bus thread.
    pub fn poll_event(&self) -> Option<TrayEvent> {
        self.event_rx.try_recv().ok()
    }

    /// Draw tray icons right-aligned in the bar, to the left of the clock.
    /// Returns the total width consumed by the tray (in physical pixels).
    pub fn draw(
        &self,
        painter: &mut Painter,
        _text: &mut TextRenderer,
        ix: &mut InteractionContext,
        palette: &FoxPalette,
        bar_x: f32,
        bar_y: f32,
        bar_w: f32,
        bar_h: f32,
        clock_width: f32,
        scale: f32,
        screen_w: u32,
        screen_h: u32,
    ) -> (f32, Vec<TextureDraw<'_>>) {
        let icon_size = (bar_h * 0.35).max(20.0);
        let gap = ICON_GAP * scale;
        let visible: Vec<usize> = self.icons.iter().enumerate()
            .filter(|(_, ic)| ic.status != "Passive")
            .map(|(i, _)| i)
            .collect();

        if visible.is_empty() {
            return (0.0, Vec::new());
        }

        let total_w = visible.len() as f32 * (icon_size + gap) - gap;
        let start_x = bar_x + bar_w - clock_width - total_w - gap * 2.0;
        let center_y = bar_y + (bar_h - icon_size) / 2.0;

        let _ = (screen_w, screen_h);
        let mut tex_draws = Vec::new();
        for (slot, &idx) in visible.iter().enumerate() {
            let icon = &self.icons[idx];
            let x = start_x + slot as f32 * (icon_size + gap);
            let rect = Rect::new(x, center_y, icon_size, icon_size);
            let zone_id = TRAY_ZONE_BASE + idx as u32;

            ix.add_zone(zone_id, rect);
            let hovered = ix.is_hovered(&rect);

            if hovered {
                let pad = 4.0 * scale;
                let hover_rect = Rect::new(
                    x - pad, center_y - pad,
                    icon_size + pad * 2.0, icon_size + pad * 2.0,
                );
                painter.rect_filled(hover_rect, 8.0 * scale, palette.surface_2);
            }

            if let Some(tex) = &icon.texture {
                tex_draws.push(TextureDraw::new(tex, x, center_y, icon_size, icon_size));
            } else {
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

    /// Handle a left-click on a tray icon.
    /// Returns true if a menu was requested (will arrive via poll_event).
    pub fn handle_click(&self, ix: &InteractionContext, phys_cx: f32, phys_cy: f32) -> bool {
        if let Some(zone) = ix.zone_at(phys_cx, phys_cy) {
            if zone >= TRAY_ZONE_BASE && zone < TRAY_ZONE_BASE + 256 {
                let idx = (zone - TRAY_ZONE_BASE) as usize;
                if let Some(icon) = self.icons.get(idx) {
                    if let Some(menu_path) = &icon.menu_path {
                        tracing::info!(
                            bus = %icon.bus_name, menu = %menu_path,
                            "tray icon clicked → requesting dbusmenu"
                        );
                        let _ = self.cmd_tx.send(TrayCommand::GetMenu {
                            bus_name: icon.bus_name.clone(),
                            menu_path: menu_path.clone(),
                        });
                        return true;
                    } else {
                        tracing::info!(
                            bus = %icon.bus_name, path = %icon.obj_path,
                            "tray icon clicked → sending Activate"
                        );
                        let _ = self.cmd_tx.send(TrayCommand::Activate {
                            bus_name: icon.bus_name.clone(),
                            obj_path: icon.obj_path.clone(),
                            x: 0, y: 0,
                        });
                    }
                }
            }
        }
        false
    }

    /// Send a dbusmenu item click event back to the D-Bus thread.
    pub fn send_menu_click(&self, bus_name: &str, menu_path: &str, item_id: i32) {
        let _ = self.cmd_tx.send(TrayCommand::MenuItemClicked {
            bus_name: bus_name.to_string(),
            menu_path: menu_path.to_string(),
            item_id,
        });
    }
}

// ── D-Bus background thread ─────────────────────────────────────────────────

fn dbus_thread(
    item_tx: mpsc::Sender<Vec<SniItem>>,
    cmd_rx: mpsc::Receiver<TrayCommand>,
    event_tx: mpsc::Sender<TrayEvent>,
) {
    let mut host = match SniHost::connect() {
        Ok(h) => h,
        Err(e) => {
            tracing::warn!("SNI host failed to start: {e}");
            return;
        }
    };
    tracing::info!("SNI host thread running");

    let palette = lntrn_ui::gpu::FoxPalette::dark();
    let fd = host.dbus_fd();

    loop {
        let mut pfd = libc::pollfd { fd, events: libc::POLLIN, revents: 0 };
        let poll_ret = unsafe { libc::poll(&mut pfd, 1, 100) };
        // If poll returned an error or the fd has POLLERR/POLLHUP/POLLNVAL,
        // it will return immediately every iteration — sleep to avoid a spin.
        if poll_ret < 0 || pfd.revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL) != 0 {
            std::thread::sleep(std::time::Duration::from_millis(100));
        }

        // Process commands from render thread
        while let Ok(cmd) = cmd_rx.try_recv() {
            match cmd {
                TrayCommand::Activate { bus_name, obj_path, x, y } => {
                    tracing::info!(bus = %bus_name, path = %obj_path, "D-Bus Activate sent");
                    host.activate(&bus_name, &obj_path, x, y);
                }
                TrayCommand::GetMenu { bus_name, menu_path } => {
                    tracing::info!(bus = %bus_name, menu = %menu_path, "requesting dbusmenu layout");
                    host.get_menu_layout(&bus_name, &menu_path);
                }
                TrayCommand::MenuItemClicked { bus_name, menu_path, item_id } => {
                    tracing::info!(bus = %bus_name, item_id, "sending dbusmenu Event");
                    host.send_menu_event(&bus_name, &menu_path, item_id);
                }
            }
        }

        // Poll D-Bus for messages
        let changed = host.poll();
        if changed {
            let _ = item_tx.send(host.items().to_vec());
        }

        // Check if a menu layout is ready
        if let Some((bus_name, menu_path, dbus_items)) = host.menu_ready.take() {
            let items = crate::dbusmenu::to_menu_items(&dbus_items, &palette);
            let _ = event_tx.send(TrayEvent::MenuReady { bus_name, menu_path, items });
        }
    }
}

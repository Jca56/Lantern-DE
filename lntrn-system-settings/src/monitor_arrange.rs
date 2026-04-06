/// Monitor arrangement: drag-to-place display layout editor.

use lntrn_render::{Painter, Rect, TextRenderer};
use lntrn_ui::gpu::{FoxPalette, InteractionContext};

use crate::config::MonitorEntry;
use crate::wayland::OutputInfo;

const ZONE_MON_BASE: u32 = 1000;
const MAX_MONITORS: u32 = 8;
const CANVAS_H: f32 = 240.0;
const PAD: f32 = 24.0;
const LABEL_SIZE: f32 = 16.0;
const NAME_SIZE: f32 = 18.0;
const RES_SIZE: f32 = 14.0;

/// Compositor output scale (must match lntrn-compositor's LANTERN_OUTPUT_SCALE).
const OUTPUT_SCALE: f32 = 1.25;

/// Monitor rectangle in the arrangement canvas (logical UI coords).
#[derive(Clone)]
struct MonRect {
    name: String,
    /// Physical resolution for display label.
    res_w: i32,
    res_h: i32,
    /// Logical position in "output space" (from config or auto).
    out_x: i32,
    out_y: i32,
    /// Logical size in "output space" (physical / scale).
    out_w: i32,
    out_h: i32,
}

/// Persists across frames.
pub struct MonitorArrangeState {
    /// The rectangles being arranged (synced from outputs + config).
    rects: Vec<MonRect>,
    /// Which monitor is being dragged (-1 = none).
    dragging: i32,
    /// Drag offset from the monitor's top-left corner.
    drag_offset_x: f32,
    drag_offset_y: f32,
    /// Previous cursor position for delta-based drag.
    last_cursor_x: f32,
    last_cursor_y: f32,
    /// Scale factor: output-space → canvas-space.
    view_scale: f32,
    /// Canvas origin offset for centering.
    canvas_offset_x: f32,
    canvas_offset_y: f32,
    /// Whether rects need to be synced from outputs.
    needs_sync: bool,
    /// Dirty flag — user has dragged something.
    pub dirty: bool,
    /// Selected monitor index (for per-monitor settings).
    pub selected: Option<usize>,
    /// True if the last click was a drag (suppress selection on release).
    drag_moved: bool,
}

impl MonitorArrangeState {
    pub fn new() -> Self {
        Self {
            rects: Vec::new(),
            dragging: -1,
            drag_offset_x: 0.0,
            drag_offset_y: 0.0,
            last_cursor_x: 0.0,
            last_cursor_y: 0.0,
            view_scale: 1.0,
            canvas_offset_x: 0.0,
            canvas_offset_y: 0.0,
            needs_sync: true,
            dirty: false,
            selected: None,
            drag_moved: false,
        }
    }

    /// Sync monitor rectangles from live wl_output data + saved config.
    pub fn sync_from_outputs(&mut self, outputs: &[(u32, OutputInfo)], config: &[MonitorEntry]) {
        if !self.needs_sync || outputs.is_empty() {
            return;
        }
        self.needs_sync = false;
        self.rects.clear();

        for (_, info) in outputs {
            if info.name.is_empty() || info.width == 0 {
                continue;
            }
            // Use fractional compositor scale, not wl_output integer scale
            let logical_w = (info.width as f32 / OUTPUT_SCALE).round() as i32;
            let logical_h = (info.height as f32 / OUTPUT_SCALE).round() as i32;

            // Use config position if available, otherwise use compositor-reported position
            let (ox, oy) = if let Some(cfg) = config.iter().find(|c| c.name == info.name) {
                (cfg.x, cfg.y)
            } else {
                (info.x, info.y)
            };

            self.rects.push(MonRect {
                name: info.name.clone(),
                res_w: info.width,
                res_h: info.height,
                out_x: ox,
                out_y: oy,
                out_w: logical_w,
                out_h: logical_h,
            });
        }
    }

    /// Export current arrangement as config entries.
    pub fn to_config(&self) -> Vec<MonitorEntry> {
        self.rects.iter().map(|r| MonitorEntry {
            name: r.name.clone(),
            x: r.out_x,
            y: r.out_y,
            resolution: String::new(),
            refresh_rate: String::new(),
            scale: 1.25,
            wallpaper: String::new(),
        }).collect()
    }

    /// Force re-sync on next frame.
    pub fn request_sync(&mut self) {
        self.needs_sync = true;
    }

    /// Get the name of the selected output (auto-selects if only one).
    pub fn selected_output_name(&mut self) -> Option<String> {
        if self.rects.len() == 1 {
            self.selected = Some(0);
        }
        let idx = self.selected?;
        self.rects.get(idx).map(|r| r.name.clone())
    }
}

/// Compute the view scale and canvas offset to center all monitors.
fn compute_view(rects: &[MonRect], canvas_w: f32, canvas_h: f32, s: f32) -> (f32, f32, f32) {
    if rects.is_empty() {
        return (0.1, 0.0, 0.0);
    }

    let mut min_x = i32::MAX;
    let mut min_y = i32::MAX;
    let mut max_x = i32::MIN;
    let mut max_y = i32::MIN;
    for r in rects {
        min_x = min_x.min(r.out_x);
        min_y = min_y.min(r.out_y);
        max_x = max_x.max(r.out_x + r.out_w);
        max_y = max_y.max(r.out_y + r.out_h);
    }

    let total_w = (max_x - min_x) as f32;
    let total_h = (max_y - min_y) as f32;
    let margin = 60.0 * s;
    let avail_w = (canvas_w - margin * 2.0).max(100.0);
    let avail_h = (canvas_h - margin * 2.0).max(100.0);

    let vs = (avail_w / total_w).min(avail_h / total_h).min(0.5 * s);

    let cx = canvas_w / 2.0 - (min_x as f32 + total_w / 2.0) * vs;
    let cy = canvas_h / 2.0 - (min_y as f32 + total_h / 2.0) * vs;

    (vs, cx, cy)
}

/// Draw the monitor arrangement area. Returns the height consumed.
pub fn draw_monitor_arrange(
    mas: &mut MonitorArrangeState,
    outputs: &[(u32, OutputInfo)],
    config: &[MonitorEntry],
    painter: &mut Painter,
    text: &mut TextRenderer,
    ix: &mut InteractionContext,
    fox: &FoxPalette,
    x: f32,
    y: f32,
    w: f32,
    s: f32,
    sw: u32,
    sh: u32,
) -> f32 {
    let pad = PAD * s;
    let canvas_h = CANVAS_H * s;
    let label_sz = LABEL_SIZE * s;
    let name_sz = NAME_SIZE * s;
    let res_sz = RES_SIZE * s;

    mas.sync_from_outputs(outputs, config);

    // Section label
    text.queue("Displays", label_sz, x + pad, y + pad, fox.text, w, sw, sh);
    let canvas_y = y + pad + label_sz + 12.0 * s;
    let canvas_w = w - pad * 2.0;

    // Canvas background
    let canvas_rect = Rect::new(x + pad, canvas_y, canvas_w, canvas_h);
    painter.rect_filled(canvas_rect, 8.0 * s, fox.surface);

    // Compute view transform
    let (vs, cx, cy) = compute_view(&mas.rects, canvas_w, canvas_h, s);
    mas.view_scale = vs;
    mas.canvas_offset_x = x + pad + cx;
    mas.canvas_offset_y = canvas_y + cy;

    // Draw monitor rectangles
    for (i, r) in mas.rects.iter().enumerate() {
        if i as u32 >= MAX_MONITORS { break; }

        let rx = mas.canvas_offset_x + r.out_x as f32 * vs;
        let ry = mas.canvas_offset_y + r.out_y as f32 * vs;
        let rw = r.out_w as f32 * vs;
        let rh = r.out_h as f32 * vs;

        let zone_id = ZONE_MON_BASE + i as u32;
        let rect = Rect::new(rx, ry, rw, rh);
        let zone = ix.add_zone(zone_id, rect);
        let is_dragging = mas.dragging == i as i32;

        // Monitor fill + border
        let is_selected = mas.selected == Some(i);
        let fill = if is_dragging {
            fox.accent.with_alpha(0.4)
        } else if is_selected {
            fox.accent.with_alpha(0.15)
        } else if zone.is_hovered() {
            fox.accent.with_alpha(0.25)
        } else {
            fox.bg.with_alpha(0.8)
        };
        let bw = if is_selected { 3.0 * s } else { 2.0 * s };
        let border_color = if is_dragging || is_selected { fox.accent } else { fox.muted };
        let border_rect = Rect::new(rx - bw, ry - bw, rw + bw * 2.0, rh + bw * 2.0);
        painter.rect_filled(border_rect, 6.0 * s, border_color);
        painter.rect_filled(rect, 4.0 * s, fill);

        // Monitor name centered
        let name_w = r.name.len() as f32 * name_sz * 0.6;
        let name_x = rx + (rw - name_w) / 2.0;
        let name_y = ry + rh / 2.0 - name_sz;
        text.queue(&r.name, name_sz, name_x, name_y, fox.text, rw, sw, sh);

        // Resolution below name
        let res_str = format!("{}x{}", r.res_w, r.res_h);
        let res_w = res_str.len() as f32 * res_sz * 0.6;
        let res_x = rx + (rw - res_w) / 2.0;
        let res_y = name_y + name_sz + 4.0 * s;
        text.queue(&res_str, res_sz, res_x, res_y, fox.text_secondary, rw, sw, sh);
    }

    // Instructions text
    let instr_y = canvas_y + canvas_h + 8.0 * s;
    let instr = if mas.rects.len() > 1 {
        "Drag monitors to arrange. Save to apply."
    } else {
        "Single display detected."
    };
    text.queue(instr, res_sz, x + pad, instr_y, fox.text_secondary, canvas_w, sw, sh);

    // Total height consumed
    canvas_y - y + canvas_h + 8.0 * s + res_sz + 16.0 * s
}

/// Handle mouse-down on a monitor rectangle. Returns true if consumed.
pub fn handle_arrange_click(
    mas: &mut MonitorArrangeState,
    zone_id: u32,
    cursor_x: f32,
    cursor_y: f32,
) -> bool {
    if zone_id < ZONE_MON_BASE || zone_id >= ZONE_MON_BASE + MAX_MONITORS {
        return false;
    }
    let idx = (zone_id - ZONE_MON_BASE) as usize;
    if idx >= mas.rects.len() {
        return false;
    }

    mas.dragging = idx as i32;
    mas.drag_moved = false;
    mas.selected = Some(idx);
    let r = &mas.rects[idx];
    let rx = mas.canvas_offset_x + r.out_x as f32 * mas.view_scale;
    let ry = mas.canvas_offset_y + r.out_y as f32 * mas.view_scale;
    mas.drag_offset_x = cursor_x - rx;
    mas.drag_offset_y = cursor_y - ry;
    mas.last_cursor_x = cursor_x;
    mas.last_cursor_y = cursor_y;
    true
}

/// Update drag position. Call on pointer motion while dragging.
pub fn handle_arrange_drag(
    mas: &mut MonitorArrangeState,
    cursor_x: f32,
    cursor_y: f32,
) {
    if mas.dragging < 0 { return; }
    let idx = mas.dragging as usize;
    if idx >= mas.rects.len() { return; }

    // Convert canvas cursor position back to output-space
    let new_rx = cursor_x - mas.drag_offset_x;
    let new_ry = cursor_y - mas.drag_offset_y;

    let out_x = ((new_rx - mas.canvas_offset_x) / mas.view_scale).round() as i32;
    let out_y = ((new_ry - mas.canvas_offset_y) / mas.view_scale).round() as i32;

    mas.rects[idx].out_x = out_x;
    mas.rects[idx].out_y = out_y;
    mas.dirty = true;
    mas.drag_moved = true;
}

/// End drag. Snap to nearest edge of other monitors.
pub fn handle_arrange_release(mas: &mut MonitorArrangeState) {
    if mas.dragging < 0 { return; }
    let idx = mas.dragging as usize;
    mas.dragging = -1;

    if idx >= mas.rects.len() { return; }

    // Snap to edges of other monitors (within 30 logical px threshold)
    let snap_threshold = 30;
    let r = mas.rects[idx].clone();

    let mut best_dx: Option<i32> = None;
    let mut best_dy: Option<i32> = None;

    for (i, other) in mas.rects.iter().enumerate() {
        if i == idx { continue; }

        // Snap horizontally: right edge of other → left edge of dragged
        let snap_right = other.out_x + other.out_w;
        let dx1 = snap_right - r.out_x;
        if dx1.abs() < snap_threshold {
            if best_dx.is_none() || dx1.abs() < best_dx.unwrap().abs() {
                best_dx = Some(dx1);
            }
        }
        // Snap left edge of other → right edge of dragged
        let dx2 = other.out_x - (r.out_x + r.out_w);
        if dx2.abs() < snap_threshold {
            if best_dx.is_none() || dx2.abs() < best_dx.unwrap().abs() {
                best_dx = Some(dx2);
            }
        }

        // Snap vertically: bottom of other → top of dragged
        let snap_bottom = other.out_y + other.out_h;
        let dy1 = snap_bottom - r.out_y;
        if dy1.abs() < snap_threshold {
            if best_dy.is_none() || dy1.abs() < best_dy.unwrap().abs() {
                best_dy = Some(dy1);
            }
        }
        // Snap top of other → bottom of dragged
        let dy2 = other.out_y - (r.out_y + r.out_h);
        if dy2.abs() < snap_threshold {
            if best_dy.is_none() || dy2.abs() < best_dy.unwrap().abs() {
                best_dy = Some(dy2);
            }
        }

        // Vertical alignment: align tops
        let dy_top = other.out_y - r.out_y;
        if dy_top.abs() < snap_threshold {
            if best_dy.is_none() || dy_top.abs() < best_dy.unwrap().abs() {
                best_dy = Some(dy_top);
            }
        }
    }

    if let Some(dx) = best_dx {
        mas.rects[idx].out_x += dx;
    }
    if let Some(dy) = best_dy {
        mas.rects[idx].out_y += dy;
    }
}

/// Check if currently dragging.
pub fn is_dragging(mas: &MonitorArrangeState) -> bool {
    mas.dragging >= 0
}

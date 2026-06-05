/// Monitor arrangement: drag-to-place display layout editor.

use lntrn_render::{Painter, Rect, TextRenderer};
use lntrn_ui::gpu::{FoxPalette, InteractionContext};

use crate::config::MonitorEntry;
use crate::output_manager::{HeadChange, OutputManagerClient};
use crate::wayland::OutputInfo;

const ZONE_MON_BASE: u32 = 1000;
const MAX_MONITORS: u32 = 8;
const CANVAS_H: f32 = 240.0;
const PAD: f32 = 24.0;
const LABEL_SIZE: f32 = 16.0;
const NAME_SIZE: f32 = 18.0;
const RES_SIZE: f32 = 14.0;

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
    ///
    /// `output_mgr` lets us read each head's *actual* compositor scale so the
    /// canvas's logical widths line up with what `set_position` will use.
    /// Hardcoding a scale here causes side-by-side drops to overlap or leave
    /// gaps when the real scale differs.
    pub fn sync_from_outputs(
        &mut self,
        outputs: &[(u32, OutputInfo)],
        config: &[MonitorEntry],
        output_mgr: &OutputManagerClient,
    ) {
        if !self.needs_sync || outputs.is_empty() {
            return;
        }
        self.needs_sync = false;
        self.rects.clear();

        for (_, info) in outputs {
            if info.name.is_empty() || info.width == 0 {
                continue;
            }

            let cfg = config.iter().find(|c| c.name == info.name);

            // Prefer the resolution saved in lantern.toml — that's the user's
            // source of truth. wl_output reports whatever mode the compositor
            // currently has applied, which can differ from the saved config
            // when the compositor hasn't reconciled yet.
            let (res_w, res_h) = cfg
                .and_then(|c| parse_resolution(&c.resolution))
                .unwrap_or((info.width, info.height));

            // Resolve the effective scale: live head.scale wins (reflects what
            // the compositor will actually place outputs at), then config, then
            // a 1.0 fallback. Match against head name, not wl_output name —
            // they should be the same string in practice.
            let scale = output_mgr.heads.iter()
                .find(|h| h.name == info.name)
                .map(|h| h.scale as f32)
                .or_else(|| cfg.map(|c| c.scale))
                .unwrap_or(1.0)
                .max(0.01);

            let logical_w = (res_w as f32 / scale).round() as i32;
            let logical_h = (res_h as f32 / scale).round() as i32;

            // Use config position if available, otherwise use compositor-reported position
            let (ox, oy) = if let Some(c) = cfg {
                (c.x, c.y)
            } else {
                (info.x, info.y)
            };

            self.rects.push(MonRect {
                name: info.name.clone(),
                res_w,
                res_h,
                out_x: ox,
                out_y: oy,
                out_w: logical_w,
                out_h: logical_h,
            });
        }
    }

    /// Build position-only `HeadChange`s for the compositor. One entry per
    /// arrange-canvas rect that matches a known head; mode/scale untouched
    /// (those are handled by monitor_settings on a separate dirty flag).
    pub fn position_changes(&self, output_mgr: &OutputManagerClient) -> Vec<HeadChange> {
        self.rects.iter().filter_map(|r| {
            let head_idx = output_mgr.heads.iter().position(|h| h.name == r.name)?;
            Some(HeadChange {
                head_idx,
                mode_idx: None,
                position: Some((r.out_x, r.out_y)),
                scale: None,
            })
        }).collect()
    }

    /// Export current arrangement as config entries, preserving scale/wallpaper
    /// and other fields from the existing config for monitors that already have
    /// entries. Only position is updated from the arrangement canvas.
    pub fn to_config(&self, existing: &[MonitorEntry]) -> Vec<MonitorEntry> {
        self.rects.iter().map(|r| {
            if let Some(prev) = existing.iter().find(|m| m.name == r.name) {
                MonitorEntry {
                    name: r.name.clone(),
                    x: r.out_x,
                    y: r.out_y,
                    resolution: prev.resolution.clone(),
                    refresh_rate: prev.refresh_rate.clone(),
                    scale: prev.scale,
                    wallpaper: prev.wallpaper.clone(),
                    primary: prev.primary,
                    vrr: prev.vrr,
                    hdr: prev.hdr,
                    sdr_brightness: prev.sdr_brightness,
                }
            } else {
                MonitorEntry {
                    name: r.name.clone(),
                    x: r.out_x,
                    y: r.out_y,
                    resolution: String::new(),
                    refresh_rate: String::new(),
                    scale: 1.0,
                    wallpaper: String::new(),
                    primary: false,
                    vrr: false,
                    hdr: false,
                    sdr_brightness: 203,
                }
            }
        }).collect()
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
#[allow(clippy::too_many_arguments)]
pub fn draw_monitor_arrange(
    mas: &mut MonitorArrangeState,
    outputs: &[(u32, OutputInfo)],
    config: &[MonitorEntry],
    output_mgr: &OutputManagerClient,
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
    show_header: bool,
) -> f32 {
    let pad = PAD * s;
    let canvas_h = CANVAS_H * s;
    let label_sz = LABEL_SIZE * s;
    let name_sz = NAME_SIZE * s;
    let res_sz = RES_SIZE * s;

    mas.sync_from_outputs(outputs, config, output_mgr);

    // Section label (skipped when drawn inside a card with its own header)
    let canvas_y = if show_header {
        text.queue("Displays", label_sz, x + pad, y + pad, fox.text, w, sw, sh);
        y + pad + label_sz + 12.0 * s
    } else {
        y
    };
    let canvas_w = w - pad * 2.0;

    // Canvas background
    let canvas_rect = Rect::new(x + pad, canvas_y, canvas_w, canvas_h);
    painter.rect_filled(canvas_rect, 8.0 * s, fox.surface);

    // Compute view transform — frozen during an active drag. If we recomputed
    // each frame, the dragged monitor would push the bounding box outward,
    // shrinking view_scale and shifting canvas_offset, which (a) makes every
    // other monitor appear to slide and (b) turns drag into a feedback loop
    // (smaller scale → same cursor delta → bigger output-space delta → bigger
    // box → smaller scale …) and the monitor flies off-canvas.
    if mas.dragging < 0 {
        let (vs, cx, cy) = compute_view(&mas.rects, canvas_w, canvas_h, s);
        mas.view_scale = vs;
        mas.canvas_offset_x = x + pad + cx;
        mas.canvas_offset_y = canvas_y + cy;
    }

    // Draw monitor rectangles
    for (i, r) in mas.rects.iter().enumerate() {
        if i as u32 >= MAX_MONITORS { break; }

        let rx = mas.canvas_offset_x + r.out_x as f32 * mas.view_scale;
        let ry = mas.canvas_offset_y + r.out_y as f32 * mas.view_scale;
        let rw = r.out_w as f32 * mas.view_scale;
        let rh = r.out_h as f32 * mas.view_scale;

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

/// End drag. Find the nearest *edge* to snap to (right/left/top/bottom) and
/// then force perpendicular alignment so the two monitors sit flush. That way
/// dropping monitor B next to A always produces a clean side-by-side or
/// stacked layout — no accidental sliver gap, no 1-pixel overlap.
pub fn handle_arrange_release(mas: &mut MonitorArrangeState) {
    if mas.dragging < 0 { return; }
    let idx = mas.dragging as usize;
    mas.dragging = -1;

    if idx >= mas.rects.len() { return; }

    // Snap radius expressed in canvas pixels (what the user perceives) then
    // mapped back to output space so the snap feel stays consistent whether
    // you're zoomed in or out.
    const SNAP_CANVAS_PX: f32 = 60.0;
    let snap_threshold = ((SNAP_CANVAS_PX / mas.view_scale.max(0.01)) as i32).max(20);

    let r = mas.rects[idx].clone();

    // Find the single closest edge across all neighbors. `which` distinguishes
    // horizontal-edge snaps (where we then align tops/bottoms/centers
    // vertically) from vertical-edge snaps (where we align horizontally).
    #[derive(Clone, Copy)]
    enum Edge { Horiz, Vert }
    let mut best: Option<(i32, Edge, usize)> = None;

    for (i, other) in mas.rects.iter().enumerate() {
        if i == idx { continue; }

        let candidates = [
            // Place dragged to the right of other (left edge meets right edge)
            (other.out_x + other.out_w - r.out_x,        Edge::Horiz),
            // Place dragged to the left of other
            (other.out_x - (r.out_x + r.out_w),          Edge::Horiz),
            // Place dragged below other
            (other.out_y + other.out_h - r.out_y,        Edge::Vert),
            // Place dragged above other
            (other.out_y - (r.out_y + r.out_h),          Edge::Vert),
        ];

        for (delta, edge) in candidates {
            if delta.abs() < snap_threshold
                && best.map_or(true, |(d, _, _)| delta.abs() < d.abs())
            {
                best = Some((delta, edge, i));
            }
        }
    }

    let Some((delta, edge, other_idx)) = best else {
        resolve_overlap(&mut mas.rects, idx);
        return;
    };

    let other = mas.rects[other_idx].clone();
    match edge {
        Edge::Horiz => {
            // Lock the touching edge.
            mas.rects[idx].out_x += delta;
            // Then pick the closest of three vertical alignments — top,
            // center, or bottom — to kill any vertical sliver.
            let r = &mas.rects[idx];
            let align_top    = other.out_y - r.out_y;
            let align_bottom = (other.out_y + other.out_h) - (r.out_y + r.out_h);
            let align_center = (other.out_y + other.out_h / 2) - (r.out_y + r.out_h / 2);
            let dy = [align_top, align_center, align_bottom]
                .into_iter()
                .min_by_key(|d| d.abs())
                .unwrap();
            mas.rects[idx].out_y += dy;
        }
        Edge::Vert => {
            mas.rects[idx].out_y += delta;
            let r = &mas.rects[idx];
            let align_left   = other.out_x - r.out_x;
            let align_right  = (other.out_x + other.out_w) - (r.out_x + r.out_w);
            let align_center = (other.out_x + other.out_w / 2) - (r.out_x + r.out_w / 2);
            let dx = [align_left, align_center, align_right]
                .into_iter()
                .min_by_key(|d| d.abs())
                .unwrap();
            mas.rects[idx].out_x += dx;
        }
    }

    resolve_overlap(&mut mas.rects, idx);
}

/// Push `idx` out of any monitor it overlaps, along the axis with the smallest
/// penetration depth. Prevents saving overlapping layouts that strand windows
/// in coordinate-space gaps between monitors.
fn resolve_overlap(rects: &mut [MonRect], idx: usize) {
    loop {
        let r = rects[idx].clone();
        let r_left = r.out_x;
        let r_right = r.out_x + r.out_w;
        let r_top = r.out_y;
        let r_bottom = r.out_y + r.out_h;

        let mut best: Option<(i32, i32)> = None;
        for (i, other) in rects.iter().enumerate() {
            if i == idx { continue; }
            let o_left = other.out_x;
            let o_right = other.out_x + other.out_w;
            let o_top = other.out_y;
            let o_bottom = other.out_y + other.out_h;

            let overlap_x = r_right.min(o_right) - r_left.max(o_left);
            let overlap_y = r_bottom.min(o_bottom) - r_top.max(o_top);
            if overlap_x <= 0 || overlap_y <= 0 { continue; }

            let push_left  = -(r_right - o_left);
            let push_right =   o_right - r_left;
            let push_up    = -(r_bottom - o_top);
            let push_down  =   o_bottom - r_top;

            let (dx, dy) = [
                (push_left, 0),
                (push_right, 0),
                (0, push_up),
                (0, push_down),
            ]
                .into_iter()
                .min_by_key(|(dx, dy)| dx.abs() + dy.abs())
                .unwrap();

            if best.map_or(true, |(bx, by)| dx.abs() + dy.abs() > bx.abs() + by.abs()) {
                best = Some((dx, dy));
            }
        }

        match best {
            Some((dx, dy)) => {
                rects[idx].out_x += dx;
                rects[idx].out_y += dy;
            }
            None => break,
        }
    }
}

/// Check if currently dragging.
pub fn is_dragging(mas: &MonitorArrangeState) -> bool {
    mas.dragging >= 0
}

/// Parse a `"WxH"` resolution string from lantern.toml. Returns `None` for
/// empty or malformed values.
fn parse_resolution(s: &str) -> Option<(i32, i32)> {
    let (w, h) = s.split_once('x')?;
    Some((w.trim().parse().ok()?, h.trim().parse().ok()?))
}

//! The Monitor page — live GPU + CPU stat cards and the per-core grid. Pure
//! function of the latest telemetry; no interactive controls live here.

use lntrn_afterglow::ipc::{CpuTuning, Info, Snapshot};
use lntrn_render::Rect;

use super::draw::{
    cpu_temp_color, gpu_temp_color, signed, tile_grid, Dc, CARD_GAP, HEADER_H, PAD, TILE_GAP,
    TILE_H,
};

/// Draws both cards starting at `y`; CPU on top, GPU below (user preference).
pub fn draw(
    dc: &mut Dc,
    info: &Info,
    snap: &Snapshot,
    tuning: Option<&CpuTuning>,
    x: f32,
    mut y: f32,
    cw: f32,
) {
    y = draw_cpu_card(dc, info, snap, tuning, x, y, cw);
    if info.gpu {
        y += CARD_GAP * dc.s;
        draw_gpu_card(dc, info, snap, x, y, cw);
    }
}

/// Returns the y just past the card.
fn draw_gpu_card(dc: &mut Dc, info: &Info, snap: &Snapshot, x: f32, y: f32, cw: f32) -> f32 {
    let s = dc.s;
    let rows = 2.0; // 7 tiles over 4 columns
    let card_h = HEADER_H * s + rows * TILE_H * s + (rows - 1.0) * TILE_GAP * s + PAD * s;
    let card = Rect::new(x, y, cw, card_h);
    dc.p.rect_filled(card, 14.0 * s, dc.pal.surface);

    dc.label(&info.gpu_name, x + PAD * s, y + 14.0 * s, 24.0, dc.pal.text);
    let sub = format!("driver {} · {} fans", info.driver, info.fans);
    dc.label(&sub, x + PAD * s, y + 40.0 * s, 14.0, dc.pal.text_secondary);

    let region = Rect::new(
        x + PAD * s,
        y + HEADER_H * s,
        cw - PAD * 2.0 * s,
        rows * TILE_H * s + TILE_GAP * s,
    );
    let cells = tile_grid(region, 4, 7, s);

    let temp_c = gpu_temp_color(dc.pal, snap.temp_c);
    dc.tile(
        cells[0],
        "TEMP",
        &format!("{}°C", snap.temp_c),
        None,
        temp_c,
        Some(snap.temp_c as f32 / 95.0),
    );

    let pl = snap.power_limit_w.max(1.0);
    dc.tile(
        cells[1],
        "POWER",
        &format!("{:.0} W", snap.power_w),
        Some(&format!("limit {:.0} W", snap.power_limit_w)),
        dc.pal.accent,
        Some(snap.power_w / pl),
    );

    dc.tile(
        cells[2],
        "GPU LOAD",
        &format!("{}%", snap.util_pct),
        None,
        dc.pal.accent,
        Some(snap.util_pct as f32 / 100.0),
    );

    let used_gb = snap.vram_used_mb as f32 / 1024.0;
    let total_gb = (snap.vram_total_mb as f32 / 1024.0).max(0.1);
    dc.tile(
        cells[3],
        "VRAM",
        &format!("{used_gb:.1} GB"),
        Some(&format!("of {total_gb:.1} GB")),
        dc.pal.info,
        Some(used_gb / total_gb),
    );

    dc.tile(
        cells[4],
        "CORE CLOCK",
        &format!("{} MHz", snap.core_mhz),
        Some(&format!("offset {}", signed(info.core_off))),
        dc.pal.text,
        None,
    );
    dc.tile(
        cells[5],
        "MEM CLOCK",
        &format!("{} MHz", snap.mem_mhz),
        Some(&format!("offset {}", signed(info.mem_off))),
        dc.pal.text,
        None,
    );

    let fan_txt = if snap.fans_pct.is_empty() {
        "—".to_string()
    } else {
        let avg = snap.fans_pct.iter().sum::<u32>() / snap.fans_pct.len() as u32;
        format!("{avg}%")
    };
    dc.tile(
        cells[6],
        "FANS",
        &fan_txt,
        Some(&format!("{} fans", snap.fans_pct.len())),
        dc.pal.info,
        None,
    );

    y + card_h
}

/// Returns the y just past the card.
fn draw_cpu_card(
    dc: &mut Dc,
    info: &Info,
    snap: &Snapshot,
    tuning: Option<&CpuTuning>,
    x: f32,
    y: f32,
    cw: f32,
) -> f32 {
    let s = dc.s;
    let grid_h = 96.0 * s;
    let card_h = HEADER_H * s + TILE_H * s + TILE_GAP * s + grid_h + PAD * s;
    let card = Rect::new(x, y, cw, card_h);
    dc.p.rect_filled(card, 14.0 * s, dc.pal.surface);

    dc.label(
        &info.cpu_model,
        x + PAD * s,
        y + 14.0 * s,
        24.0,
        dc.pal.text,
    );
    let p = snap.cores.iter().filter(|c| c.is_pcore).count();
    let sub = format!(
        "{} threads · {p} P · {} E",
        snap.cores.len(),
        snap.cores.len() - p
    );
    dc.label(&sub, x + PAD * s, y + 40.0 * s, 14.0, dc.pal.text_secondary);

    let region = Rect::new(
        x + PAD * s,
        y + HEADER_H * s,
        cw - PAD * 2.0 * s,
        TILE_H * s,
    );
    let cells = tile_grid(region, 4, 4, s);

    let (temp_val, temp_col) = match snap.cpu_temp_c {
        Some(c) => (format!("{c:.0}°C"), cpu_temp_color(dc.pal, c)),
        None => ("—".to_string(), dc.pal.text),
    };
    dc.tile(
        cells[0],
        "PKG TEMP",
        &temp_val,
        None,
        temp_col,
        snap.cpu_temp_c.map(|c| c / 100.0),
    );
    dc.tile(
        cells[1],
        "CPU LOAD",
        &format!("{:.0}%", snap.cpu_util_pct),
        None,
        dc.pal.accent,
        Some(snap.cpu_util_pct / 100.0),
    );

    let pwr = match snap.pkg_w {
        Some(p) => format!("{p:.0} W"),
        None => "—".to_string(),
    };
    let pl_sub = if info.rapl {
        format!("PL1 {} W", info.pl1_w)
    } else {
        "RAPL n/a".to_string()
    };
    let pwr_frac = match (snap.pkg_w, info.pl1_w) {
        (Some(p), pl) if pl > 0 => Some(p / pl as f32),
        _ => None,
    };
    dc.tile(
        cells[2],
        "PKG POWER",
        &pwr,
        Some(&pl_sub),
        dc.pal.accent,
        pwr_frac,
    );

    let gov = tuning.map(|t| t.governor.as_str()).unwrap_or("—");
    let turbo = match tuning.and_then(|t| t.turbo) {
        Some(true) => "turbo on",
        Some(false) => "turbo off",
        None => "",
    };
    dc.tile(cells[3], "GOVERNOR", gov, Some(turbo), dc.pal.info, None);

    let grid_y = y + HEADER_H * s + TILE_H * s + TILE_GAP * s;
    draw_core_grid(dc, snap, x + PAD * s, grid_y, cw - PAD * 2.0 * s, grid_h);

    y + card_h
}

/// A row of slim per-core meters: fill height = utilisation, P-cores accent,
/// E-cores info-tinted.
fn draw_core_grid(dc: &mut Dc, snap: &Snapshot, x: f32, y: f32, w: f32, h: f32) {
    let n = snap.cores.len().max(1);
    let per_row = ((n + 1) / 2).max(1); // two rows
    let gap = 6.0 * dc.s;
    let cell_w = (w - gap * (per_row as f32 - 1.0)) / per_row as f32;
    let row_h = (h - gap) / 2.0;

    for (i, core) in snap.cores.iter().enumerate() {
        let (col, row) = (i % per_row, i / per_row);
        let cx = x + col as f32 * (cell_w + gap);
        let cy = y + row as f32 * (row_h + gap);
        let cell = Rect::new(cx, cy, cell_w, row_h);
        dc.p.rect_filled(cell, 4.0 * dc.s, dc.pal.surface_2);

        let frac = (core.util_pct / 100.0).clamp(0.0, 1.0);
        let fill_h = (row_h * frac).max(if frac > 0.0 { 2.0 * dc.s } else { 0.0 });
        let base = if core.is_pcore {
            dc.pal.accent
        } else {
            dc.pal.info
        };
        if fill_h > 0.0 {
            dc.p.rect_filled(
                Rect::new(cx, cy + row_h - fill_h, cell_w, fill_h),
                4.0 * dc.s,
                base,
            );
        }
    }
}

pub fn draw_offline(dc: &mut Dc, status: &str, x: f32, y: f32, cw: f32, hf: f32) {
    let s = dc.s;
    let card = Rect::new(x, y, cw, hf - y - PAD * s);
    dc.p.rect_filled(card, 14.0 * s, dc.pal.surface);
    let cy = y + 60.0 * s;
    dc.label(
        "afterglowd not reachable",
        x + PAD * s,
        cy,
        24.0,
        dc.pal.danger,
    );
    dc.label(
        status,
        x + PAD * s,
        cy + 38.0 * s,
        16.0,
        dc.pal.text_secondary,
    );
    dc.label(
        "Start the daemon (boot service):",
        x + PAD * s,
        cy + 76.0 * s,
        16.0,
        dc.pal.text,
    );
    dc.label(
        "sudo rc-service lntrn-afterglowd start",
        x + PAD * s,
        cy + 104.0 * s,
        16.0,
        dc.pal.accent,
    );
}

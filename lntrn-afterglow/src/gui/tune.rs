//! The Tune page — staged tuning controls in the Apply-button model. Two
//! columns (GPU left, CPU right), an action bar at the bottom. Sliders are
//! hit-tested by geometry in mod.rs (they need drag); toggles/cycle-boxes/
//! buttons register interaction zones here and are dispatched by zone id.

use lntrn_render::Rect;
use lntrn_ui::gpu::{Button, ButtonVariant, InteractionContext, Slider, Toggle};

use lntrn_afterglow::ipc::{CpuTuning, Info};

use super::draw::{content_rect, Dc, PAD};
use super::pending::{
    Knob, Pending, ZONE_APPLY, ZONE_EPP, ZONE_GOV, ZONE_RESET, ZONE_SAVE, ZONE_TGL_FAN,
    ZONE_TGL_LOCK, ZONE_TGL_TURBO,
};

const SLIDER_ROW: f32 = 64.0;
const TOGGLE_ROW: f32 = 42.0;
const BOX_ROW: f32 = 60.0;
const TRACK_H: f32 = 26.0;
const BTN_W: f32 = 118.0;
const BTN_H: f32 = 42.0;
/// Vertical space reserved above each column for its GPU / CPU header.
const COL_HEADER: f32 = 48.0;

/// Geometry of the interactive controls, recomputed each frame and also from
/// mod.rs for hit-testing. Sliders carry their track rect; everything else is
/// a single rect.
pub struct TuneLayout {
    pub sliders: Vec<(Knob, Rect)>,
    pub fan_toggle: Rect,
    pub lock_toggle: Rect,
    pub turbo_toggle: Rect,
    pub gov_box: Rect,
    pub epp_box: Rect,
    pub apply: Rect,
    pub save: Rect,
    pub reset: Rect,
}

pub fn layout(info: &Info, wf: f32, hf: f32, s: f32) -> TuneLayout {
    let content = content_rect(wf, hf, s);
    let pad = PAD * s;
    let col_w = (content.w - pad) / 2.0;
    let left_x = content.x;
    let right_x = content.x + col_w + pad;
    let track = |x: f32, row_y: f32| Rect::new(x, row_y + 30.0 * s, col_w, TRACK_H * s);

    let mut sliders = Vec::new();

    // ── Left column: GPU ──
    let mut ly = content.y + COL_HEADER * s;
    let (mut fan_toggle, mut lock_toggle) =
        (Rect::new(0.0, 0.0, 0.0, 0.0), Rect::new(0.0, 0.0, 0.0, 0.0));
    if info.gpu {
        sliders.push((Knob::CoreOffset, track(left_x, ly)));
        ly += SLIDER_ROW * s;
        sliders.push((Knob::MemOffset, track(left_x, ly)));
        ly += SLIDER_ROW * s;
        sliders.push((Knob::PowerLimit, track(left_x, ly)));
        ly += SLIDER_ROW * s;
        fan_toggle = Rect::new(left_x, ly, col_w, TOGGLE_ROW * s);
        ly += TOGGLE_ROW * s;
        sliders.push((Knob::FanPct, track(left_x, ly)));
        ly += SLIDER_ROW * s;
        lock_toggle = Rect::new(left_x, ly, col_w, TOGGLE_ROW * s);
        ly += TOGGLE_ROW * s;
        sliders.push((Knob::LockMhz, track(left_x, ly)));
    }

    // ── Right column: CPU ──
    let mut ry = content.y + COL_HEADER * s;
    let gov_box = Rect::new(right_x, ry + 22.0 * s, col_w, 34.0 * s);
    ry += BOX_ROW * s;
    let epp_box = Rect::new(right_x, ry + 22.0 * s, col_w, 34.0 * s);
    ry += BOX_ROW * s;
    let turbo_toggle = Rect::new(right_x, ry, col_w, TOGGLE_ROW * s);
    ry += TOGGLE_ROW * s;
    sliders.push((Knob::MaxFreq, track(right_x, ry)));
    ry += SLIDER_ROW * s;
    if info.rapl {
        sliders.push((Knob::Pl1, track(right_x, ry)));
        ry += SLIDER_ROW * s;
        sliders.push((Knob::Pl2, track(right_x, ry)));
    }

    // ── Action bar (bottom) ──
    let by = content.y + content.h - BTN_H * s;
    let gap = 10.0 * s;
    let reset = Rect::new(content.x + content.w - BTN_W * s, by, BTN_W * s, BTN_H * s);
    let save = Rect::new(reset.x - (BTN_W + 10.0) * s, by, BTN_W * s, BTN_H * s);
    let apply = Rect::new(save.x - (BTN_W + 10.0) * s, by, BTN_W * s, BTN_H * s);
    let _ = gap;

    TuneLayout {
        sliders,
        fan_toggle,
        lock_toggle,
        turbo_toggle,
        gov_box,
        epp_box,
        apply,
        save,
        reset,
    }
}

#[allow(clippy::too_many_arguments)]
pub fn draw(
    dc: &mut Dc,
    input: &mut InteractionContext,
    info: &Info,
    tuning: Option<&CpuTuning>,
    pending: &Pending,
    cursor: Option<(f32, f32)>,
    dragging: Option<Knob>,
    status: &str,
    wf: f32,
    hf: f32,
) {
    let s = dc.s;
    let lay = layout(info, wf, hf, s);

    // ── Column headers + divider so it's obvious which knobs are which ──
    let content = content_rect(wf, hf, s);
    let col_w = (content.w - PAD * s) / 2.0;
    let right_x = content.x + col_w + PAD * s;
    let hy = content.y;
    dc.label("GPU", content.x, hy, 23.0, dc.pal.accent);
    dc.label_w(
        &short_gpu(&info.gpu_name),
        content.x + 62.0 * s,
        hy + 4.0 * s,
        15.0,
        dc.pal.text_secondary,
        col_w - 64.0 * s,
    );
    dc.label("CPU", right_x, hy, 23.0, dc.pal.accent);
    dc.label_w(
        &short_cpu(&info.cpu_model),
        right_x + 58.0 * s,
        hy + 4.0 * s,
        15.0,
        dc.pal.text_secondary,
        col_w - 60.0 * s,
    );
    // underline each header
    dc.p.rect_filled(
        Rect::new(content.x, hy + 30.0 * s, col_w, 2.0 * s),
        1.0,
        dc.pal.accent.with_alpha(0.5),
    );
    dc.p.rect_filled(
        Rect::new(right_x, hy + 30.0 * s, col_w, 2.0 * s),
        1.0,
        dc.pal.accent.with_alpha(0.5),
    );
    // faint vertical divider between the columns
    let mid = content.x + col_w + PAD * s * 0.5;
    dc.p.rect_filled(
        Rect::new(mid - 0.5 * s, hy, 1.0 * s, content.h - BTN_H * s - 16.0 * s),
        0.0,
        dc.pal.muted.with_alpha(0.12),
    );

    // Sliders (geometry-hit, not zones)
    for &(knob, track) in &lay.sliders {
        let enabled = super::pending::slider_editable(knob, pending);
        let hovered =
            dragging.is_none() && cursor.map_or(false, |(cx, cy)| contains(track, cx, cy));
        draw_slider_row(
            dc,
            knob,
            track,
            pending,
            info,
            hovered,
            dragging == Some(knob),
            enabled,
        );
    }

    // GPU toggles
    if info.gpu {
        draw_toggle(
            dc,
            input,
            ZONE_TGL_FAN,
            lay.fan_toggle,
            pending.fan_manual,
            "Manual fan",
        );
        draw_toggle(
            dc,
            input,
            ZONE_TGL_LOCK,
            lay.lock_toggle,
            pending.lock_enabled,
            "Lock core clock (undervolt)",
        );
    }

    // CPU cycle-boxes + turbo
    let govs = tuning.map(|t| t.governors.len()).unwrap_or(0);
    let epps = tuning.map(|t| t.epps.len()).unwrap_or(0);
    draw_cycle_box(
        dc,
        input,
        ZONE_GOV,
        lay.gov_box,
        "GOVERNOR",
        value_or(&pending.governor),
        govs > 1,
    );
    draw_cycle_box(
        dc,
        input,
        ZONE_EPP,
        lay.epp_box,
        "ENERGY PREF (EPP)",
        value_or(&pending.epp),
        epps > 1,
    );
    draw_toggle(
        dc,
        input,
        ZONE_TGL_TURBO,
        lay.turbo_toggle,
        pending.turbo,
        "Intel Turbo Boost",
    );

    // Action bar
    draw_button(
        dc,
        input,
        ZONE_APPLY,
        lay.apply,
        "Apply",
        ButtonVariant::Primary,
    );
    draw_button(dc, input, ZONE_SAVE, lay.save, "Save", ButtonVariant::Ghost);
    draw_button(
        dc,
        input,
        ZONE_RESET,
        lay.reset,
        "Reset",
        ButtonVariant::Default,
    );

    if !status.is_empty() {
        let col = if status.starts_with("err") || status.contains("error") {
            dc.pal.danger
        } else {
            dc.pal.success
        };
        dc.label(status, lay.apply.x, lay.apply.y + 12.0 * s, 16.0, col);
    }
}

fn value_or(v: &str) -> &str {
    if v.is_empty() {
        "—"
    } else {
        v
    }
}

/// "NVIDIA GeForce RTX 3080 Ti" → "RTX 3080 Ti".
fn short_gpu(name: &str) -> String {
    name.trim()
        .trim_start_matches("NVIDIA ")
        .trim_start_matches("GeForce ")
        .to_string()
}

/// "Intel(R) Core(TM) i7-14700K" → "Core i7-14700K".
fn short_cpu(model: &str) -> String {
    let cleaned = model.replace("(R)", "").replace("(TM)", "");
    let collapsed = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    collapsed.trim_start_matches("Intel ").to_string()
}

fn contains(r: Rect, x: f32, y: f32) -> bool {
    x >= r.x && x <= r.x + r.w && y >= r.y && y <= r.y + r.h
}

#[allow(clippy::too_many_arguments)]
fn draw_slider_row(
    dc: &mut Dc,
    knob: Knob,
    track: Rect,
    pending: &Pending,
    info: &Info,
    hovered: bool,
    active: bool,
    enabled: bool,
) {
    let s = dc.s;
    let title = Pending::knob_title(knob);
    let value = pending.knob_value_str(knob);
    let title_col = if enabled { dc.pal.text } else { dc.pal.muted };
    let val_col = if enabled { dc.pal.accent } else { dc.pal.muted };
    dc.label(title, track.x, track.y - 26.0 * s, 17.0, title_col);
    let vw = value.chars().count() as f32 * 17.0 * s * 0.54;
    dc.label(
        &value,
        track.x + track.w - vw,
        track.y - 26.0 * s,
        17.0,
        val_col,
    );

    let frac = pending.slider_frac(knob, info);
    let mut sl = Slider::new(track)
        .value(frac)
        .hovered(hovered)
        .active(active);
    if !enabled {
        sl.fill_start = dc.pal.surface_2;
        sl.fill_end = dc.pal.muted;
        sl.thumb_color = dc.pal.surface;
    }
    sl.draw(dc.p, dc.pal);
}

fn draw_toggle(
    dc: &mut Dc,
    input: &mut InteractionContext,
    zone: u32,
    rect: Rect,
    on: bool,
    label: &str,
) {
    let st = input.add_zone(zone, rect);
    Toggle::new(rect, on)
        .label(label)
        .scale(dc.s)
        .hovered(st.is_hovered())
        .draw(dc.p, dc.t, dc.pal, dc.w, dc.h);
}

fn draw_cycle_box(
    dc: &mut Dc,
    input: &mut InteractionContext,
    zone: u32,
    rect: Rect,
    caption: &str,
    value: &str,
    enabled: bool,
) {
    let s = dc.s;
    dc.label(
        caption,
        rect.x,
        rect.y - 20.0 * s,
        15.0,
        dc.pal.text_secondary,
    );
    let st = input.add_zone(zone, rect);
    let bg = if enabled && st.is_hovered() {
        dc.pal.surface
    } else {
        dc.pal.surface_2
    };
    dc.p.rect_filled(rect, 8.0 * s, bg);
    dc.label(
        value,
        rect.x + 12.0 * s,
        rect.y + 7.0 * s,
        18.0,
        dc.pal.text,
    );
    if enabled {
        dc.label(
            "⟳",
            rect.x + rect.w - 26.0 * s,
            rect.y + 7.0 * s,
            17.0,
            dc.pal.accent,
        );
    }
}

fn draw_button(
    dc: &mut Dc,
    input: &mut InteractionContext,
    zone: u32,
    rect: Rect,
    label: &str,
    variant: ButtonVariant,
) {
    let st = input.add_zone(zone, rect);
    Button::new(rect, label)
        .variant(variant)
        .scale(dc.s)
        .hovered(st.is_hovered())
        .pressed(st.is_active())
        .draw(dc.p, dc.t, dc.pal, dc.w, dc.h);
}

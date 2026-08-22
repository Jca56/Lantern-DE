//! Frame orchestration: window chrome, the Monitor/Tune tab strip, and
//! dispatch to whichever page is active. The pages live in monitor.rs / tune.rs.

use lntrn_render::{Color, Rect};
use lntrn_ui::gpu::{draw_window_bg, FoxPalette, InteractionContext, TitleBar};

use lntrn_afterglow::ipc::{CpuTuning, Info, Snapshot};

use super::draw::{content_rect, Dc, PAD, TAB_H, TITLE_BAR_H};
use super::pending::{Knob, Pending, ZONE_TAB_MONITOR, ZONE_TAB_TUNE};
use super::{monitor, tune, Gpu, Tab, ZONE_CLOSE, ZONE_MAXIMIZE, ZONE_MINIMIZE};

#[allow(clippy::too_many_arguments)]
pub fn render_frame(
    gpu: &mut Gpu,
    input: &mut InteractionContext,
    palette: &FoxPalette,
    bg_opacity: f32,
    scale: f32,
    tab: Tab,
    info: Option<&Info>,
    snap: Option<&Snapshot>,
    tuning: Option<&CpuTuning>,
    pending: &Pending,
    cursor: Option<(f32, f32)>,
    dragging: Option<Knob>,
    status: &str,
    tune_status: &str,
    connected: bool,
) {
    let Gpu { ctx, painter, text } = gpu;
    let (w, h) = (ctx.width(), ctx.height());
    let (wf, hf) = (w as f32, h as f32);
    let s = scale;

    painter.clear();
    input.begin_frame();
    draw_window_bg(
        painter,
        Rect::new(0.0, 0.0, wf, hf),
        18.0 * s,
        palette,
        bg_opacity,
    );

    // ── Title bar + connection dot ──────────────────────────────────
    let title_rect = Rect::new(0.0, 0.0, wf, TITLE_BAR_H * s);
    let tb = TitleBar::new(title_rect);
    let close = input.add_zone(ZONE_CLOSE, tb.close_button_rect());
    let maxi = input.add_zone(ZONE_MAXIMIZE, tb.maximize_button_rect());
    let mini = input.add_zone(ZONE_MINIMIZE, tb.minimize_button_rect());
    TitleBar::new(title_rect)
        .scale(s)
        .transparent_bg(true)
        .close_hovered(close.is_hovered())
        .maximize_hovered(maxi.is_hovered())
        .minimize_hovered(mini.is_hovered())
        .draw(painter, palette);

    let mut dc = Dc {
        p: painter,
        t: text,
        pal: palette,
        s,
        w,
        h,
    };
    dc.label("Afterglow", PAD * s, 15.0 * s, 22.0, palette.text);
    let dot = if connected {
        palette.success
    } else {
        palette.danger
    };
    dc.p.rect_filled(
        Rect::new(120.0 * s, 24.0 * s, 9.0 * s, 9.0 * s),
        4.5 * s,
        dot,
    );
    dc.label(status, 136.0 * s, 18.0 * s, 14.0, palette.text_secondary);

    // ── Tab strip ───────────────────────────────────────────────────
    draw_tabs(&mut dc, input, tab, wf, s);

    // ── Active page ─────────────────────────────────────────────────
    let content = content_rect(wf, hf, s);
    match (info, snap) {
        (Some(info), Some(snap)) => match tab {
            Tab::Monitor => {
                monitor::draw(&mut dc, info, snap, tuning, content.x, content.y, content.w)
            }
            Tab::Tune => tune::draw(
                &mut dc,
                input,
                info,
                tuning,
                pending,
                cursor,
                dragging,
                tune_status,
                wf,
                hf,
            ),
        },
        _ => monitor::draw_offline(&mut dc, status, content.x, content.y, content.w, hf),
    }

    // ── Submit ──────────────────────────────────────────────────────
    match ctx.begin_frame("lntrn-afterglow") {
        Ok(mut frame) => {
            painter.render_into(ctx, &mut frame, Color::rgba(0.0, 0.0, 0.0, 0.0));
            let view = frame.view().clone();
            text.render_queued(ctx, frame.encoder_mut(), &view);
            frame.submit(&ctx.queue);
        }
        Err(e) => eprintln!("[lntrn-afterglow] render error: {e}"),
    }
}

fn draw_tabs(dc: &mut Dc, input: &mut InteractionContext, tab: Tab, wf: f32, s: f32) {
    let y = TITLE_BAR_H * s;
    let h = TAB_H * s;
    dc.p.rect_filled(
        Rect::new(0.0, y + h - 1.0, wf, 1.0),
        0.0,
        dc.pal.muted.with_alpha(0.2),
    );
    let tabs = [
        ("Monitor", ZONE_TAB_MONITOR, Tab::Monitor),
        ("Tune", ZONE_TAB_TUNE, Tab::Tune),
    ];
    let mut x = PAD * s;
    let tw = 130.0 * s;
    for (label, zone, which) in tabs {
        let rect = Rect::new(x, y, tw, h);
        let st = input.add_zone(zone, rect);
        let active = tab == which;
        let col = if active {
            dc.pal.text
        } else if st.is_hovered() {
            dc.pal.text_secondary
        } else {
            dc.pal.muted
        };
        dc.label(label, x + 22.0 * s, y + (h - 22.0 * s) * 0.5, 19.0, col);
        if active {
            dc.p.rect_filled(
                Rect::new(x + 14.0 * s, y + h - 3.0 * s, tw - 28.0 * s, 3.0 * s),
                1.5 * s,
                dc.pal.accent,
            );
        }
        x += tw;
    }
}

use lntrn_render::{Color, Rect};
use lntrn_ui::gpu::{
    FontSize, FoxPalette, InteractionContext, MenuEvent,
    TextLabel,
};

use crate::app::App;
use crate::icons::IconCache;
use crate::terminal_panel::TerminalPanel;
use crate::{Gpu, DesktopPanel};

/// Render a full frame.
pub fn render_frame(
    gpu: &mut Gpu,
    app: &mut App,
    input: &mut InteractionContext,
    icon_cache: &mut IconCache,
    file_info: &mut crate::file_info::FileInfoCache,
    palette: &FoxPalette,
    scale: f32,
    view_menu: &mut lntrn_ui::gpu::ContextMenu,
    context_menu: &mut lntrn_ui::gpu::ContextMenu,
    tab_drag: Option<usize>,
    bg_opacity: f32,
    active_panel: DesktopPanel,
    terminal_panel: &mut TerminalPanel,
    term_opacity: f32,
    transition: &crate::PanelTransition,
) -> (Option<MenuEvent>, Option<MenuEvent>) {
    let w = gpu.ctx.width();
    let h = gpu.ctx.height();
    let wf = w as f32;
    let hf = h as f32;
    let pal = palette;
    let s = scale;

    gpu.painter.clear();
    gpu.text.clear();
    gpu.mono_text.clear();
    input.begin_frame();

    // ── Window background (transparent for layer shell) ──────────────
    gpu.painter.rect_filled(Rect::new(0.0, 0.0, wf, hf), 0.0, pal.bg.with_alpha(bg_opacity));

    // ── Slide animation offsets ────────────────────────────────────
    let dir = if transition.active { transition.direction() } else { 0.0 };
    let eased = if transition.active { transition.eased() } else { 1.0 };
    let incoming_x = if transition.active { (1.0 - eased) * wf * dir } else { 0.0 };
    let outgoing_x = if transition.active { -eased * wf * dir } else { 0.0 };

    // Draw outgoing panel during transition
    if transition.active {
        draw_terminal_panel(transition.from, outgoing_x,
            &mut gpu.painter, &mut gpu.text, &mut gpu.mono_text, input, pal, s, wf, hf, w, h,
            terminal_panel, term_opacity);
    }

    // Draw incoming/active terminal or home panel
    draw_terminal_panel(active_panel, incoming_x,
        &mut gpu.painter, &mut gpu.text, &mut gpu.mono_text, input, pal, s, wf, hf, w, h,
        terminal_panel, term_opacity);

    // ── If Files panel is not needed at all, do GPU submit and return ─
    let files_needed = active_panel == DesktopPanel::Files
        || (transition.active && transition.from == DesktopPanel::Files);
    if !files_needed {
        gpu.painter.set_layer(1);
        gpu.text.set_layer(1);
        let ctx_evt = context_menu.draw(&mut gpu.painter, &mut gpu.text, input, w, h);
        gpu.text.set_layer(0);
        gpu.painter.set_layer(0);
        let frame = gpu.ctx.begin_frame("Lantern Desktop");
        match frame {
            Ok(mut frame) => {
                let view = frame.view().clone();
                gpu.painter.render_layer(0, &mut gpu.ctx, frame.encoder_mut(), &view, Some(Color::rgba(0.0, 0.0, 0.0, 0.0)));
                gpu.mono_text.render_layer(0, &mut gpu.ctx, frame.encoder_mut(), &view);
                gpu.text.render_layer(0, &mut gpu.ctx, frame.encoder_mut(), &view);
                frame.flush(&mut gpu.ctx);
                gpu.painter.render_layer(1, &mut gpu.ctx, frame.encoder_mut(), &view, None);
                gpu.text.render_layer(1, &mut gpu.ctx, frame.encoder_mut(), &view);
                frame.submit(&gpu.ctx.queue);
            }
            Err(e) => eprintln!("[desktop] render error: {e}"),
        }
        return (ctx_evt, None);
    }

    // ── Files panel ──────────────────────────────────────────────────
    let ox = if active_panel == DesktopPanel::Files {
        incoming_x
    } else {
        outgoing_x // Files is the outgoing panel during this transition
    };

    crate::render_files::draw_files_panel(
        gpu, input, pal, s, wf, hf, w, h,
        app, icon_cache, file_info, context_menu, view_menu, tab_drag, ox,
    )
}

/// Draw terminal panel content at a horizontal offset (for slide animation).
/// Does nothing for Home panel (transparent). Does nothing for Files panel (handled separately).
#[allow(clippy::too_many_arguments)]
fn draw_terminal_panel(
    panel: DesktopPanel,
    ox: f32,
    painter: &mut lntrn_render::Painter,
    text: &mut lntrn_render::TextRenderer,
    mono_text: &mut lntrn_render::TextRenderer,
    input: &mut InteractionContext,
    pal: &FoxPalette,
    s: f32, wf: f32, hf: f32, w: u32, h: u32,
    terminal_panel: &mut TerminalPanel,
    term_opacity: f32,
) {
    if panel != DesktopPanel::Terminal { return; }

    let tab_h = crate::terminal_panel::TAB_BAR_HEIGHT * s;
    // Clip to the visible portion of the sliding panel
    let clip_x = ox.max(0.0);
    let clip_w = (wf - clip_x).max(0.0);
    let clip = Rect::new(clip_x, 0.0, clip_w, hf);
    painter.push_clip(clip);
    mono_text.push_clip([clip.x, clip.y, clip.w, clip.h]);
    text.push_clip([clip.x, clip.y, clip.w, clip.h]);

    // Backgrounds
    painter.rect_filled(Rect::new(ox, 0.0, wf, tab_h), 0.0, pal.bg.with_alpha(term_opacity));
    painter.rect_filled(Rect::new(ox, tab_h, wf, hf - tab_h), 0.0, pal.bg.with_alpha(term_opacity));

    // Tab bar
    let labels = terminal_panel.tab_labels();
    let tab_font = 18.0 * s;
    let tab_pad = 14.0 * s;
    let close_sz = 14.0 * s;
    let close_pad = 8.0 * s;
    let tab_r = 8.0 * s;
    let tab_gap = 4.0 * s;
    let active_blue = Color::from_rgb8(100, 160, 255);
    let danger_red = Color::from_rgb8(220, 60, 60);
    let tab_y = 4.0 * s;
    let tab_inner_h = tab_h - 8.0 * s;
    let mut tx = ox + 8.0 * s;
    let multi = labels.len() > 1;

    for (i, label) in labels.iter().enumerate() {
        let text_w = label.len().max(4) as f32 * tab_font * 0.52;
        let close_space = if multi { close_sz + close_pad } else { 0.0 };
        let tw = text_w + tab_pad * 2.0 + close_space;
        let tr = Rect::new(tx, tab_y, tw, tab_inner_h);
        let zone = input.add_zone(crate::ZONE_TERM_TAB_BASE + i as u32, tr);
        let active = i == terminal_panel.active;

        if active {
            painter.rect_filled(tr, tab_r, pal.surface.with_alpha(0.7));
        } else if zone.is_hovered() {
            painter.rect_filled(tr, tab_r, pal.surface.with_alpha(0.4));
        } else {
            painter.rect_filled(tr, tab_r, pal.surface.with_alpha(0.2));
        }

        let label_x = tx + tab_pad;
        let color = if active { active_blue } else { pal.text };
        TextLabel::new(label, label_x, tab_y + (tab_inner_h - tab_font) * 0.5)
            .size(FontSize::Custom(tab_font))
            .color(color)
            .bold()
            .draw(text, w, h);

        if multi {
            let x_sz = close_sz;
            let x_x = tx + tw - tab_pad - x_sz;
            let x_y = tab_y + (tab_inner_h - x_sz) * 0.5;
            let x_rect = Rect::new(x_x, x_y, x_sz, x_sz);
            let x_zone = input.add_zone(crate::ZONE_TERM_TAB_CLOSE_BASE + i as u32, x_rect);
            let x_color = if x_zone.is_hovered() { danger_red } else { pal.text.with_alpha(0.5) };
            let m = 3.0 * s;
            let lw = 2.0 * s;
            painter.line(x_x + m, x_y + m, x_x + x_sz - m, x_y + x_sz - m, lw, x_color);
            painter.line(x_x + x_sz - m, x_y + m, x_x + m, x_y + x_sz - m, lw, x_color);
        }

        tx += tw + tab_gap;
    }

    // + button
    let plus_w = tab_font * 0.52 + tab_pad * 2.0;
    let plus_rect = Rect::new(tx, tab_y, plus_w, tab_inner_h);
    let plus_zone = input.add_zone(crate::ZONE_TERM_TAB_NEW, plus_rect);
    if plus_zone.is_hovered() {
        painter.rect_filled(plus_rect, tab_r, pal.surface.with_alpha(0.4));
    } else {
        painter.rect_filled(plus_rect, tab_r, pal.surface.with_alpha(0.2));
    }
    let plus_x = tx + (plus_w - tab_font * 0.52) * 0.5;
    TextLabel::new("+", plus_x, tab_y + (tab_inner_h - tab_font) * 0.5)
        .size(FontSize::Custom(tab_font))
        .color(pal.text)
        .bold()
        .draw(text, w, h);

    // Terminal grid
    let session = terminal_panel.active_session();
    crate::terminal_render::draw_terminal_ex(
        painter, mono_text,
        &session.terminal,
        terminal_panel.font_size,
        (ox, tab_h),
        w, h,
        terminal_panel.cursor_visible,
        Color::rgba(0.0, 0.0, 0.0, 0.0),
        0,
    );

    text.pop_clip();
    mono_text.pop_clip();
    painter.pop_clip();
}

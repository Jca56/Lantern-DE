use lntrn_render::{Painter, Rect, TextRenderer};
use lntrn_ui::gpu::{
    Fill, FontSize, FoxPalette, GradientBorder, InteractionContext, Panel, ScrollArea, Scrollbar,
    TextLabel,
};

use crate::layout::*;

pub const ZONE_SCROLLBAR: u32 = 80;

/// Settings-style list item labels for the scroll demo.
const SCROLL_ITEM_LABELS: &[&str] = &[
    "Display Settings",
    "Network Config",
    "Audio Output",
    "Keyboard Layout",
    "Mouse & Touchpad",
    "Power Management",
    "Notifications",
    "Default Apps",
    "Accessibility",
    "About System",
    "User Accounts",
    "Privacy",
    "Date & Time",
    "Language & Region",
    "Software Updates",
];

pub fn draw_containers_tab(
    painter: &mut Painter,
    text_renderer: &mut TextRenderer,
    fox: &FoxPalette,
    ix: &mut InteractionContext,
    content: Rect,
    scroll_offset: &mut f32,
    size: (u32, u32),
) -> Option<Rect> {
    let border = fox.muted.with_alpha(0.3);

    // ── Left column: Panel + Fill + Alpha Gradient ───────────────────────
    let panel_area = cont_panel_area_rect(content);

    // Panel with gradient fill demo (top portion)
    let panel_h = (panel_area.h - SECTION_V_GAP * 2.0) * 0.45;
    let panel_demo = Rect::new(panel_area.x, panel_area.y, panel_area.w, panel_h);
    Panel::new(panel_demo)
        .fill(Fill::vertical(fox.surface_2, fox.surface))
        .radius(SUB_PANEL_RADIUS)
        .draw(painter);
    painter.rect_stroke(panel_demo, SUB_PANEL_RADIUS, 1.0, border);

    TextLabel::new("Panel + Fill", panel_demo.x + 18.0, panel_demo.y + 14.0)
        .size(FontSize::Small)
        .color(fox.text)
        .draw(text_renderer, size.0, size.1);

    // Inner demo panels showing fill types
    let demo_w = (panel_demo.w - 54.0) / 3.0;
    let demo_h = 60.0;
    let demo_y = panel_demo.y + 52.0;
    let fills = [
        ("Solid", Fill::Solid(fox.accent.with_alpha(0.4))),
        ("Vertical", Fill::vertical(fox.accent, fox.bg)),
        ("Radial", Fill::RadialGradient { center: fox.accent, edge: fox.bg }),
    ];

    for (i, (label, fill)) in fills.iter().enumerate() {
        let dx = panel_demo.x + 18.0 + i as f32 * (demo_w + 9.0);
        let r = Rect::new(dx, demo_y, demo_w, demo_h);
        Panel::new(r).fill(match fill {
            Fill::Solid(c) => Fill::Solid(*c),
            Fill::LinearGradient { angle, start, end } => Fill::LinearGradient {
                angle: *angle,
                start: *start,
                end: *end,
            },
            Fill::RadialGradient { center, edge } => Fill::RadialGradient {
                center: *center,
                edge: *edge,
            },
        }).radius(8.0).draw(painter);

        TextLabel::new(label, dx + 8.0, demo_y + demo_h + 6.0)
            .size(FontSize::Label)
            .color(fox.muted)
            .draw(text_renderer, size.0, size.1);
    }

    // GradientBorder demo (middle portion)
    let gb_y = panel_area.y + panel_h + SECTION_V_GAP;
    let gb_h = (panel_area.h - SECTION_V_GAP * 2.0) * 0.3;
    let gb_rect = Rect::new(panel_area.x, gb_y, panel_area.w, gb_h.max(60.0));

    let gb_colors = fox.gradient_border_colors();
    GradientBorder::new(gb_rect)
        .fill(Fill::Solid(fox.surface))
        .radius(SUB_PANEL_RADIUS)
        .border_width(3.0)
        .colors(gb_colors)
        .draw(painter);

    TextLabel::new("GradientBorder", gb_rect.x + 22.0, gb_rect.y + 16.0)
        .size(FontSize::Small)
        .color(fox.text)
        .draw(text_renderer, size.0, size.1);

    TextLabel::new("4-sided gradient border from the palette", gb_rect.x + 22.0, gb_rect.y + 44.0)
        .size(FontSize::Caption)
        .color(fox.muted)
        .max_width(gb_rect.w - 44.0)
        .draw(text_renderer, size.0, size.1);

    // Alpha Gradient demo (bottom portion)
    let ag_y = gb_y + gb_h + SECTION_V_GAP;
    let ag_h = (panel_area.h - panel_h - gb_h - SECTION_V_GAP * 2.0).max(50.0);
    let ag_rect = Rect::new(panel_area.x, ag_y, panel_area.w, ag_h);

    Panel::new(ag_rect)
        .fill(Fill::vertical(fox.surface_2, fox.surface))
        .radius(SUB_PANEL_RADIUS)
        .draw(painter);
    painter.rect_stroke(ag_rect, SUB_PANEL_RADIUS, 1.0, border);

    TextLabel::new("Alpha Gradient", ag_rect.x + 18.0, ag_rect.y + 14.0)
        .size(FontSize::Small)
        .color(fox.text)
        .draw(text_renderer, size.0, size.1);

    // Draw the gradient bar: accent fading to transparent (single GPU draw call)
    let bar_x = ag_rect.x + 18.0;
    let bar_y = ag_rect.y + 48.0;
    let bar_width = ag_rect.w - 36.0;
    let bar_h = (ag_h - 64.0).max(16.0);
    let bar_rect = Rect::new(bar_x, bar_y, bar_width, bar_h);

    // Accent → fully transparent, left to right (angle 0.0)
    painter.rect_gradient_linear(
        bar_rect,
        8.0,
        0.0,
        fox.accent,
        fox.accent.with_alpha(0.0),
    );

    // Second row: accent → bg color (shows how it blends against a surface)
    let bar2_y = bar_y + bar_h + 12.0;
    let bar2_h = bar_h.min((ag_rect.y + ag_h - bar2_y - 8.0).max(12.0));
    if bar2_h > 8.0 {
        painter.rect_gradient_linear(
            Rect::new(bar_x, bar2_y, bar_width, bar2_h),
            8.0,
            0.0,
            fox.accent,
            fox.bg,
        );
    }

    // ── Right column: Scroll area + texture demo ─────────────────────────
    let scroll_col = cont_scroll_area_rect(content);

    let scroll_h = (scroll_col.h - SECTION_V_GAP) * 0.6;
    let scroll_panel = Rect::new(scroll_col.x, scroll_col.y, scroll_col.w, scroll_h);

    Panel::new(scroll_panel)
        .fill(Fill::vertical(fox.surface_2, fox.surface))
        .radius(SUB_PANEL_RADIUS)
        .draw(painter);
    painter.rect_stroke(scroll_panel, SUB_PANEL_RADIUS, 1.0, border);

    TextLabel::new("Scroll Area", scroll_panel.x + 18.0, scroll_panel.y + 14.0)
        .size(FontSize::Small)
        .color(fox.text)
        .draw(text_renderer, size.0, size.1);

    let scroll_viewport = Rect::new(
        scroll_panel.x + 8.0,
        scroll_panel.y + 44.0,
        scroll_panel.w - 16.0,
        scroll_panel.h - 52.0,
    );
    let content_height = SCROLL_DEMO_ITEMS as f32 * SCROLL_DEMO_ITEM_H;

    // Apply wheel scroll if cursor is inside viewport
    if ix.is_hovered(&scroll_viewport) {
        let delta = ix.scroll_delta() * 40.0;
        ScrollArea::apply_scroll(scroll_offset, delta, content_height, scroll_viewport.h);
    }

    let area = ScrollArea::new(scroll_viewport, content_height, scroll_offset);
    let scrollbar = Scrollbar::new(&scroll_viewport, content_height, *scroll_offset);
    let sb_state = ix.add_zone(ZONE_SCROLLBAR, scrollbar.thumb);

    // Handle scrollbar thumb drag
    if sb_state.is_active() {
        if let Some((_, y)) = ix.cursor() {
            *scroll_offset = scrollbar.offset_for_thumb_y(y, content_height, scroll_viewport.h);
        }
    }

    // Scroll offset label
    let offset_text = format!("offset: {:.0}/{:.0}", area.offset, area.max_offset());
    TextLabel::new(
        &offset_text,
        scroll_panel.x + scroll_panel.w - 180.0,
        scroll_panel.y + 14.0,
    )
        .size(FontSize::Caption)
        .color(fox.muted)
        .draw(text_renderer, size.0, size.1);

    // Scroll items -- settings-style list
    let item_colors = [fox.accent, fox.danger, fox.success, fox.text_secondary];
    area.begin(painter);
    let cy = area.content_y();
    for i in 0..SCROLL_DEMO_ITEMS {
        let item_y = cy + i as f32 * SCROLL_DEMO_ITEM_H;
        if item_y + SCROLL_DEMO_ITEM_H < scroll_viewport.y
            || item_y > scroll_viewport.y + scroll_viewport.h
        {
            continue;
        }
        let item_rect = Rect::new(
            scroll_viewport.x + 4.0,
            item_y + 2.0,
            scroll_viewport.w - 20.0,
            SCROLL_DEMO_ITEM_H - 4.0,
        );
        let color = item_colors[i % item_colors.len()];

        // Subtle background
        painter.rect_filled(item_rect, 8.0, fox.surface_2.with_alpha(0.5));

        // Left accent stripe
        let stripe = Rect::new(item_rect.x, item_rect.y + 4.0, 3.0, item_rect.h - 8.0);
        painter.rect_filled(stripe, 1.5, color.with_alpha(0.7));

        // Item label
        let label = SCROLL_ITEM_LABELS[i % SCROLL_ITEM_LABELS.len()];
        TextLabel::new(label, item_rect.x + 14.0, item_rect.y + 8.0)
            .size(FontSize::Caption)
            .color(fox.text)
            .max_width(item_rect.w - 28.0)
            .draw(text_renderer, size.0, size.1);
    }
    area.end(painter);
    scrollbar.draw(painter, sb_state, fox);

    // ── Texture + clipping demo ──────────────────────────────────────────
    let tex_panel = Rect::new(
        scroll_col.x,
        scroll_col.y + scroll_h + SECTION_V_GAP,
        scroll_col.w,
        (scroll_col.h - scroll_h - SECTION_V_GAP).max(60.0),
    );

    Panel::new(tex_panel)
        .fill(Fill::vertical(fox.surface_2, fox.surface))
        .radius(SUB_PANEL_RADIUS)
        .draw(painter);
    painter.rect_stroke(tex_panel, SUB_PANEL_RADIUS, 1.0, border);

    TextLabel::new("Texture + Clipping", tex_panel.x + 18.0, tex_panel.y + 14.0)
        .size(FontSize::Small)
        .color(fox.text)
        .draw(text_renderer, size.0, size.1);

    let clip = Rect::new(
        tex_panel.x + 8.0,
        tex_panel.y + 40.0,
        tex_panel.w - 16.0,
        tex_panel.h - 48.0,
    );
    painter.push_clip(clip);
    // Clipped demo shapes (intentionally overflow clip bounds)
    painter.rect_filled(
        Rect::new(clip.x + clip.w - 40.0, clip.y + 10.0, 120.0, 50.0),
        8.0,
        fox.danger.with_alpha(0.7),
    );
    painter.rect_filled(
        Rect::new(clip.x + 10.0, clip.y + clip.h - 30.0, 100.0, 80.0),
        8.0,
        fox.accent.with_alpha(0.7),
    );
    painter.pop_clip();

    Some(clip)
}

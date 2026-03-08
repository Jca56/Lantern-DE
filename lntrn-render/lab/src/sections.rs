use lntrn_render::{Color, Painter, Rect, TextRenderer};
use lntrn_ui::gpu::{
    Fill, FontSize, FoxPalette, InteractionState, Panel, ScrollArea, Scrollbar, Slider, TextLabel,
};

use crate::layout::{SCROLL_DEMO_ITEM_H, SCROLL_DEMO_ITEMS};

pub fn draw_text_reference(
    painter: &mut Painter,
    text_renderer: &mut TextRenderer,
    fox: &FoxPalette,
    border: Color,
    rect: Rect,
    size: (u32, u32),
) {
    let text = fox.text;
    Panel::new(rect)
        .fill(Fill::vertical(fox.surface_2, fox.surface))
        .radius(18.0)
        .draw(painter);
    painter.rect_stroke(rect, 18.0, 1.0, border);

    let scale_x = rect.x + 18.0;
    let colors_x = rect.x + rect.w * 0.56;

    TextLabel::new("Text scale", scale_x, rect.y + 14.0)
        .size(FontSize::Small)
        .color(text)
        .draw(text_renderer, size.0, size.1);
    TextLabel::new("Text colors", colors_x, rect.y + 14.0)
        .size(FontSize::Small)
        .color(text)
        .draw(text_renderer, size.0, size.1);

    for (index, (label, font_size)) in [
        ("Heading 32px", FontSize::Heading),
        ("Subheading 28px", FontSize::Subheading),
        ("Body 24px", FontSize::Body),
        ("Small 22px", FontSize::Small),
        ("Caption 20px", FontSize::Caption),
    ]
    .into_iter()
    .enumerate()
    {
        TextLabel::new(label, scale_x, rect.y + 44.0 + index as f32 * 38.0)
            .size(font_size)
            .color(text)
            .max_width(rect.w * 0.42)
            .draw(text_renderer, size.0, size.1);
    }

    for (index, (label, text_color)) in [
        ("White", Color::WHITE),
        ("Gray 200", fox.text_secondary),
        ("Gray 144", fox.muted),
        ("Amber", fox.accent),
    ]
    .into_iter()
    .enumerate()
    {
        TextLabel::new(label, colors_x, rect.y + 44.0 + index as f32 * 38.0)
            .size(FontSize::Caption)
            .color(text_color)
            .draw(text_renderer, size.0, size.1);
    }
}

pub fn draw_slider_section(
    painter: &mut Painter,
    text_renderer: &mut TextRenderer,
    fox: &FoxPalette,
    border: Color,
    panel_rect: Rect,
    slider_rect: Rect,
    slider_value: f32,
    hovered: bool,
    active: bool,
    size: (u32, u32),
) {
    let text = fox.text;
    Panel::new(panel_rect)
        .fill(Fill::vertical(fox.surface_2, fox.surface))
        .radius(18.0)
        .draw(painter);
    painter.rect_stroke(panel_rect, 18.0, 1.0, border);

    TextLabel::new("Gold slider", panel_rect.x + 18.0, panel_rect.y + 14.0)
        .size(FontSize::Small)
        .color(text)
        .draw(text_renderer, size.0, size.1);
    TextLabel::new(
        &format!("{:.0}%", slider_value * 100.0),
        panel_rect.x + panel_rect.w - 88.0,
        panel_rect.y + 14.0,
    )
        .size(FontSize::Small)
        .color(fox.accent)
        .max_width(70.0)
        .draw(text_renderer, size.0, size.1);
    Slider::new(slider_rect)
        .value(slider_value)
        .hovered(hovered)
        .active(active)
        .draw(painter, fox);
}

pub fn draw_texture_demo(
    painter: &mut Painter,
    text_renderer: &mut TextRenderer,
    border: Color,
    text_color: Color,
    panel_rect: Rect,
    size: (u32, u32),
) -> Rect {
    Panel::new(panel_rect)
        .fill(Fill::vertical(Color::from_rgb8(51, 51, 51), Color::from_rgb8(39, 39, 39)))
        .radius(18.0)
        .draw(painter);
    painter.rect_stroke(panel_rect, 18.0, 1.0, border);

    TextLabel::new("Texture + Clipping", panel_rect.x + 18.0, panel_rect.y + 14.0)
        .size(FontSize::Small)
        .color(text_color)
        .draw(text_renderer, size.0, size.1);

    let clip = Rect::new(
        panel_rect.x + 8.0,
        panel_rect.y + 40.0,
        panel_rect.w - 16.0,
        panel_rect.h - 48.0,
    );
    painter.push_clip(clip);
    painter.rect_filled(
        Rect::new(clip.x + clip.w - 40.0, clip.y + 10.0, 120.0, 50.0),
        8.0,
        Color::from_rgb8(218, 54, 51).with_alpha(0.7),
    );
    painter.rect_filled(
        Rect::new(clip.x + 10.0, clip.y + clip.h - 30.0, 100.0, 80.0),
        8.0,
        Color::from_rgb8(88, 166, 255).with_alpha(0.7),
    );
    painter.pop_clip();
    clip
}

pub fn draw_scroll_demo(
    painter: &mut Painter,
    text_renderer: &mut TextRenderer,
    fox: &FoxPalette,
    border: Color,
    panel_rect: Rect,
    area: &ScrollArea,
    scrollbar: &Scrollbar,
    scrollbar_state: InteractionState,
    size: (u32, u32),
) {
    let text = fox.text;
    let muted = fox.muted;
    let viewport = area.viewport;

    Panel::new(panel_rect)
        .fill(Fill::vertical(fox.surface_2, fox.surface))
        .radius(18.0)
        .draw(painter);
    painter.rect_stroke(panel_rect, 18.0, 1.0, border);

    TextLabel::new("Scroll Area", panel_rect.x + 18.0, panel_rect.y + 14.0)
        .size(FontSize::Small)
        .color(text)
        .draw(text_renderer, size.0, size.1);
    TextLabel::new(
        &format!("offset: {:.0}/{:.0}", area.offset, area.max_offset()),
        panel_rect.x + panel_rect.w - 180.0,
        panel_rect.y + 14.0,
    )
        .size(FontSize::Caption)
        .color(muted)
        .draw(text_renderer, size.0, size.1);

    area.begin(painter);
    let cy = area.content_y();
    let item_colors = [fox.accent, fox.danger, fox.success, Color::from_rgb8(88, 166, 255)];
    for i in 0..SCROLL_DEMO_ITEMS {
        let item_y = cy + i as f32 * SCROLL_DEMO_ITEM_H;
        if item_y + SCROLL_DEMO_ITEM_H < viewport.y || item_y > viewport.y + viewport.h {
            continue;
        }
        let item_rect = Rect::new(
            viewport.x + 4.0,
            item_y + 2.0,
            viewport.w - 20.0,
            SCROLL_DEMO_ITEM_H - 4.0,
        );
        let color = item_colors[i % item_colors.len()];
        painter.rect_filled(item_rect, 8.0, color.with_alpha(0.18));
        painter.rect_stroke(item_rect, 8.0, 1.0, color.with_alpha(0.35));
        text_renderer.queue(
            &format!("Item {} - scroll me!", i + 1),
            20.0,
            item_rect.x + 12.0,
            item_rect.y + 8.0,
            text,
            item_rect.w - 24.0,
            size.0,
            size.1,
        );
    }
    area.end(painter);
    scrollbar.draw(painter, scrollbar_state, fox);
}
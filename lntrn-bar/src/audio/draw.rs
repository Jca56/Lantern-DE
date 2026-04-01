//! Audio popup rendering — bar icon, popup, sliders.

use lntrn_render::{Color, GpuContext, Painter, Rect, TextRenderer, TextureDraw, TexturePass};
use lntrn_ui::gpu::{FoxPalette, InteractionContext};

use crate::svg_icon::IconCache;
use crate::mpris::PlaybackStatus;
use super::{
    Audio, ZONE_AUDIO_ICON, ZONE_VOL_SLIDER, ZONE_MUTE_BTN, ZONE_MIC_SLIDER, ZONE_MIC_MUTE,
    ZONE_SINK_BASE, ZONE_SOURCE_BASE, ZONE_STREAM_SLIDER_BASE,
    ZONE_MEDIA_PREV, ZONE_MEDIA_PLAY, ZONE_MEDIA_NEXT,
};

fn icon_dir() -> std::path::PathBuf { crate::lantern_icons_dir() }

// ── Layout constants (pre-scale) ───────────────────────────────────────────

const PAD: f32 = 20.0;
const CORNER_R: f32 = 12.0;
const GAP: f32 = 8.0;
const POPUP_W: f32 = 360.0;
const TITLE_FONT: f32 = 24.0;
const BODY_FONT: f32 = 20.0;
const SMALL_FONT: f32 = 16.0;
const SLIDER_H: f32 = 12.0;
const SECTION_GAP: f32 = 16.0;
const ROW_H: f32 = 42.0;
const MUTE_BTN_SIZE: f32 = 32.0;
const STREAM_ROW_H: f32 = 48.0;
const MEDIA_BTN_SIZE: f32 = 36.0;
const MEDIA_BTN_GAP: f32 = 16.0;

impl Audio {
    pub fn load_icons(
        &mut self, icons: &mut IconCache, tex_pass: &TexturePass, gpu: &GpuContext, size: u32,
    ) {
        if self.icons_loaded { return; }
        let dir = icon_dir();
        for (key, file) in [
            ("sound-high", "spark-sound-high.svg"),
            ("sound-medium", "spark-sound-medium.svg"),
            ("sound-low", "spark-sound-low.svg"),
            ("sound-muted", "spark-sound-muted.svg"),
        ] {
            icons.load(tex_pass, gpu, key, &dir.join(file), size, size);
        }
        self.icons_loaded = true;
    }

    pub fn measure(&self, bar_h: f32, scale: f32) -> f32 {
        let pad = 9.0 * scale;
        (bar_h - pad * 2.0).max(16.0)
    }

    pub fn draw<'a>(
        &self, _painter: &mut Painter, _text: &mut TextRenderer,
        ix: &mut InteractionContext, icons: &'a IconCache, _palette: &FoxPalette,
        x: f32, bar_y: f32, bar_h: f32, scale: f32, _screen_w: u32, _screen_h: u32,
    ) -> (f32, Vec<TextureDraw<'a>>) {
        let pad = 9.0 * scale;
        let icon_size = (bar_h - pad * 2.0).max(16.0);
        let icon_y = bar_y + pad;
        let mut tex_draws = Vec::new();
        if let Some(tex) = icons.get(self.icon_key()) {
            tex_draws.push(TextureDraw::new(tex, x, icon_y, icon_size, icon_size));
        }
        ix.add_zone(ZONE_AUDIO_ICON, Rect::new(x, icon_y, icon_size, icon_size));
        (icon_size, tex_draws)
    }

    /// Shared height calculation used by both draw_popup and popup_rect.
    fn content_height(&self, scale: f32) -> f32 {
        let title_font = TITLE_FONT * scale;
        let body_font = BODY_FONT * scale;
        let small_font = SMALL_FONT * scale;
        let slider_h = SLIDER_H * scale;
        let section_gap = SECTION_GAP * scale;
        let row_h = ROW_H * scale;
        let stream_row_h = STREAM_ROW_H * scale;

        let mut h = 0.0;

        // Now-playing section (only when media active)
        if self.media.is_some() {
            let btn_sz = MEDIA_BTN_SIZE * scale;
            h += body_font + 4.0 * scale;              // track title
            h += small_font + section_gap * 0.5;         // artist
            h += btn_sz + section_gap;                   // transport buttons
            h += 1.0 * scale + section_gap;              // separator
        }

        h += title_font + section_gap;       // "Volume XX%"
        h += slider_h + section_gap;                 // volume slider
        h += 1.0 * scale + section_gap;              // separator
        h += body_font + section_gap;                 // "Microphone"
        h += slider_h + section_gap;                  // mic slider

        if !self.sinks.is_empty() {
            h += 1.0 * scale + section_gap;           // separator
            h += small_font + section_gap * 0.5;      // "Output"
            h += self.sinks.len() as f32 * row_h;
        }

        if !self.sources.is_empty() {
            h += 1.0 * scale + section_gap;           // separator
            h += small_font + section_gap * 0.5;      // "Input"
            h += self.sources.len() as f32 * row_h;
        }

        if !self.streams.is_empty() {
            h += 1.0 * scale + section_gap;           // separator
            h += small_font + section_gap * 0.5;      // "Apps"
            h += self.streams.len() as f32 * stream_row_h;
        }

        h
    }

    pub fn draw_popup(
        &mut self, painter: &mut Painter, text: &mut TextRenderer,
        ix: &mut InteractionContext, palette: &FoxPalette,
        audio_x: f32, audio_w: f32, bar_y: f32, scale: f32,
        screen_w: u32, screen_h: u32,
    ) {
        if !self.open { return; }

        let pad = PAD * scale;
        let corner_r = CORNER_R * scale;
        let gap = GAP * scale;
        let popup_w = POPUP_W * scale;
        let title_font = TITLE_FONT * scale;
        let body_font = BODY_FONT * scale;
        let small_font = SMALL_FONT * scale;
        let slider_h = SLIDER_H * scale;
        let section_gap = SECTION_GAP * scale;
        let row_h = ROW_H * scale;
        let mute_btn_size = MUTE_BTN_SIZE * scale;
        let stream_row_h = STREAM_ROW_H * scale;

        let content_h = self.content_height(scale);
        let popup_h = pad * 2.0 + content_h;
        let popup_x = (audio_x + audio_w / 2.0 - popup_w / 2.0)
            .max(gap)
            .min(screen_w as f32 - popup_w - gap);
        let popup_y = (bar_y - popup_h - gap).max(0.0);

        // Shadow + background
        let shadow_expand = 3.0 * scale;
        painter.rect_filled(
            Rect::new(popup_x - shadow_expand, popup_y + shadow_expand,
                popup_w + shadow_expand * 2.0, popup_h + shadow_expand),
            corner_r + 2.0, Color::BLACK.with_alpha(0.35),
        );
        let bg = Rect::new(popup_x, popup_y, popup_w, popup_h);
        painter.rect_filled(bg, corner_r, palette.bg);
        painter.rect_stroke_sdf(bg, corner_r, 1.0 * scale, Color::WHITE.with_alpha(0.08));

        let cx = popup_x + pad;
        let cw = popup_w - pad * 2.0;
        let mut y = popup_y + pad;

        // ── Now Playing section (only when media active) ──
        if let Some(ref media) = self.media {
            let title = if media.title.is_empty() { "Unknown Track" } else { &media.title };
            text.queue(title, body_font, cx, y, palette.text, cw, screen_w, screen_h);
            y += body_font + 4.0 * scale;

            let subtitle = if media.artist.is_empty() {
                media.player_name.clone()
            } else {
                format!("{} \u{2022} {}", media.artist, media.player_name)
            };
            text.queue(&subtitle, small_font, cx, y, palette.text_secondary, cw, screen_w, screen_h);
            y += small_font + section_gap * 0.5;

            // Transport buttons: [prev] [play/pause] [next]
            let btn_sz = MEDIA_BTN_SIZE * scale;
            let btn_gap = MEDIA_BTN_GAP * scale;
            let total_btns_w = btn_sz * 3.0 + btn_gap * 2.0;
            let btn_x = cx + (cw - total_btns_w) / 2.0;

            draw_media_btn(painter, ix, palette, btn_x, y, btn_sz, scale,
                ZONE_MEDIA_PREV, MediaIcon::Prev);
            let play_icon = if media.status == PlaybackStatus::Playing {
                MediaIcon::Pause
            } else {
                MediaIcon::Play
            };
            draw_media_btn(painter, ix, palette, btn_x + btn_sz + btn_gap, y, btn_sz, scale,
                ZONE_MEDIA_PLAY, play_icon);
            draw_media_btn(painter, ix, palette, btn_x + (btn_sz + btn_gap) * 2.0, y, btn_sz, scale,
                ZONE_MEDIA_NEXT, MediaIcon::Next);

            y += btn_sz + section_gap;

            // Separator
            painter.rect_filled(Rect::new(cx, y, cw, 1.0 * scale), 0.0,
                palette.muted.with_alpha(0.2));
            y += 1.0 * scale + section_gap;
        }

        // ── Volume section ──
        let vol_pct = (self.volume * 100.0).round() as u32;
        let vol_label = if self.muted {
            "Volume — Muted".to_string()
        } else {
            format!("Volume — {}%", vol_pct)
        };
        text.queue(&vol_label, title_font, cx, y, palette.text, cw - mute_btn_size - gap, screen_w, screen_h);

        // Mute button
        let mute_x = cx + cw - mute_btn_size;
        let mute_y = y + (title_font - mute_btn_size) / 2.0;
        let mute_rect = Rect::new(mute_x, mute_y, mute_btn_size, mute_btn_size);
        let mute_state = ix.add_zone(ZONE_MUTE_BTN, mute_rect);
        let mute_hovered = mute_state.is_hovered();
        let mute_bg = if mute_hovered {
            palette.muted.with_alpha(0.3)
        } else if self.muted {
            palette.danger.with_alpha(0.2)
        } else {
            Color::TRANSPARENT
        };
        painter.rect_filled(mute_rect, 6.0 * scale, mute_bg);
        let mute_label = if self.muted { "🔇" } else { "🔊" };
        let mute_ty = mute_y + (mute_btn_size - small_font) / 2.0;
        text.queue(mute_label, small_font, mute_x + 6.0 * scale, mute_ty,
            if self.muted { palette.danger } else { palette.text }, mute_btn_size, screen_w, screen_h);

        y += title_font + section_gap;

        // Volume slider
        self.vol_slider_rect = Some(Rect::new(cx, y, cw, slider_h));
        self.draw_slider(painter, ix, palette, cx, y, cw, slider_h, scale,
            self.volume, self.muted, ZONE_VOL_SLIDER);
        y += slider_h + section_gap;

        // Separator
        painter.rect_filled(Rect::new(cx, y, cw, 1.0 * scale), 0.0, palette.muted.with_alpha(0.2));
        y += 1.0 * scale + section_gap;

        // ── Microphone section ──
        let mic_pct = (self.mic_volume * 100.0).round() as u32;
        let mic_label = if self.mic_muted {
            "Microphone — Muted".to_string()
        } else {
            format!("Microphone — {}%", mic_pct)
        };
        text.queue(&mic_label, body_font, cx, y, palette.text, cw - mute_btn_size - gap, screen_w, screen_h);

        // Mic mute button
        let mic_mute_x = cx + cw - mute_btn_size;
        let mic_mute_y = y + (body_font - mute_btn_size) / 2.0;
        let mic_mute_rect = Rect::new(mic_mute_x, mic_mute_y, mute_btn_size, mute_btn_size);
        let mic_state = ix.add_zone(ZONE_MIC_MUTE, mic_mute_rect);
        let mic_hovered = mic_state.is_hovered();
        let mic_bg = if mic_hovered {
            palette.muted.with_alpha(0.3)
        } else if self.mic_muted {
            palette.danger.with_alpha(0.2)
        } else {
            Color::TRANSPARENT
        };
        painter.rect_filled(mic_mute_rect, 6.0 * scale, mic_bg);
        let mic_mute_label = if self.mic_muted { "🔇" } else { "🎤" };
        let mic_mute_ty = mic_mute_y + (mute_btn_size - small_font) / 2.0;
        text.queue(mic_mute_label, small_font, mic_mute_x + 6.0 * scale, mic_mute_ty,
            if self.mic_muted { palette.danger } else { palette.text }, mute_btn_size, screen_w, screen_h);

        y += body_font + section_gap;

        // Mic slider
        self.mic_slider_rect = Some(Rect::new(cx, y, cw, slider_h));
        self.draw_slider(painter, ix, palette, cx, y, cw, slider_h, scale,
            self.mic_volume, self.mic_muted, ZONE_MIC_SLIDER);
        y += slider_h + section_gap;

        // ── Output devices ──
        if !self.sinks.is_empty() {
            painter.rect_filled(Rect::new(cx, y, cw, 1.0 * scale), 0.0, palette.muted.with_alpha(0.2));
            y += 1.0 * scale + section_gap;

            text.queue("Output", small_font, cx, y, palette.muted, cw, screen_w, screen_h);
            y += small_font + section_gap * 0.5;

            for (idx, sink) in self.sinks.iter().enumerate() {
                self.draw_device_row(painter, text, ix, palette, scale,
                    cx, y, cw, row_h, body_font, ZONE_SINK_BASE + idx as u32,
                    &sink.name, sink.is_default, screen_w, screen_h);
                y += row_h;
            }
        }

        // ── Input devices ──
        if !self.sources.is_empty() {
            painter.rect_filled(Rect::new(cx, y, cw, 1.0 * scale), 0.0, palette.muted.with_alpha(0.2));
            y += 1.0 * scale + section_gap;

            text.queue("Input", small_font, cx, y, palette.muted, cw, screen_w, screen_h);
            y += small_font + section_gap * 0.5;

            for (idx, source) in self.sources.iter().enumerate() {
                self.draw_device_row(painter, text, ix, palette, scale,
                    cx, y, cw, row_h, body_font, ZONE_SOURCE_BASE + idx as u32,
                    &source.name, source.is_default, screen_w, screen_h);
                y += row_h;
            }
        }

        // ── Per-app streams ──
        if !self.streams.is_empty() {
            painter.rect_filled(Rect::new(cx, y, cw, 1.0 * scale), 0.0, palette.muted.with_alpha(0.2));
            y += 1.0 * scale + section_gap;

            text.queue("Apps", small_font, cx, y, palette.muted, cw, screen_w, screen_h);
            y += small_font + section_gap * 0.5;

            self.stream_slider_rects.clear();
            for (idx, stream) in self.streams.iter().enumerate() {
                let row_rect = Rect::new(cx, y, cw, stream_row_h);
                let hovered = ix.add_zone(ZONE_STREAM_SLIDER_BASE + idx as u32, row_rect)
                    .is_hovered();
                if hovered {
                    painter.rect_filled(row_rect, 8.0 * scale, palette.muted.with_alpha(0.1));
                }

                // App name
                let name_y = y + 4.0 * scale;
                text.queue(&stream.name, small_font, cx + 8.0 * scale, name_y,
                    palette.text, cw * 0.5, screen_w, screen_h);

                // Volume percentage
                let vol_pct = (stream.volume * 100.0).round() as u32;
                let vol_text = format!("{}%", vol_pct);
                let pct_x = cx + cw - 48.0 * scale;
                text.queue(&vol_text, small_font, pct_x, name_y,
                    palette.muted, 48.0 * scale, screen_w, screen_h);

                // Slider below name
                let sl_y = y + small_font + 8.0 * scale;
                let sl_x = cx + 8.0 * scale;
                let sl_w = cw - 16.0 * scale;
                let sl_h = 8.0 * scale;
                let slider_rect = Rect::new(sl_x, sl_y, sl_w, sl_h);
                self.stream_slider_rects.push((stream.id, slider_rect));
                self.draw_slider(painter, ix, palette, sl_x, sl_y, sl_w, sl_h, scale,
                    stream.volume, stream.muted, ZONE_STREAM_SLIDER_BASE + idx as u32);

                y += stream_row_h;
            }
        }
    }

    fn draw_device_row(
        &self, painter: &mut Painter, text: &mut TextRenderer,
        ix: &mut InteractionContext, palette: &FoxPalette, scale: f32,
        cx: f32, y: f32, cw: f32, row_h: f32, font: f32, zone_id: u32,
        name: &str, is_default: bool, screen_w: u32, screen_h: u32,
    ) {
        let row_rect = Rect::new(cx, y, cw, row_h);
        let state = ix.add_zone(zone_id, row_rect);
        let hovered = state.is_hovered();

        if hovered {
            painter.rect_filled(row_rect, 8.0 * scale, palette.muted.with_alpha(0.2));
        }
        if is_default {
            painter.rect_filled(row_rect, 8.0 * scale, palette.accent.with_alpha(0.12));
        }

        let text_y = y + (row_h - font) / 2.0;
        let mut lx = cx + 8.0 * scale;
        if is_default {
            text.queue("✓", font, lx, text_y, palette.accent, font, screen_w, screen_h);
            lx += font + 4.0 * scale;
        }

        let name_color = if is_default { palette.text } else { palette.text_secondary };
        text.queue(name, font, lx, text_y, name_color, cw * 0.85, screen_w, screen_h);
    }

    pub fn draw_slider(
        &self, painter: &mut Painter, ix: &mut InteractionContext, palette: &FoxPalette,
        x: f32, y: f32, w: f32, h: f32, scale: f32,
        frac: f32, is_muted: bool, zone_id: u32,
    ) {
        let track_r = h / 2.0;
        let track_rect = Rect::new(x, y, w, h);
        ix.add_zone(zone_id, track_rect);

        // Track background
        painter.rect_filled(track_rect, track_r, palette.surface);

        // Fill
        let fill_frac = frac.clamp(0.0, 1.0);
        if fill_frac > 0.0 {
            let fill_w = (w * fill_frac).max(h);
            let fill_color = if is_muted { palette.muted } else { palette.accent };
            painter.rect_filled(Rect::new(x, y, fill_w, h), track_r, fill_color);
        }

        // Thumb circle
        let thumb_x = x + w * fill_frac;
        let thumb_r = h * 0.9;
        painter.circle_filled(thumb_x, y + h / 2.0, thumb_r,
            if is_muted { palette.muted } else { palette.text });
    }

    pub fn popup_rect(
        &self, audio_x: f32, audio_w: f32, bar_y: f32, scale: f32, screen_w: u32,
    ) -> Option<Rect> {
        if !self.open { return None; }

        let pad = PAD * scale;
        let gap = GAP * scale;
        let popup_w = POPUP_W * scale;

        let content_h = self.content_height(scale);
        let popup_h = pad * 2.0 + content_h;
        let popup_x = (audio_x + audio_w / 2.0 - popup_w / 2.0)
            .max(gap)
            .min(screen_w as f32 - popup_w - gap);
        let popup_y = (bar_y - popup_h - gap).max(0.0);

        Some(Rect::new(popup_x, popup_y, popup_w, popup_h))
    }
}

// ── Media button helpers ──────────────────────────────────────────────────

enum MediaIcon { Prev, Play, Pause, Next }

fn draw_media_btn(
    painter: &mut Painter, ix: &mut InteractionContext, palette: &FoxPalette,
    x: f32, y: f32, size: f32, scale: f32, zone_id: u32, icon: MediaIcon,
) {
    let rect = Rect::new(x, y, size, size);
    let state = ix.add_zone(zone_id, rect);
    let hovered = state.is_hovered();

    if hovered {
        painter.rect_filled(rect, 8.0 * scale, palette.muted.with_alpha(0.25));
    }

    let color = if hovered { palette.accent } else { palette.text };
    let cx = x + size / 2.0;
    let cy = y + size / 2.0;
    let r = size * 0.3;

    match icon {
        MediaIcon::Play => {
            // Right-pointing triangle
            painter.triangle(
                cx - r * 0.6, cy - r,
                cx + r, cy,
                cx - r * 0.6, cy + r,
                color,
            );
        }
        MediaIcon::Pause => {
            // Two vertical bars
            let bar_w = r * 0.4;
            let bar_h = r * 1.6;
            painter.rect_filled(
                Rect::new(cx - r * 0.55, cy - bar_h / 2.0, bar_w, bar_h),
                2.0 * scale, color,
            );
            painter.rect_filled(
                Rect::new(cx + r * 0.15, cy - bar_h / 2.0, bar_w, bar_h),
                2.0 * scale, color,
            );
        }
        MediaIcon::Prev => {
            // Left-pointing triangle
            painter.triangle(
                cx + r * 0.5, cy - r * 0.8,
                cx - r * 0.5, cy,
                cx + r * 0.5, cy + r * 0.8,
                color,
            );
        }
        MediaIcon::Next => {
            // Right-pointing triangle
            painter.triangle(
                cx - r * 0.5, cy - r * 0.8,
                cx + r * 0.5, cy,
                cx - r * 0.5, cy + r * 0.8,
                color,
            );
        }
    }
}

//! Drawing for the Properties → Audio section: art tile, the two-column
//! tag grid with its Camelot chip, the stream-facts line and the
//! Save / Revert bar. State + actions live in the parent module.

use lntrn_render::{Painter, Rect, TextRenderer};
use lntrn_ui::gpu::{Button, ButtonVariant, FoxPalette, InteractionContext, TextInput};

use super::{keys, AudioEdit, Field};
use crate::{
    ZONE_PROPS_AUDIO_ART, ZONE_PROPS_AUDIO_ART_REMOVE, ZONE_PROPS_AUDIO_FIELD_BASE,
    ZONE_PROPS_AUDIO_REVERT, ZONE_PROPS_AUDIO_SAVE,
};

const ART_SIZE: f32 = 150.0;
const FIELD_H: f32 = 44.0;
const ROW_GAP: f32 = 8.0;
const ROWS: f32 = 6.0;
const LABEL_FONT: f32 = 15.0;
const CAPTION_FONT: f32 = 13.0;
const LABEL_W: f32 = 62.0;
const SHORT_LABEL_W: f32 = 52.0;
const GAP: f32 = 14.0;
const TOP_PAD: f32 = 6.0;
const BTN_H: f32 = 40.0;
const BTN_W: f32 = 110.0;
const CHIP_W: f32 = 50.0;
/// Status text and the notice under the action bar. The notice is what
/// says why a save was refused, so it is sized to be read.
const STATUS_FONT: f32 = 18.0;
const NOTICE_LINE_H: f32 = 26.0;
const NOTICE_PAD: f32 = 12.0;
const NOTICE_GAP: f32 = 4.0;

impl AudioEdit {
    pub fn body_height(&self, s: f32) -> f32 {
        (TOP_PAD + ROWS * (FIELD_H + ROW_GAP) + LABEL_FONT + 10.0 + BTN_H + 8.0) * s
            + self.notice_height(s)
    }

    /// Height of the error notice under the action bar, 0 without one.
    fn notice_height(&self, s: f32) -> f32 {
        match &self.status {
            Some(st) if st.is_err && !st.lines.is_empty() => {
                (NOTICE_GAP + 2.0 * NOTICE_PAD + st.lines.len() as f32 * NOTICE_LINE_H + 8.0) * s
            }
            _ => 0.0,
        }
    }

    /// Draw the section body at (x, y) spanning `w`; returns the height used.
    #[allow(clippy::too_many_arguments)]
    pub fn draw(
        &mut self,
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
        let art_sz = ART_SIZE * s;
        let gap = GAP * s;
        let field_h = FIELD_H * s;
        let row_gap = ROW_GAP * s;
        let label_w = LABEL_W * s;
        let short_w = SHORT_LABEL_W * s;
        let fields_x = x + art_sz + gap;
        let fields_w = w - art_sz - gap;
        let top = y + TOP_PAD * s;

        self.draw_art_tile(painter, text, ix, fox, x, top, art_sz, s, sw, sh);

        let mut fy = top;
        for f in [Field::Title, Field::Artist, Field::Album, Field::Genre] {
            self.draw_field(f, fields_x, fy, fields_w, label_w, painter, text, ix, fox, s, sw, sh);
            fy += field_h + row_gap;
        }
        let half = (fields_w - gap) / 2.0;
        let right_x = fields_x + half + gap;
        self.draw_field(Field::Year, fields_x, fy, half, short_w, painter, text, ix, fox, s, sw, sh);
        self.draw_field(Field::Track, right_x, fy, half, short_w, painter, text, ix, fox, s, sw, sh);
        fy += field_h + row_gap;
        self.draw_field(Field::Bpm, fields_x, fy, half, short_w, painter, text, ix, fox, s, sw, sh);
        let chip_w = CHIP_W * s;
        let key_w = half - chip_w - 6.0 * s;
        self.draw_field(Field::Key, right_x, fy, key_w, short_w, painter, text, ix, fox, s, sw, sh);
        self.draw_key_chip(painter, text, fox, right_x + half - chip_w, fy, chip_w, field_h, s, sw, sh);
        fy += field_h + row_gap;

        // Read-only stream facts: duration · rate · depth · channels · codec.
        let font = LABEL_FONT * s;
        text.queue(&self.summary, font, x, fy, fox.text_secondary, w, sw, sh);
        fy += font + 10.0 * s;

        self.draw_action_bar(painter, text, ix, fox, x, fy, w, s, sw, sh);
        fy += BTN_H * s + 8.0 * s;
        fy += self.draw_notice(painter, text, fox, x, fy, w, s, sw, sh);
        fy - y
    }

    /// An error, in full: a refused save says why, and that takes a few
    /// lines. (It used to be one clipped line beside the buttons.) Returns
    /// the height used, which is what `notice_height` promised.
    #[allow(clippy::too_many_arguments)]
    fn draw_notice(
        &mut self,
        painter: &mut Painter,
        text: &mut TextRenderer,
        fox: &FoxPalette,
        x: f32,
        y: f32,
        w: f32,
        s: f32,
        sw: u32,
        sh: u32,
    ) -> f32 {
        let height = self.notice_height(s);
        let Some(st) = self.status.as_mut().filter(|st| st.is_err) else {
            return 0.0;
        };
        if height == 0.0 {
            return 0.0;
        }
        let font = STATUS_FONT * s;
        let pad = NOTICE_PAD * s;
        let bar = 5.0 * s;
        // The slack keeps the text renderer from wrapping a line that
        // measures exactly as wide as its box.
        let avail = w - bar - 2.0 * pad - 4.0 * s;
        if st.lines.iter().any(|l| text.measure_width(l, font) > avail) {
            // The first break was by character count; this text is wider
            // than that allows for. Break it by its real width. The height
            // reserved for this frame was for the old line count, so ask
            // for one more frame (once: a single word wider than the panel
            // stays too wide however often it is re-broken).
            let before = st.lines.len();
            st.rewrap(|l| text.measure_width(l, font) <= avail);
            if st.lines.len() != before {
                self.relayout.set(true);
            }
        }
        let top = y + NOTICE_GAP * s;
        let lines_h = st.lines.len() as f32 * NOTICE_LINE_H * s;
        let panel = Rect::new(x, top, w, lines_h + 2.0 * pad);
        painter.rect_filled(panel, 8.0 * s, fox.danger.with_alpha(0.14));
        painter.rect_filled(Rect::new(x, top, bar, panel.h), 2.0 * s, fox.danger);
        let line_h = NOTICE_LINE_H * s;
        for (i, line) in st.lines.iter().enumerate() {
            let ly = top + pad + i as f32 * line_h + (line_h - font) / 2.0;
            text.queue(line, font, x + bar + pad, ly, fox.text, avail + 4.0 * s, sw, sh);
        }
        height
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_art_tile(
        &mut self,
        painter: &mut Painter,
        text: &mut TextRenderer,
        ix: &mut InteractionContext,
        fox: &FoxPalette,
        x: f32,
        y: f32,
        size: f32,
        s: f32,
        sw: u32,
        sh: u32,
    ) {
        let tile = Rect::new(x, y, size, size);
        let zone = ix.add_zone(ZONE_PROPS_AUDIO_ART, tile);
        let radius = 10.0 * s;
        painter.rect_filled(tile, radius, fox.surface_2);
        let inset = 3.0 * s;
        self.art_rect = Some(Rect::new(
            tile.x + inset,
            tile.y + inset,
            size - inset * 2.0,
            size - inset * 2.0,
        ));
        if self.texture.is_none() {
            // Vinyl placeholder: disc, label, spindle hole.
            let cx = tile.x + size / 2.0;
            let cy = tile.y + size / 2.0;
            for (d, color) in [
                (size * 0.72, fox.muted.with_alpha(0.25)),
                (size * 0.28, fox.accent.with_alpha(0.6)),
                (size * 0.05, fox.surface_2),
            ] {
                painter.rect_filled(Rect::new(cx - d / 2.0, cy - d / 2.0, d, d), d / 2.0, color);
            }
        }
        if zone.is_hovered() {
            painter.rect_stroke_sdf(tile, radius, 2.0 * s, fox.accent);
        }

        let cap_font = CAPTION_FONT * s;
        let caption = if self.picking {
            "Choosing…"
        } else if self.texture.is_some() {
            "Click to change"
        } else {
            "Click to add art"
        };
        let cw = text.measure_width(caption, cap_font);
        let cap_y = tile.y + size + 6.0 * s;
        text.queue(
            caption,
            cap_font,
            tile.x + (size - cw) / 2.0,
            cap_y,
            fox.text_secondary,
            size,
            sw,
            sh,
        );
        if self.texture.is_some() {
            let lbl = "Remove";
            let lw = text.measure_width(lbl, cap_font);
            let ry = cap_y + cap_font + 6.0 * s;
            let rr = Rect::new(
                tile.x + (size - lw) / 2.0 - 8.0 * s,
                ry - 3.0 * s,
                lw + 16.0 * s,
                cap_font + 6.0 * s,
            );
            let rz = ix.add_zone(ZONE_PROPS_AUDIO_ART_REMOVE, rr);
            if rz.is_hovered() {
                painter.rect_filled(rr, 4.0 * s, fox.danger.with_alpha(0.15));
            }
            let color = if rz.is_hovered() {
                fox.danger
            } else {
                fox.text_secondary
            };
            text.queue(lbl, cap_font, rr.x + 8.0 * s, ry, color, lw + 8.0 * s, sw, sh);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_field(
        &self,
        field: Field,
        x: f32,
        y: f32,
        w: f32,
        label_w: f32,
        painter: &mut Painter,
        text: &mut TextRenderer,
        ix: &mut InteractionContext,
        fox: &FoxPalette,
        s: f32,
        sw: u32,
        sh: u32,
    ) {
        let idx = field as usize;
        let field_h = FIELD_H * s;
        let font = LABEL_FONT * s;
        text.queue(
            field.label(),
            font,
            x,
            y + (field_h - font) / 2.0,
            fox.text_secondary,
            label_w,
            sw,
            sh,
        );
        let rect = Rect::new(x + label_w, y, (w - label_w).max(20.0 * s), field_h);
        let zone = ix.add_zone(ZONE_PROPS_AUDIO_FIELD_BASE + idx as u32, rect);
        TextInput::new(rect)
            .text(&self.bufs[idx])
            .placeholder(field.placeholder())
            .focused(self.focused == Some(idx))
            .hovered(zone.is_hovered())
            .cursor_pos(self.cursors[idx])
            .scale(s)
            .draw(painter, text, fox, sw, sh);
    }

    /// Shows the *other* notation next to the Key field: type "F#m" and the
    /// chip says 11A; type "11A" and it says F#m. "?" when unparseable.
    #[allow(clippy::too_many_arguments)]
    fn draw_key_chip(
        &self,
        painter: &mut Painter,
        text: &mut TextRenderer,
        fox: &FoxPalette,
        x: f32,
        y: f32,
        w: f32,
        field_h: f32,
        s: f32,
        sw: u32,
        sh: u32,
    ) {
        let raw = self.bufs[Field::Key as usize].trim();
        if raw.is_empty() {
            return;
        }
        let typed_camelot = raw.chars().next().is_some_and(|c| c.is_ascii_digit());
        let (label, bg, fg) = match keys::normalize(raw) {
            Some(k) => (
                if typed_camelot { k.musical } else { k.camelot },
                fox.accent.with_alpha(0.18),
                fox.accent,
            ),
            None => ("?", fox.warning.with_alpha(0.18), fox.warning),
        };
        let chip_h = 30.0 * s;
        let r = Rect::new(x, y + (field_h - chip_h) / 2.0, w, chip_h);
        painter.rect_filled(r, 6.0 * s, bg);
        let font = 15.0 * s;
        let lw = text.measure_width(label, font);
        text.queue(
            label,
            font,
            r.x + (r.w - lw) / 2.0,
            r.y + (r.h - font) / 2.0,
            fg,
            r.w,
            sw,
            sh,
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_action_bar(
        &self,
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
    ) {
        let btn_h = BTN_H * s;
        let btn_w = BTN_W * s;
        let font = STATUS_FONT * s;
        let ty = y + (btn_h - font) / 2.0;
        if self.saving {
            text.queue("Saving…", font, x, ty, fox.text_secondary, w, sw, sh);
            return;
        }
        // Errors get the notice below the bar; this is for "Saved".
        if let Some(st) = self.status.as_ref().filter(|st| !st.is_err) {
            text.queue(&st.lines.join(" "), font, x, ty, fox.success, w * 0.6, sw, sh);
        }
        if !self.is_dirty() {
            return;
        }
        let save = Rect::new(x + w - btn_w, y, btn_w, btn_h);
        let sz = ix.add_zone(ZONE_PROPS_AUDIO_SAVE, save);
        Button::new(save, "Save")
            .variant(ButtonVariant::Primary)
            .hovered(sz.is_hovered())
            .pressed(sz.is_active())
            .scale(s)
            .draw(painter, text, fox, sw, sh);
        let revert = Rect::new(save.x - btn_w - 10.0 * s, y, btn_w, btn_h);
        let rz = ix.add_zone(ZONE_PROPS_AUDIO_REVERT, revert);
        Button::new(revert, "Revert")
            .hovered(rz.is_hovered())
            .pressed(rz.is_active())
            .scale(s)
            .draw(painter, text, fox, sw, sh);
    }
}

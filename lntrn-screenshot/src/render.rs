//! Drawing for the screenshot overlay: the dim, the selection rectangle with
//! its handles and size readout, the window-pick highlight, and the toolbar.

use lntrn_render::{
    Color, GpuContext, Painter, Rect, SurfaceError, TextRenderer, TextureDraw, TexturePass,
};

use crate::selection::HANDLE_SIZE;
use crate::toolbar::{ToolbarAction, ToolbarLayout};
use crate::{SelectionUi, UiMode};

// Lantern look. Inlined to avoid pulling lntrn-ui / lntrn-theme into a
// tool this small (per project convention: apps style directly with
// Painter rather than depending on the shared theme crates).
// Built via `from_rgba8` (sRGB → linear), so these can't be `const`.
fn text_tan() -> Color {
    Color::from_rgba8(0xe8, 0xdc, 0xc8, 0xff)
}
fn accent_orange() -> Color {
    Color::from_rgba8(0xff, 0x9b, 0x42, 0xff)
}

impl SelectionUi {
    /// Draw the window-pick overlay: dim everything except the hovered
    /// window, outline it, and label it. With nothing hovered, dim the lot.
    fn render_pick_window(
        &self,
        painter: &mut Painter,
        text: &mut TextRenderer,
        sw: f32,
        sh: f32,
        scale: f32,
        dim: Color,
    ) {
        let Some(idx) = self.hover_window else {
            painter.rect_filled(Rect::new(0.0, 0.0, sw, sh), 0.0, dim);
            return;
        };
        let w = &self.windows[idx];
        // Clamp the highlight to the screen so the dim cut-out lines up.
        let x = w.x.max(0.0);
        let y = w.y.max(0.0);
        let rw = (w.x + w.w).min(sw) - x;
        let rh = (w.y + w.h).min(sh) - y;

        dim_around(painter, sw, sh, x, y, rw, rh, dim);
        let stroke = (3.0 * scale).max(2.0);
        painter.rect_stroke(Rect::new(x, y, rw, rh), 0.0, stroke, accent_orange());

        if !w.label.is_empty() {
            let lf = 20.0 * scale.max(1.0);
            let lp = 8.0 * scale.max(1.0);
            let lw = text.measure_width(&w.label, lf) + lp * 2.0;
            let lh = lf + lp * 2.0;
            let ly = if y > lh + 4.0 { y - lh - 4.0 } else { y + 4.0 };
            painter.rect_filled(
                Rect::new(x, ly, lw, lh),
                6.0 * scale.max(1.0),
                Color::from_rgba8(0, 0, 0, 210),
            );
            text.queue(
                &w.label,
                lf,
                x + lp,
                ly + lp,
                text_tan(),
                lw,
                sw as u32,
                sh as u32,
            );
        }
    }

    pub(crate) fn render(
        &self,
        gpu: &mut GpuContext,
        painter: &mut Painter,
        text: &mut TextRenderer,
        tex_pass: &TexturePass,
        screenshot_tex: &lntrn_render::GpuTexture,
        scale: f32,
    ) -> Result<(), SurfaceError> {
        let sw = gpu.width() as f32;
        let sh = gpu.height() as f32;
        let dim = Color::from_rgba8(0, 0, 0, 140);

        painter.clear();

        if self.mode == UiMode::PickWindow {
            self.render_pick_window(painter, text, sw, sh, scale, dim);
        } else if let Some(ref sel) = self.selection {
            let (sx, sy, sw_, sh_) = sel.normalized();

            painter.rect_filled(Rect::new(0.0, 0.0, sw, sy), 0.0, dim);
            painter.rect_filled(Rect::new(0.0, sy + sh_, sw, sh - sy - sh_), 0.0, dim);
            painter.rect_filled(Rect::new(0.0, sy, sx, sh_), 0.0, dim);
            painter.rect_filled(Rect::new(sx + sw_, sy, sw - sx - sw_, sh_), 0.0, dim);

            let stroke = (2.0 * scale).max(2.0);
            painter.rect_stroke(Rect::new(sx, sy, sw_, sh_), 0.0, stroke, accent_orange());

            // Drag handles — scaled with the output so they're easy to grab on HiDPI.
            let hs = HANDLE_SIZE * scale.max(1.0);
            let half = hs / 2.0;
            let handles = [
                (sx - half, sy - half),
                (sx + sw_ / 2.0 - half, sy - half),
                (sx + sw_ - half, sy - half),
                (sx + sw_ - half, sy + sh_ / 2.0 - half),
                (sx + sw_ - half, sy + sh_ - half),
                (sx + sw_ / 2.0 - half, sy + sh_ - half),
                (sx - half, sy + sh_ - half),
                (sx - half, sy + sh_ / 2.0 - half),
            ];
            for (hx, hy) in handles {
                painter.rect_filled(Rect::new(hx, hy, hs, hs), 2.0, Color::WHITE);
                painter.rect_stroke(Rect::new(hx, hy, hs, hs), 2.0, 1.0 * scale, accent_orange());
            }

            // Size readout above (or below) the selection.
            let label = format!("{} x {}", sw_ as u32, sh_ as u32);
            let label_font = 18.0 * scale.max(1.0);
            let label_pad = 6.0 * scale.max(1.0);
            let label_box_w = 180.0 * scale.max(1.0);
            let label_box_h = label_font + label_pad * 2.0;
            let label_y = if sy > label_box_h + 4.0 {
                sy - label_box_h - 4.0
            } else {
                sy + sh_ + 4.0
            };
            painter.rect_filled(
                Rect::new(sx, label_y, label_box_w, label_box_h),
                4.0 * scale.max(1.0),
                Color::from_rgba8(0, 0, 0, 200),
            );
            text.queue(
                &label,
                label_font,
                sx + label_pad,
                label_y + label_pad,
                text_tan(),
                label_box_w - label_pad * 2.0,
                sw as u32,
                sh as u32,
            );
        } else {
            painter.rect_filled(Rect::new(0.0, 0.0, sw, sh), 0.0, dim);
        }

        // Toolbar pill, drawn on top of the dim / selection.
        let toolbar = ToolbarLayout::compute(sw, sh, scale);
        let active = (self.mode == UiMode::PickWindow).then_some(ToolbarAction::Window);
        toolbar.render(painter, text, self.cursor, active, sw as u32, sh as u32);

        let mut frame = gpu.begin_frame("screenshot")?;
        let view = frame.view().clone();

        {
            let encoder = frame.encoder_mut();
            let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("clear"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                ..Default::default()
            });
        }

        let tex_draw = TextureDraw::new(screenshot_tex, 0.0, 0.0, sw, sh);
        tex_pass.render_pass(gpu, frame.encoder_mut(), &view, &[tex_draw], None);
        painter.render_pass_overlay(gpu, frame.encoder_mut(), &view);
        text.render_queued(gpu, frame.encoder_mut(), &view);

        frame.submit(&gpu.queue);
        Ok(())
    }
}

/// Fill the four bands around an inner rect with `dim`, leaving the rect
/// itself untouched. Shared by region-selection and window-pick rendering.
fn dim_around(painter: &mut Painter, sw: f32, sh: f32, x: f32, y: f32, w: f32, h: f32, dim: Color) {
    painter.rect_filled(Rect::new(0.0, 0.0, sw, y), 0.0, dim);
    painter.rect_filled(Rect::new(0.0, y + h, sw, sh - y - h), 0.0, dim);
    painter.rect_filled(Rect::new(0.0, y, x, h), 0.0, dim);
    painter.rect_filled(Rect::new(x + w, y, sw - x - w, h), 0.0, dim);
}

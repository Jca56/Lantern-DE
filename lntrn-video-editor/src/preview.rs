//! Preview monitor — manages the GPU texture for the current video frame
//! and renders it aspect-fit in the preview panel area.

use lntrn_render::{
    Color, GpuContext, GpuTexture, Painter, Rect, TextRenderer, TextureDraw, TexturePass,
};

use crate::chrome;
use crate::decoder::DecodedFrame;
use crate::layout::PANEL_PAD;
use crate::playback::{self, PlayState, Playback};

pub struct PreviewMonitor {
    pub tex_pass: TexturePass,
    pub video_texture: Option<GpuTexture>,
}

impl PreviewMonitor {
    pub fn new(gpu: &GpuContext) -> Self {
        Self {
            tex_pass: TexturePass::new(gpu),
            video_texture: None,
        }
    }

    /// Upload a decoded frame to the GPU as a texture.
    pub fn upload_frame(&mut self, gpu: &GpuContext, frame: &DecodedFrame) {
        self.video_texture = Some(
            self.tex_pass.upload(gpu, &frame.rgba, frame.width, frame.height),
        );
    }

    /// Draw the preview panel: black letterbox + video frame + timecode.
    /// Queues painter/text draws but does NOT execute the texture render pass yet.
    pub fn draw(
        &self,
        p: &mut Painter,
        t: &mut TextRenderer,
        preview_rect: &Rect,
        playback: &Playback,
        s: f32,
        sw: u32,
        sh: u32,
    ) {
        let pad = PANEL_PAD * s;
        let accent = chrome::accent();
        let text_dim = chrome::text_dim();

        // Panel background
        p.rect_filled(*preview_rect, 0.0, chrome::BG);

        // Compute aspect-fit rect for the video (or default 16:9)
        let transport_h = 44.0 * s;
        let avail_w = preview_rect.w - pad * 2.0;
        let avail_h = preview_rect.h - pad * 2.0 - transport_h;

        let (vw, vh) = if playback.has_media() {
            (playback.video_width as f32, playback.video_height as f32)
        } else {
            (16.0, 9.0) // default aspect
        };
        let aspect = vw / vh;

        let (pw, ph) = if avail_w / avail_h > aspect {
            (avail_h * aspect, avail_h)
        } else {
            (avail_w, avail_w / aspect)
        };
        let px = preview_rect.x + (preview_rect.w - pw) * 0.5;
        let py = preview_rect.y + pad + (avail_h - ph) * 0.5;

        // Black preview box
        let video_rect = Rect::new(px, py, pw, ph);
        p.rect_filled(video_rect, 0.0, Color::BLACK);
        p.rect_stroke_sdf(video_rect, 0.0, 1.0 * s, chrome::BORDER);

        // If no media, show hint text
        if !playback.has_media() {
            let msg = "Open a video file";
            let msg_sz = 22.0 * s;
            let msg_w = msg_sz * 0.55 * msg.len() as f32;
            t.queue(
                msg, msg_sz,
                px + (pw - msg_w) * 0.5,
                py + (ph - msg_sz) * 0.5,
                text_dim, preview_rect.w, sw, sh,
            );
        }

        // Timecode + play state
        let ctrl_y = preview_rect.y + preview_rect.h - transport_h + 4.0 * s;
        let ctrl_h = 32.0 * s;

        // Play/pause indicator
        let state_label = match playback.state {
            PlayState::Playing => "||",
            PlayState::Paused => ">",
            PlayState::Empty => "-",
        };
        let ind_x = preview_rect.x + pad;
        let ind_r = Rect::new(ind_x, ctrl_y, 32.0 * s, ctrl_h);
        p.rect_filled(ind_r, 6.0 * s, chrome::BUTTON);
        p.rect_stroke_sdf(ind_r, 6.0 * s, 1.0 * s, chrome::BORDER);
        t.queue(state_label, 18.0 * s, ind_x + 8.0 * s, ctrl_y + 5.0 * s,
            accent, preview_rect.w, sw, sh);

        // Timecode
        let tc = playback::format_timecode(playback.position, playback.fps.max(1.0));
        let tc_sz = 18.0 * s;
        let tc_w = tc_sz * 0.60 * tc.len() as f32;
        let tc_x = preview_rect.x + (preview_rect.w - tc_w) * 0.5;
        let tc_bg = Rect::new(tc_x - 8.0 * s, ctrl_y, tc_w + 16.0 * s, ctrl_h);
        p.rect_filled(tc_bg, 6.0 * s, chrome::INPUT_BG);
        p.rect_stroke_sdf(tc_bg, 6.0 * s, 1.0 * s, chrome::BORDER);
        t.queue(&tc, tc_sz, tc_x, ctrl_y + 5.0 * s, accent, preview_rect.w, sw, sh);

        // Duration on right
        if playback.has_media() {
            let dur = playback::format_timecode(playback.duration, playback.fps);
            let dur_sz = 16.0 * s;
            let dur_w = dur_sz * 0.60 * dur.len() as f32;
            let dur_x = preview_rect.x + preview_rect.w - pad - dur_w;
            t.queue(&dur, dur_sz, dur_x, ctrl_y + 6.0 * s, text_dim, preview_rect.w, sw, sh);
        }
    }

    /// Execute the texture render pass (call after painter.render_pass, before text.render_queued).
    pub fn render_pass(
        &self,
        gpu: &GpuContext,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        preview_rect: &Rect,
        playback: &Playback,
        s: f32,
    ) {
        let tex = match &self.video_texture {
            Some(t) => t,
            None => return,
        };
        if !playback.has_media() { return; }

        let pad = PANEL_PAD * s;
        let transport_h = 44.0 * s;
        let avail_w = preview_rect.w - pad * 2.0;
        let avail_h = preview_rect.h - pad * 2.0 - transport_h;

        let vw = playback.video_width as f32;
        let vh = playback.video_height as f32;
        let aspect = vw / vh;

        let (pw, ph) = if avail_w / avail_h > aspect {
            (avail_h * aspect, avail_h)
        } else {
            (avail_w, avail_w / aspect)
        };
        let px = preview_rect.x + (preview_rect.w - pw) * 0.5;
        let py = preview_rect.y + pad + (avail_h - ph) * 0.5;

        let draw = TextureDraw::new(tex, px, py, pw, ph);
        self.tex_pass.render_pass(gpu, encoder, view, &[draw], None);
    }
}

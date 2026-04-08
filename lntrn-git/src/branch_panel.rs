//! Branch panel — dashboard showing all branches with ahead/behind counts.

use lntrn_render::{Painter, Rect, TextRenderer};
use lntrn_ui::gpu::{FoxPalette, InteractionContext, ScrollArea, Scrollbar};

use crate::git;

// Zone IDs
const ZONE_BRANCH_ROW_BASE: u32 = 5000;
const ZONE_SCROLLBAR: u32 = 5499;

pub struct BranchPanel {
    pub branches: Vec<git::BranchDetail>,
    scroll_offset: f32,
    content_height: f32,
    viewport_h: f32,
}

impl BranchPanel {
    pub fn new() -> Self {
        Self {
            branches: Vec::new(),
            scroll_offset: 0.0,
            content_height: 0.0,
            viewport_h: 0.0,
        }
    }

    pub fn on_scroll(&mut self, delta: f32) {
        ScrollArea::apply_scroll(
            &mut self.scroll_offset, delta,
            self.content_height, self.viewport_h,
        );
    }

    pub fn draw(
        &mut self, painter: &mut Painter, text: &mut TextRenderer,
        ix: &mut InteractionContext, palette: &FoxPalette,
        cx: f32, cy: f32, cw: f32, ch: f32,
        s: f32, sw: u32, sh: u32,
    ) {
        let title_font = 24.0 * s;
        let body_font = 22.0 * s;
        let small_font = 16.0 * s;
        let badge_font = 18.0 * s;
        let row_h = 80.0 * s;
        let pad = 20.0 * s;
        let gap = 12.0 * s;

        let mut y = cy + gap;

        // Header
        text.queue("Branches", title_font, cx + pad, y, palette.text, cw, sw, sh);
        y += title_font + gap;

        if self.branches.is_empty() {
            text.queue("Loading branches...", body_font, cx + pad, y, palette.muted, cw, sw, sh);
            return;
        }

        // Find base branch name for label
        let base_name = self.branches.iter()
            .find(|b| b.name == "main" || b.name == "master")
            .map(|b| b.name.as_str())
            .unwrap_or("main");

        let list_top = y;
        let list_h = ch - (y - cy);
        let total_content_h = self.branches.len() as f32 * row_h;

        self.content_height = total_content_h;
        self.viewport_h = list_h;

        let viewport = Rect::new(cx, list_top, cw, list_h);
        let scroll = ScrollArea::new(viewport, total_content_h, &mut self.scroll_offset);
        scroll.begin(painter, text);

        let base_y = scroll.content_y();
        for (i, branch) in self.branches.iter().enumerate() {
            let fy = base_y + i as f32 * row_h;
            if fy + row_h < list_top || fy > list_top + list_h { continue; }

            let row_rect = Rect::new(cx, fy, cw, row_h);
            let zone_id = ZONE_BRANCH_ROW_BASE + i as u32;
            let state = ix.add_zone(zone_id, row_rect);

            // Row background
            if branch.is_current {
                painter.rect_filled(row_rect, 8.0 * s, palette.accent.with_alpha(0.08));
            } else if state.is_hovered() {
                painter.rect_filled(row_rect, 8.0 * s, palette.muted.with_alpha(0.08));
            }

            let ty = fy + pad * 0.5;
            let mut lx = cx + pad;

            // Current branch indicator
            if branch.is_current {
                let dot_r = 5.0 * s;
                let dot_y = ty + body_font * 0.5;
                painter.circle_filled(lx + dot_r, dot_y, dot_r, palette.accent);
                lx += dot_r * 2.0 + 10.0 * s;
            } else {
                lx += 20.0 * s;
            }

            // Branch name
            let name_color = if branch.is_current { palette.accent } else { palette.text };
            text.queue(&branch.name, body_font, lx, ty, name_color, cw * 0.4, sw, sh);

            // Ahead/behind badges (relative to main)
            let badge_x = cx + cw * 0.5;
            if branch.ahead > 0 || branch.behind > 0 {
                let ahead_str = format!("+{}", branch.ahead);
                let behind_str = format!("-{}", branch.behind);

                // Ahead badge
                let aw = ahead_str.len() as f32 * badge_font * 0.55 + 12.0 * s;
                let badge_h = 26.0 * s;
                let badge_y = ty + (body_font - badge_h) * 0.5;

                let ahead_rect = Rect::new(badge_x, badge_y, aw, badge_h);
                painter.rect_filled(ahead_rect, 4.0 * s, palette.accent.with_alpha(0.15));
                text.queue(&ahead_str, badge_font,
                    badge_x + 6.0 * s, badge_y + (badge_h - badge_font) * 0.5,
                    palette.accent, aw, sw, sh);

                // Behind badge
                let bw = behind_str.len() as f32 * badge_font * 0.55 + 12.0 * s;
                let behind_rect = Rect::new(badge_x + aw + 6.0 * s, badge_y, bw, badge_h);
                painter.rect_filled(behind_rect, 4.0 * s, palette.warning.with_alpha(0.15));
                text.queue(&behind_str, badge_font,
                    behind_rect.x + 6.0 * s, badge_y + (badge_h - badge_font) * 0.5,
                    palette.warning, bw, sw, sh);

                // "vs main" label
                let vs_x = behind_rect.x + bw + 8.0 * s;
                text.queue(&format!("vs {base_name}"), small_font, vs_x,
                    ty + 2.0 * s, palette.muted, 100.0 * s, sw, sh);
            } else if branch.name == base_name {
                text.queue("base", badge_font, badge_x, ty + 2.0 * s,
                    palette.muted, 60.0 * s, sw, sh);
            }

            // Upstream indicator
            if !branch.has_upstream {
                let no_remote = "local only";
                let nr_x = cx + cw - pad - 100.0 * s;
                text.queue(no_remote, small_font, nr_x, ty + 2.0 * s,
                    palette.warning.with_alpha(0.7), 100.0 * s, sw, sh);
            }

            // Last commit subject (second line)
            if !branch.last_commit.is_empty() {
                let commit_y = ty + body_font + 4.0 * s;
                let commit_x = cx + pad + 20.0 * s;
                text.queue(&branch.last_commit, small_font, commit_x, commit_y,
                    palette.muted, cw - pad * 2.0 - 20.0 * s, sw, sh);
            }

            // Divider
            if i < self.branches.len() - 1 {
                let div_y = fy + row_h - 1.0 * s;
                painter.rect_filled(
                    Rect::new(cx + pad, div_y, cw - pad * 2.0, 1.0 * s),
                    0.0, palette.muted.with_alpha(0.12),
                );
            }
        }

        scroll.end(painter, text);

        if total_content_h > list_h {
            let scrollbar = Scrollbar::new(&viewport, total_content_h, self.scroll_offset);
            let sb_state = ix.add_zone(ZONE_SCROLLBAR, scrollbar.thumb);
            scrollbar.draw(painter, sb_state, palette);
        }
    }
}

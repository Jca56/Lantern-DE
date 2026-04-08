//! Commit graph — custom DAG layout with circles and lines.

use lntrn_render::{Color, Painter, Rect, TextRenderer};
use lntrn_ui::gpu::{FoxPalette, InteractionContext, ScrollArea, Scrollbar};

use crate::git;

const ZONE_SCROLLBAR: u32 = 5999;
const ROW_H: f32 = 40.0;
const LANE_W: f32 = 24.0;
const NODE_R: f32 = 5.0;

/// 6 rotating lane colors.
const LANE_COLORS: [(f32, f32, f32); 6] = [
    (0.40, 0.75, 1.00), // blue
    (0.60, 0.90, 0.45), // green
    (1.00, 0.65, 0.30), // orange
    (0.85, 0.50, 0.95), // purple
    (1.00, 0.45, 0.50), // red
    (0.45, 0.90, 0.85), // teal
];

fn lane_color(lane: usize) -> Color {
    let (r, g, b) = LANE_COLORS[lane % LANE_COLORS.len()];
    Color::rgba(r, g, b, 1.0)
}

/// A positioned node in the graph.
struct LayoutNode {
    lane: usize,
    /// Lines connecting this commit to its parents: (from_lane, to_lane, target_row).
    edges: Vec<(usize, usize, usize)>,
}

pub struct GraphView {
    commits: Vec<git::GraphCommit>,
    layout: Vec<LayoutNode>,
    scroll_offset: f32,
    content_height: f32,
    viewport_h: f32,
}

impl GraphView {
    pub fn new() -> Self {
        Self {
            commits: Vec::new(),
            layout: Vec::new(),
            scroll_offset: 0.0,
            content_height: 0.0,
            viewport_h: 0.0,
        }
    }

    pub fn set_commits(&mut self, commits: Vec<git::GraphCommit>) {
        self.layout = Self::compute_layout(&commits);
        self.commits = commits;
        self.scroll_offset = 0.0;
    }

    pub fn on_scroll(&mut self, delta: f32) {
        ScrollArea::apply_scroll(
            &mut self.scroll_offset, delta,
            self.content_height, self.viewport_h,
        );
    }

    /// Assign lanes to commits and compute edges.
    fn compute_layout(commits: &[git::GraphCommit]) -> Vec<LayoutNode> {
        // Active lanes: each slot holds the hash of the commit expected in that lane.
        let mut lanes: Vec<Option<String>> = Vec::new();
        let mut nodes = Vec::with_capacity(commits.len());

        // Build a hash->row index map for edge targeting.
        let mut hash_to_row: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
        for (i, c) in commits.iter().enumerate() {
            hash_to_row.insert(&c.hash, i);
        }

        for (_row, commit) in commits.iter().enumerate() {
            // Find which lane this commit occupies (it should have been reserved by a parent).
            let my_lane = lanes.iter().position(|slot| {
                slot.as_ref().map_or(false, |h| h == &commit.hash)
            });

            let my_lane = if let Some(l) = my_lane {
                // Clear the reservation.
                lanes[l] = None;
                l
            } else {
                // No reservation — this is a branch head. Find a free lane or create one.
                let free = lanes.iter().position(|s| s.is_none());
                if let Some(f) = free { f } else { lanes.push(None); lanes.len() - 1 }
            };

            // Reserve lanes for parents.
            let mut edges = Vec::new();
            for (pi, parent_hash) in commit.parents.iter().enumerate() {
                if let Some(&target_row) = hash_to_row.get(parent_hash.as_str()) {
                    if pi == 0 {
                        // First parent: stays in the same lane.
                        lanes[my_lane] = Some(parent_hash.clone());
                        edges.push((my_lane, my_lane, target_row));
                    } else {
                        // Additional parents (merge): find a free lane.
                        let existing = lanes.iter().position(|slot| {
                            slot.as_ref().map_or(false, |h| h == parent_hash)
                        });
                        let parent_lane = if let Some(l) = existing {
                            l
                        } else {
                            let free = lanes.iter().position(|s| s.is_none());
                            let l = if let Some(f) = free { f } else { lanes.push(None); lanes.len() - 1 };
                            lanes[l] = Some(parent_hash.clone());
                            l
                        };
                        edges.push((my_lane, parent_lane, target_row));
                    }
                }
            }

            nodes.push(LayoutNode { lane: my_lane, edges });
        }

        nodes
    }

    pub fn draw(
        &mut self, painter: &mut Painter, text: &mut TextRenderer,
        ix: &mut InteractionContext, palette: &FoxPalette,
        cx: f32, cy: f32, cw: f32, ch: f32,
        s: f32, sw: u32, sh: u32,
    ) {
        let row_h = ROW_H * s;
        let lane_w = LANE_W * s;
        let node_r = NODE_R * s;
        let body_font = 18.0 * s;
        let small_font = 14.0 * s;
        let badge_font = 14.0 * s;
        let pad = 16.0 * s;
        let line_w = 2.0 * s;

        if self.commits.is_empty() {
            text.queue("Loading commit graph...", 22.0 * s, cx + 20.0 * s, cy + 40.0 * s,
                palette.muted, cw, sw, sh);
            return;
        }

        let max_lane = self.layout.iter().map(|n| n.lane).max().unwrap_or(0) + 1;
        let graph_w = max_lane as f32 * lane_w + pad;
        let text_x = cx + graph_w + pad;
        let text_w = cw - graph_w - pad * 2.0;

        let total_h = self.commits.len() as f32 * row_h;
        self.content_height = total_h;
        self.viewport_h = ch;

        let viewport = Rect::new(cx, cy, cw, ch);
        let scroll = ScrollArea::new(viewport, total_h, &mut self.scroll_offset);
        scroll.begin(painter, text);

        let base_y = scroll.content_y();

        // Draw edges first (behind nodes).
        for (row, node) in self.layout.iter().enumerate() {
            let y1 = base_y + row as f32 * row_h + row_h * 0.5;
            for &(from_lane, to_lane, target_row) in &node.edges {
                let y2 = base_y + target_row as f32 * row_h + row_h * 0.5;
                // Skip if both endpoints are off-screen.
                if (y1 < cy && y2 < cy) || (y1 > cy + ch && y2 > cy + ch) { continue; }

                let x1 = cx + pad + from_lane as f32 * lane_w + lane_w * 0.5;
                let x2 = cx + pad + to_lane as f32 * lane_w + lane_w * 0.5;
                let color = lane_color(from_lane);

                if from_lane == to_lane {
                    // Straight vertical line.
                    painter.line(x1, y1, x2, y2, line_w, color);
                } else {
                    // Diagonal: go down one row vertically, then diagonal to target lane.
                    let mid_y = y1 + row_h;
                    painter.line(x1, y1, x1, mid_y, line_w, color);
                    painter.line(x1, mid_y, x2, y2, line_w, color);
                }
            }
        }

        // Draw nodes and text.
        for (row, (commit, node)) in self.commits.iter().zip(self.layout.iter()).enumerate() {
            let y = base_y + row as f32 * row_h;
            if y + row_h < cy || y > cy + ch { continue; }

            let node_x = cx + pad + node.lane as f32 * lane_w + lane_w * 0.5;
            let node_y = y + row_h * 0.5;
            let color = lane_color(node.lane);

            // Commit circle
            painter.circle_filled(node_x, node_y, node_r, color);
            painter.circle_stroke(node_x, node_y, node_r + 1.0 * s, 1.0 * s, palette.surface);

            // Short hash
            let ty = y + (row_h - body_font) * 0.5;
            text.queue(&commit.short_hash, small_font, text_x, ty + 2.0 * s,
                palette.muted, 80.0 * s, sw, sh);

            // Subject
            let subject_x = text_x + 80.0 * s;
            text.queue(&commit.subject, body_font, subject_x, ty,
                palette.text, text_w - 80.0 * s, sw, sh);

            // Decorations (branch labels)
            if !commit.decorations.is_empty() {
                let mut dx = subject_x + commit.subject.len().min(50) as f32 * body_font * 0.5 + 8.0 * s;
                for dec in &commit.decorations {
                    let label = dec.replace("HEAD -> ", "").replace("origin/", "");
                    let bw = label.len() as f32 * badge_font * 0.55 + 10.0 * s;
                    let bh = 20.0 * s;
                    let by = y + (row_h - bh) * 0.5;
                    let is_head = dec.contains("HEAD");
                    let badge_color = if is_head { palette.accent } else { palette.muted };
                    painter.rect_filled(Rect::new(dx, by, bw, bh), 3.0 * s, badge_color.with_alpha(0.2));
                    text.queue(&label, badge_font, dx + 5.0 * s, by + (bh - badge_font) * 0.5,
                        badge_color, bw, sw, sh);
                    dx += bw + 4.0 * s;
                }
            }
        }

        scroll.end(painter, text);

        if total_h > ch {
            let scrollbar = Scrollbar::new(&viewport, total_h, self.scroll_offset);
            let sb_state = ix.add_zone(ZONE_SCROLLBAR, scrollbar.thumb);
            scrollbar.draw(painter, sb_state, palette);
        }
    }
}

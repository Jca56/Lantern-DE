//! The History tab: the newest commits of every branch as a graph, each
//! on its lane with lines down to its parents, beside its hash, what it
//! says, and the branches that point at it.

use lntrn_kit::{Keep, bits, elide, look, mono_style, small_style};
use lntrn_math::{Rect, Vec2};
use lntrn_ui::{FILL, Ui};

use crate::app::App;
use crate::glyphs::lane_color;
use crate::history::Graph;

const ROW_H: f64 = 48.0;
const LANE_W: f64 = 26.0;
/// Lanes past this many are drawn over the last: a graph that wide has
/// stopped being readable, and the text needs its room.
const MAX_LANES: usize = 10;
const RADIUS: f64 = 15.0;

pub fn draw(app: &mut App, ui: &mut Ui) {
    app.want_history();
    let Some(repo) = &app.repo else { return };
    let m = ui.m;
    let height = (ui.remaining_height() - m.px(4.0)).max(0.0);
    let panel = Rect::from_min_size(ui.cursor(), Vec2::new(ui.avail_width(), height));
    let radius = m.px(RADIUS);
    ui.draw.rounded_rect(panel, radius, look::WELL);
    ui.draw.stroke_rect(panel, m.px(1.0), radius, look::LINE);
    let words = match &repo.graph {
        None => Some("Reading the history…"),
        Some(g) if g.commits.is_empty() => Some("No commits yet"),
        Some(_) => None,
    };
    if let Some(words) = words {
        let rect = ui.alloc(Vec2::new(FILL, height));
        let style = small_style(ui);
        ui.text_centered(words, &style, rect, look::TEXT_DIM);
        return;
    }
    let Some(graph) = &repo.graph else { return };
    // Clear of the panel's rounded corners, top and bottom.
    let pad = m.px(8.0);
    ui.space(pad);
    ui.scroll_area("history", Some((height - pad * 2.0 - m.gap).max(0.0)), |ui| rows(ui, graph));
}

/// Where lane `lane` runs, as an x.
fn lane_x(ui: &Ui, left: f64, lane: usize) -> f64 {
    left + ui.m.px(22.0) + lane.min(MAX_LANES - 1) as f64 * ui.m.px(LANE_W)
}

fn rows(ui: &mut Ui, graph: &Graph) {
    let m = ui.m;
    let row_h = m.px(ROW_H);
    let area = ui.alloc(Vec2::new(FILL, graph.commits.len() as f64 * row_h));
    let clip = ui.clip();
    let (top, bottom) = (clip.min.y, clip.max.y);
    let y_of = |row: usize| area.min.y + row as f64 * row_h + row_h * 0.5;
    let line_w = m.px(2.5);

    // Lines first, under the dots: every one that crosses what is in view.
    for (row, node) in graph.nodes.iter().enumerate() {
        let y1 = y_of(row);
        for edge in &node.edges {
            let y2 = y_of(edge.row);
            if y2 < top || y1 > bottom {
                continue;
            }
            let (x1, x2) = (lane_x(ui, area.min.x, edge.from), lane_x(ui, area.min.x, edge.to));
            let color = lane_color(if edge.from == edge.to { edge.from } else { edge.from.max(edge.to) });
            if x1 == x2 {
                ui.draw.line(Vec2::new(x1, y1), Vec2::new(x2, y2), line_w, color);
            } else {
                // Across to the other lane within one row, then straight
                // down it.
                let bend = (y1 + row_h).min(y2);
                ui.draw.line(Vec2::new(x1, y1), Vec2::new(x2, bend), line_w, color);
                ui.draw.line(Vec2::new(x2, bend), Vec2::new(x2, y2), line_w, color);
            }
        }
    }

    let (style, mono) = (ui.text_style(), mono_style(ui));
    let hash_w = ui.measure("0000000", &mono) + m.px(16.0);
    let text_x = lane_x(ui, area.min.x, graph.lanes.max(1) - 1) + m.px(24.0);
    let right = area.max.x - m.px(14.0);
    for (row, (commit, node)) in graph.commits.iter().zip(&graph.nodes).enumerate() {
        let y = y_of(row);
        if y + row_h < top || y - row_h > bottom {
            continue;
        }
        let dot = Vec2::new(lane_x(ui, area.min.x, node.lane), y);
        ui.draw.circle(dot, m.px(8.0), look::WELL);
        ui.draw.circle(dot, m.px(6.0), lane_color(node.lane));

        let line = Rect::new(Vec2::new(text_x, y - row_h * 0.5), Vec2::new(right, y + row_h * 0.5));
        ui.text_in_rect(&commit.short_hash, &mono, line, look::TEXT_DIM);
        // The refs on it come after what it says, which gives way to them.
        let labels: Vec<(String, bool)> = commit.decorations.iter().map(|d| (d.trim_start_matches("HEAD -> ").to_owned(), d.starts_with("HEAD"))).collect();
        let mut labels_w = 0.0;
        for (text, _) in &labels {
            labels_w += bits::badge_width(ui, text) + m.px(6.0);
        }
        let subject_x = text_x + hash_w;
        let room = (right - subject_x - labels_w - m.px(10.0)).max(m.px(80.0));
        let subject = elide(ui, &commit.subject, &style, room, Keep::Start);
        ui.text_in_rect(&subject, &style, Rect::new(Vec2::new(subject_x, line.min.y), Vec2::new(subject_x + room, line.max.y)), look::TEXT);
        let mut x = subject_x + ui.measure(&subject, &style) + m.px(12.0);
        let accent = ui.theme.accent;
        for (text, head) in &labels {
            if x > right {
                break;
            }
            x += bits::badge(ui, x, y, text, if *head { accent } else { look::INFO }) + m.px(6.0);
        }
    }
}

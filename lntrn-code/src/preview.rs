//! The Preview editor: a Markdown document rendered as rich text. It
//! follows the editor: when the caret moves to another line, the
//! preview scrolls to the heading (or paragraph) that line is under.

use lntrn_ui::Ui;

use crate::doc::{Doc, DocId};
use crate::syntax::Language;

/// The line the preview anchors on for a caret at `line`: the nearest
/// heading at or above it, else the start of its paragraph.
fn anchor_line(lines: &[String], line: usize) -> usize {
    let line = line.min(lines.len().saturating_sub(1));
    let is_heading = |l: &str| l.trim_start().starts_with('#');
    if let Some(h) = (0..=line).rev().find(|&l| is_heading(&lines[l])) {
        return h;
    }
    let mut start = line;
    while start > 0 && !lines[start - 1].trim().is_empty() {
        start -= 1;
    }
    start
}

pub fn draw_preview(ui: &mut Ui, doc: Option<&Doc>, follow: &mut Option<(DocId, usize)>) {
    match doc {
        Some(d) if d.lang() == Language::Markdown => {
            let text = d.buffer.to_text();
            let scroll_id = ui.id("preview");
            let moved = *follow != Some((d.id, d.cursor.line));
            ui.scroll_area("preview", None, |ui| {
                if moved {
                    let lines = d.buffer.lines();
                    let anchor = anchor_line(lines, d.cursor.line);
                    let before = lines[..anchor].join("\n");
                    let y = if anchor == 0 { 0.0 } else { ui.rich_text_height(&before, ui.avail_width()) };
                    ui.state.scroll(scroll_id).offset.y = y;
                    ui.state.request_rebuild = true;
                }
                ui.rich_text(&text);
            });
            *follow = Some((d.id, d.cursor.line));
        }
        Some(d) => {
            ui.heading(&d.title);
            ui.label_dim("Not a Markdown file. Focus a .md file to preview it here.");
        }
        None => {
            ui.heading("Preview");
            ui.label_dim("Focus a Markdown file to preview it here.");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anchors_on_headings_then_paragraphs() {
        let lines: Vec<String> = ["# A", "text", "", "para", "more", "## B", "under"].iter().map(|s| s.to_string()).collect();
        assert_eq!(anchor_line(&lines, 1), 0, "the heading above");
        assert_eq!(anchor_line(&lines, 4), 0, "still under A");
        assert_eq!(anchor_line(&lines, 6), 5);
        let plain: Vec<String> = ["a", "b", "", "c", "d"].iter().map(|s| s.to_string()).collect();
        assert_eq!(anchor_line(&plain, 4), 3, "no heading: the paragraph start");
        assert_eq!(anchor_line(&plain, 99), 3, "past the end clamps");
    }
}

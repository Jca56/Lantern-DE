//! The Preview editor: a Markdown document rendered as rich text.

use lntrn_ui::Ui;

use crate::doc::Doc;
use crate::syntax::Language;

pub fn draw_preview(ui: &mut Ui, doc: Option<&Doc>) {
    match doc {
        Some(d) if d.lang() == Language::Markdown => {
            let text = d.buffer.to_text();
            ui.scroll_area("preview", None, |ui| {
                ui.rich_text(&text);
            });
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

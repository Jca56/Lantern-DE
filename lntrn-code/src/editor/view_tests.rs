//! The document view headless: the minimap never hides text.

use lntrn_ui::WidgetId;
use lntrn_ui::testing::Harness;

use crate::buffer::Pos;
use crate::doc::{Doc, DocId};
use crate::editor::lsp_ui::LspUi;
use crate::editor::view::{ViewOpts, draw_doc};
use crate::settings::Settings;

/// With the caret at the end of the longest line, the view scrolls far
/// enough that the caret sits left of the minimap: for lines far wider
/// than the view and for ones that only just reach under the map.
#[test]
fn line_ends_clear_the_minimap() {
    let settings = Settings::default();
    assert!(settings.minimap, "the map is on by default");
    let map_id = WidgetId::ROOT.with("code").with("minimap");
    for len in (40..=240).step_by(6) {
        let line = "x".repeat(len);
        let mut doc = Doc::from_text(DocId(1), Some(std::path::PathBuf::from("demo.rs")), &format!("fn main() {{}}\n{line}\n"), 4);
        doc.set_cursor(Pos::new(1, len), false);
        let mut lsp = LspUi::default();
        let mut h = Harness::new(1200.0, 800.0);
        for _ in 0..3 {
            h.frame(|ui| {
                ui.state.focus = Some(ui.id("code"));
                draw_doc(ui, &mut doc, &settings, ViewOpts { area_active: true, matches: &[], current_match: None, diags: &[], git: &[], lsp: &mut lsp });
            });
        }
        let caret = h.state.ime_rect.expect("the focused editor drew its caret");
        let strip = h.rect_of(map_id).expect("the minimap was drawn");
        assert!(caret.max.x <= strip.min.x, "{len} cells: caret right edge {} is under the minimap at {}", caret.max.x, strip.min.x);
    }
}

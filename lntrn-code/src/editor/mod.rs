//! The code editor: a document drawn as a monospace grid inside a
//! two-way scroll area, with a gutter, selection, caret and syntax
//! colors ([`view`]); the keys it answers to ([`input`]); the editing
//! operations behind them and the menus ([`ops`]); the file tabs above
//! it ([`tabs`]); and the find/replace bar ([`find`]).

pub mod decor;
pub mod find;
pub mod fold;
pub mod input;
pub mod lsp_ui;
pub mod minimap;
pub mod ops;
pub mod prose;
pub mod tabs;
pub mod view;
pub mod wrap;

use lntrn_text::TextStyle;
use lntrn_ui::{AreaId, Ui, WidgetId};

use crate::settings::Settings;

/// The widget id of the editor in an area's body, so the app can hand it
/// keyboard focus from outside a rebuild.
pub fn editor_id(area: AreaId) -> WidgetId {
    WidgetId::ROOT.with_u64(area as u64).with("body").with("code")
}

/// The code font at the current scale.
pub fn code_style(ui: &Ui, settings: &Settings) -> TextStyle {
    TextStyle::new((settings.font_size * ui.m.scale).round().max(6.0) as f32).mono()
}

/// The monospace grid: one cell's advance and the line height, in
/// physical pixels.
pub fn cell_metrics(ui: &mut Ui, style: &TextStyle) -> (f64, f64) {
    let cell_w = ui.measure("M", style).max(1.0);
    let lh = (style.line_height() as f64).ceil();
    (cell_w, lh)
}

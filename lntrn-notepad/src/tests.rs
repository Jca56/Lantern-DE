//! Regression tests for the editor's bulk text operations — the 2026-08-15
//! paste overhaul replaced the per-char insert loop, and these pin down that
//! the bulk path produces the same document the char loop did.

use crate::editor::{Editor, Pos};

fn editor_with(text: &str) -> Editor {
    let mut e = Editor::new();
    e.insert_str(text);
    e
}

#[test]
fn paste_single_line_mid_line() {
    let mut e = editor_with("hello world");
    e.cursor_line = 0;
    e.cursor_col = 5;
    e.insert_str("XYZ");
    assert_eq!(e.lines, vec!["helloXYZ world"]);
    assert_eq!((e.cursor_line, e.cursor_col), (0, 8));
    assert_eq!(e.formats.len(), e.lines.len());
}

#[test]
fn paste_multi_line_splits_and_keeps_tail() {
    let mut e = editor_with("hello world");
    e.cursor_line = 0;
    e.cursor_col = 5;
    e.insert_str("AA\nBB\nCC");
    assert_eq!(e.lines, vec!["helloAA", "BB", "CC world"]);
    // Cursor lands right after the pasted text, before the old tail.
    assert_eq!((e.cursor_line, e.cursor_col), (2, 2));
    assert_eq!(e.formats.len(), e.lines.len());
}

#[test]
fn paste_preserves_format_of_moved_tail() {
    let mut e = editor_with("abcdef");
    // Bold the tail "def" (bytes 3..6), then split before it with a paste.
    e.sel_anchor = Some(Pos::new(0, 3));
    e.cursor_line = 0;
    e.cursor_col = 6;
    e.toggle_format(|a| a.bold = true);
    e.sel_anchor = None;
    e.cursor_col = 3;
    e.insert_str("X\nY");
    assert_eq!(e.lines, vec!["abcX", "Ydef"]);
    // "def" moved to line 1 offset 1..4 and must still be bold.
    assert!(e.formats.get(1).attrs_at(1).bold);
    assert!(e.formats.get(1).attrs_at(3).bold);
    assert!(!e.formats.get(1).attrs_at(0).bold);
    assert_eq!(e.formats.len(), e.lines.len());
}

#[test]
fn paste_expands_span_at_cursor_like_typing_did() {
    let mut e = editor_with("abcdef");
    // Bold bytes 1..5 ("bcde"), paste inside the bold run.
    e.sel_anchor = Some(Pos::new(0, 1));
    e.cursor_line = 0;
    e.cursor_col = 5;
    e.toggle_format(|a| a.bold = true);
    e.sel_anchor = None;
    e.cursor_col = 3;
    e.insert_str("ZZ");
    assert_eq!(e.lines, vec!["abcZZdef"]);
    // The old per-char path let the crossing span absorb the insertion.
    assert!(e.formats.get(0).attrs_at(3).bold);
    assert!(e.formats.get(0).attrs_at(4).bold);
}

#[test]
fn paste_trailing_newline_makes_empty_line() {
    let mut e = editor_with("ab");
    e.cursor_col = 2;
    e.insert_str("X\n");
    assert_eq!(e.lines, vec!["abX", ""]);
    assert_eq!((e.cursor_line, e.cursor_col), (1, 0));
    assert_eq!(e.formats.len(), e.lines.len());
}

#[test]
fn large_paste_is_fast_enough() {
    // The old O(n²) loop took minutes on this; the bulk path must be instant.
    let blob = "x".repeat(2_000_000);
    let mut e = Editor::new();
    let t = std::time::Instant::now();
    e.insert_str(&blob);
    assert!(
        t.elapsed() < std::time::Duration::from_secs(2),
        "paste too slow: {:?}",
        t.elapsed()
    );
    assert_eq!(e.lines[0].len(), 2_000_000);
}

#[test]
fn typing_after_click_keeps_font_size() {
    // Set 32px with no selection (pending), type, then "click" (pending
    // cleared, cursor unchanged) — further typing must stay 32px.
    let mut e = Editor::new();
    e.set_font_size(32.0);
    e.insert_char('a');
    e.pending_attrs = None; // what a cursor-moving click does
    e.insert_char('b');
    assert_eq!(e.formats.get(0).attrs_at(0).font_size, Some(32.0));
    assert_eq!(e.formats.get(0).attrs_at(1).font_size, Some(32.0));
}

#[test]
fn enter_carries_format_to_new_line() {
    let mut e = Editor::new();
    e.set_font_size(32.0);
    e.insert_char('a');
    e.pending_attrs = None;
    e.insert_char('\n');
    e.insert_char('b');
    assert_eq!(e.formats.get(1).attrs_at(0).font_size, Some(32.0));
}

#[test]
fn paste_inherits_insertion_format() {
    let mut e = Editor::new();
    e.set_font_size(32.0);
    e.insert_char('a');
    e.pending_attrs = None;
    e.insert_str("XY\nZ");
    assert_eq!(e.lines, vec!["aXY", "Z"]);
    assert_eq!(e.formats.get(0).attrs_at(2).font_size, Some(32.0));
    assert_eq!(e.formats.get(1).attrs_at(0).font_size, Some(32.0));
}

#[test]
fn insert_formatted_mid_span_leaves_no_overlap() {
    let mut e = editor_with("abcd");
    e.sel_anchor = Some(Pos::new(0, 0));
    e.cursor_line = 0;
    e.cursor_col = 4;
    e.toggle_format(|a| a.bold = true);
    e.sel_anchor = None;
    // Type an italic-only char strictly inside the bold span.
    e.cursor_col = 2;
    e.pending_attrs = Some({
        let mut a = crate::format::TextAttrs::default();
        a.italic = true;
        a
    });
    e.insert_char('z');
    let spans = e.formats.get(0).spans();
    for w in spans.windows(2) {
        assert!(w[0].end <= w[1].start, "overlapping spans: {spans:?}");
    }
    assert!(e.formats.get(0).attrs_at(2).italic);
    assert!(!e.formats.get(0).attrs_at(2).bold);
    assert!(e.formats.get(0).attrs_at(3).bold);
}

#[test]
fn pending_default_breaks_out_of_span() {
    // Un-bolding mid-bold-run must produce an unformatted char (the old
    // insert_formatted expanded the surrounding span and kept it bold).
    let mut e = editor_with("abcd");
    e.sel_anchor = Some(Pos::new(0, 0));
    e.cursor_line = 0;
    e.cursor_col = 4;
    e.toggle_format(|a| a.bold = true);
    e.sel_anchor = None;
    e.cursor_col = 2;
    e.toggle_format(|a| a.bold = !a.bold); // pending: bold off
    e.insert_char('z');
    assert!(!e.formats.get(0).attrs_at(2).bold);
    assert!(e.formats.get(0).attrs_at(1).bold);
    assert!(e.formats.get(0).attrs_at(3).bold);
}

#[test]
fn delete_selection_with_stale_anchor_does_not_panic() {
    let mut e = editor_with("short");
    // Anchor far beyond the document — as a stale find/replace leftover.
    e.sel_anchor = Some(Pos::new(7, 42));
    e.cursor_line = 0;
    e.cursor_col = 2;
    e.delete_selection();
    assert_eq!(e.lines, vec!["sh"]);
}

#[test]
fn delete_selection_snaps_mid_char_cols() {
    let mut e = editor_with("a→b");
    // '→' spans bytes 1..4; col 2 is mid-char and must snap, not panic.
    e.sel_anchor = Some(Pos::new(0, 2));
    e.cursor_line = 0;
    e.cursor_col = 5;
    e.delete_selection();
    assert!(e.lines[0].is_char_boundary(e.cursor_col));
}

// ── LineFormats (moved from format.rs for the file-size rule) ──────────────

use crate::format::LineFormats;

/// Applying a font family to a plain range must leave a span carrying that
/// font index — the core of the font-picker feature.
#[test]
fn font_applies_to_range() {
    let mut lf = LineFormats::new();
    // "hello world", apply font index 3 to "world" (cols 6..11).
    lf.apply_format(6, 11, |a| a.font = Some(3));
    assert_eq!(lf.attrs_at(0).font, None, "plain text keeps default font");
    assert_eq!(
        lf.attrs_at(7).font,
        Some(3),
        "selected range carries the font"
    );
    // Query uniform over the range should report the font.
    assert_eq!(lf.query_uniform(6, 11).font, Some(3));
}

/// Font + bold can coexist on the same span.
#[test]
fn font_and_bold_coexist() {
    let mut lf = LineFormats::new();
    lf.apply_format(0, 5, |a| a.font = Some(2));
    lf.apply_format(0, 5, |a| a.bold = true);
    let at = lf.attrs_at(2);
    assert_eq!(at.font, Some(2));
    assert!(at.bold);
}

/// Bullet toggling lives on the paragraph, independent of spans.
#[test]
fn bullet_is_paragraph_level() {
    let mut lf = LineFormats::new();
    assert!(!lf.para.bullet);
    lf.para.bullet = true;
    assert!(lf.para.bullet);
    assert_eq!(lf.para.line_spacing, 1.0, "default spacing is single");
}

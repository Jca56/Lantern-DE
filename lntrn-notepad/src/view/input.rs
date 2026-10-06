//! What the pointer and the keys do to the page: placing the caret and
//! selecting, the handles (the sheet's edges, a picture's corner, the
//! scroll bar), a box ticked, and typing, moving, cutting and pasting.

use lntrn_math::{Rect, Vec2};
use lntrn_ui::{Key, KeyPress, Response, Ui, UiState};

use super::page::{BAR, Drag, PageIn, PageOut, Place, View, thumb_rect, wide_for};
use super::paint::check_box;
use crate::doc::{List, Pos};
use crate::editor::Editor;

/// Presses this close together, in time and place, count as one more
/// click of the same.
const CLICK_GAP: f64 = 0.45;
const CLICK_REACH: f64 = 6.0;

/// The pointer's part of a frame.
#[allow(clippy::too_many_arguments)]
pub fn pointer(ui: &mut Ui, ed: &mut Editor, view: &mut View, place: &mut Place, p: &mut PageIn, r: &Response, sides: &[Response; 2], bar: &Response, corner: Option<((usize, Rect), Response)>, out: &mut PageOut) {
    let (m, pointer, desk) = (ui.m, ui.state.pointer, place.desk);
    let over = ui.state.pointer_in_window && desk.contains(pointer) && !ui.state.shielded(ui.layer(), pointer);
    if over && ui.state.wheel.y != 0.0 {
        view.scroll -= ui.state.wheel.y;
        ui.state.wheel = Vec2::ZERO;
    }
    let most = place.max_scroll(ed.layout.height());
    // ---- the sheet's edges: each as far from the middle as the other ----
    if sides.iter().any(|s| s.pressed) {
        view.drag = Drag::Margin;
    }
    if view.drag == Drag::Margin && sides.iter().any(|s| s.held) {
        *p.page_width = wide_for(desk.width() - BAR * m.scale, (pointer.x - place.sheet.center().x).abs() * 2.0, m.scale);
        ui.state.request_rebuild = true;
    }
    if view.drag == Drag::Margin && sides.iter().any(|s| s.released) {
        (view.drag, out.resized) = (Drag::None, true);
    }
    // ---- the scroll bar: its thumb dragged, or its track pressed ----
    let track = bar.rect;
    if bar.pressed && most > 0.0 {
        let thumb = thumb_rect(track, view.scroll, most);
        view.drag = Drag::Scroll(if thumb.contains(pointer) { pointer.y - thumb.min.y } else { thumb.height() * 0.5 });
    }
    if let (Drag::Scroll(held), true) = (view.drag, bar.held) {
        let thumb = thumb_rect(track, view.scroll, most);
        let run = (track.height() - thumb.height()).max(1.0);
        view.scroll = ((pointer.y - held - track.min.y) / run).clamp(0.0, 1.0) * most;
        ui.state.request_rebuild = true;
    }
    // ---- a picture's corner: wider or narrower, keeping its shape ----
    if let Some(((para, _), cr)) = &corner {
        if cr.hovered || cr.held {
            ui.state.cursor_icon = lntrn_ui::CursorIcon::NwseResize;
        }
        if cr.pressed {
            let wide = ed.layout.sets[*para].picture.map_or(64.0, |(w, _)| w) / m.scale as f32;
            view.drag = Drag::Picture(*para, wide, pointer.x);
            ed.resize_picture(*para, wide, true);
        }
        if let (Drag::Picture(para, wide, from), true) = (view.drag, cr.held) {
            ed.resize_picture(para, wide + ((pointer.x - from) / m.scale) as f32, false);
        }
    }
    if bar.released || corner.as_ref().is_some_and(|(_, cr)| cr.released) {
        view.drag = Drag::None;
    }

    // ---- the page itself ----
    let (x, y) = place.to_doc(pointer);
    if r.pressed {
        let again = ui.state.now - view.clicks.0 < CLICK_GAP && (pointer - view.clicks.1).length() < CLICK_REACH * m.scale;
        view.clicks = (ui.state.now, pointer, if again { view.clicks.2 % 3 + 1 } else { 1 });
        let para = ed.layout.para_at(y);
        let (set, top) = (&ed.layout.sets[para], ed.layout.tops[para]);
        // A box is ticked without the caret going anywhere.
        if let (List::Check(_), Some(setting)) = (set.src.attrs.list, view.setting) {
            let (bx, by, side) = check_box(set, &setting);
            let reach = side * 0.3;
            if (bx - reach..bx + side + reach).contains(&x) && (top + by - reach..top + by + side + reach).contains(&y) {
                view.drag = Drag::None;
                return ed.tick(para);
            }
        }
        // A picture is picked whole.
        let row = &set.rows[0];
        if set.picture.is_some_and(|(w, h)| (row.x..row.x + w).contains(&x) && (top + row.top..top + row.top + h).contains(&y)) {
            view.drag = Drag::None;
            return ed.select(Pos::new(para, 0), Pos::new(para, set.src.text.len()));
        }
        let at = ed.layout.hit(x, y);
        match view.clicks.2 {
            1 => ed.move_to(at, ui.state.mods.shift()),
            2 => {
                let (a, b) = ed.doc.word_at(at);
                ed.select(a, b);
            }
            _ => {
                let (a, b) = ed.doc.para_at(at);
                ed.select(a, b);
            }
        }
        view.drag = Drag::Select;
    }
    if view.drag == Drag::Select && r.dragging && view.clicks.2 == 1 {
        // Past the top or bottom the page scrolls under the pointer.
        let past = (pointer.y - desk.max.y).max(0.0) + (pointer.y - desk.min.y).min(0.0);
        if past != 0.0 {
            view.scroll = (view.scroll + past * 0.25).clamp(0.0, most);
            *place = Place::of(desk, *p.page_width, view.scroll, m.scale);
            ui.state.request_redraw_after(0.016);
        }
        let (x, y) = place.to_doc(Vec2::new(pointer.x, pointer.y.clamp(desk.min.y, desk.max.y)));
        let at = ed.layout.hit(x, y);
        if at != ed.caret {
            ed.move_to(at, true);
        }
        ed.follow = false;
    }
    if r.released && view.drag == Drag::Select {
        view.drag = Drag::None;
    }
    // A right click asks for the menu: on the selection when it is in
    // it, else at the place clicked.
    if over && ui.state.right_pressed {
        let at = ed.layout.hit(x, y);
        if ed.selection().is_none_or(|(a, b)| at < a || at > b) {
            ed.move_to(at, false);
        }
        out.context = Some(pointer);
    }
}

enum Edit {
    Key(KeyPress),
    Text(String),
}

/// Whether the page takes a key. What it leaves (Ctrl with most
/// letters, the function keys, Escape) is the app's.
fn wants(k: &KeyPress) -> bool {
    let m = k.mods;
    if m.alt() || m.super_key() {
        return false;
    }
    match k.key {
        Key::ArrowLeft | Key::ArrowRight | Key::ArrowUp | Key::ArrowDown | Key::Home | Key::End | Key::PageUp | Key::PageDown | Key::Backspace | Key::Delete | Key::Enter => true,
        Key::Tab | Key::Space => !m.ctrl(),
        Key::Insert => m.shift(),
        Key::Char(c) if m.ctrl() => matches!((c.to_ascii_lowercase(), m.shift()), ('a' | 'c' | 'x' | 'y', false) | ('v' | 'z', _)),
        Key::Char(_) => true,
        _ => false,
    }
}

/// This frame's keys and typed text, in the order they came.
fn take_edits(state: &mut UiState) -> Vec<Edit> {
    let mut out: Vec<(u32, Edit)> = Vec::new();
    state.keys.retain(|k| {
        let take = wants(k);
        if take {
            out.push((k.seq, Edit::Key(*k)));
        }
        !take
    });
    let alt = state.mods.alt();
    for (seq, text) in state.text_input.drain(..) {
        if !(alt && text.is_ascii()) {
            out.push((seq, Edit::Text(text)));
        }
    }
    out.sort_by_key(|(seq, _)| *seq);
    out.into_iter().map(|(_, edit)| edit).collect()
}

/// Copy the selection for other apps and for ourselves. A picture on
/// its own goes out as a picture too.
pub fn copy(state: &mut UiState, ed: &mut Editor, clip: &mut Option<crate::editor::Clip>, cut: bool) {
    let Some(taken) = (if cut { ed.cut() } else { ed.copy() }) else { return };
    state.set_clipboard(taken.text.clone());
    if let ([para], [(_, picture)]) = (&taken.paras[..], &taken.pictures[..])
        && para.is_picture()
    {
        state.set_clipboard_image(picture.image.clone());
    }
    *clip = Some(taken);
}

/// Ask for what is on the clipboard: it is put in once it has been read.
pub fn ask_paste(state: &mut UiState, view: &mut View, plain: bool) {
    (view.paste, view.paste_plain) = (true, plain);
    state.clipboard_image_wanted = true;
    state.request_rebuild = true;
}

/// The keys' part of a frame. `view_height` is how much of the page
/// shows, for a page up or down.
pub fn keys(ui: &mut Ui, ed: &mut Editor, view: &mut View, p: &mut PageIn, view_height: f32) {
    for edit in take_edits(ui.state) {
        let k = match edit {
            Edit::Text(text) => {
                ed.type_text(&text);
                continue;
            }
            Edit::Key(k) => k,
        };
        let (ctrl, shift) = (k.mods.ctrl(), k.mods.shift());
        match k.key {
            Key::ArrowLeft => ed.step(false, ctrl, shift),
            Key::ArrowRight => ed.step(true, ctrl, shift),
            Key::ArrowUp => ed.climb(-1.0, shift),
            Key::ArrowDown => ed.climb(1.0, shift),
            Key::PageUp => ed.climb(-view_height * 0.85, shift),
            Key::PageDown => ed.climb(view_height * 0.85, shift),
            Key::Home => ed.edge(false, ctrl, shift),
            Key::End => ed.edge(true, ctrl, shift),
            Key::Backspace => ed.erase(true, ctrl),
            Key::Delete => ed.erase(false, ctrl),
            Key::Enter => ed.enter(),
            Key::Tab => ed.tab(shift),
            Key::Insert => ask_paste(ui.state, view, false),
            Key::Char(c) if ctrl => match c.to_ascii_lowercase() {
                'a' => ed.select_all(),
                'c' => copy(ui.state, ed, p.clip, false),
                'x' => copy(ui.state, ed, p.clip, true),
                'v' => ask_paste(ui.state, view, shift),
                'z' if shift => drop(ed.redo()),
                'z' => drop(ed.undo()),
                'y' => drop(ed.redo()),
                _ => {}
            },
            _ => {}
        }
    }
}

/// A paste that was asked for, once the clipboard has been read: a
/// picture if there is one, else what one of our documents copied as it
/// was, else text.
pub fn pasted(ui: &mut Ui, ed: &mut Editor, view: &mut View, p: &mut PageIn) {
    if !view.paste || ui.state.clipboard_wanted || ui.state.clipboard_image_wanted {
        return;
    }
    view.paste = false;
    let text = ui.state.clipboard.replace('\r', "");
    let picture = ui.state.clipboard_image.take();
    let ours = p.clip.as_ref().filter(|clip| match &picture {
        // The picture on the clipboard is the one we put there.
        Some(image) => clip.pictures.len() == 1 && clip.paras.len() == 1 && clip.pictures[0].1.image == *image,
        None => clip.text == text,
    });
    match (view.paste_plain, ours, picture) {
        (true, _, _) => ed.paste_text(&text),
        (false, Some(clip), _) => ed.paste(clip),
        (false, None, Some(image)) => {
            if let Err(e) = ed.insert_image(&image) {
                lntrn_core::log_warn!("pasting a picture: {e}");
            }
        }
        (false, None, None) => ed.paste_text(&text),
    }
}

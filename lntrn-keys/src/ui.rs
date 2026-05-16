//! Render the TUI to a byte buffer. One big paint per redraw.

use crate::state::{AddStage, Mode, State};
use crate::term::{self, bg, bold, clear_all, clear_eol, fg, move_to, reset};

/// Lantern palette in 256-color terminal codes.
const TAN: u8 = 180;       // #e8dcc8-ish — primary text
const ORANGE: u8 = 208;    // accent
const DIM_GREY: u8 = 244;
const RED: u8 = 196;
const GREEN: u8 = 76;
const BG_PANEL: u8 = 234;  // dark grey
const BG_HILITE: u8 = 237;

pub fn render(state: &State, term_size: (u16, u16)) -> Vec<u8> {
    let mut buf = Vec::with_capacity(8192);
    let (cols, rows) = term_size;
    clear_all(&mut buf);
    draw_header(&mut buf, state, cols);
    draw_list(&mut buf, state, cols, rows);
    draw_footer(&mut buf, state, cols, rows);
    if let Mode::Revealing(secret) = &state.mode {
        draw_reveal(&mut buf, secret, cols, rows);
    } else if let Mode::Adding(stage) = &state.mode {
        draw_add_prompt(&mut buf, stage, cols, rows);
    } else if state.mode == Mode::ConfirmDelete {
        draw_confirm_delete(&mut buf, state, cols, rows);
    }
    reset(&mut buf);
    buf
}

fn draw_header(buf: &mut Vec<u8>, state: &State, cols: u16) {
    move_to(buf, 0, 0);
    bg(buf, BG_PANEL);
    fg(buf, ORANGE);
    bold(buf);
    let title = "  Lantern Keychain";
    buf.extend_from_slice(title.as_bytes());
    reset(buf);
    bg(buf, BG_PANEL);
    fg(buf, DIM_GREY);
    let count = format!("{} items", state.all.len());
    let pad = (cols as usize).saturating_sub(title.len() + count.len() + 2);
    for _ in 0..pad { buf.push(b' '); }
    buf.extend_from_slice(count.as_bytes());
    buf.extend_from_slice(b"  ");
    reset(buf);

    // search line
    move_to(buf, 0, 1);
    fg(buf, DIM_GREY);
    buf.extend_from_slice(b"  /  ");
    reset(buf);
    fg(buf, TAN);
    if state.filter.is_empty() {
        fg(buf, DIM_GREY);
        buf.extend_from_slice(b"search (press / to filter)");
    } else {
        buf.extend_from_slice(state.filter.as_bytes());
        if state.mode == Mode::Filtering {
            bg(buf, ORANGE); buf.push(b' '); reset(buf);
        }
    }
    clear_eol(buf);
}

fn draw_list(buf: &mut Vec<u8>, state: &State, cols: u16, rows: u16) {
    let list_top: u16 = 3;
    let list_bot = rows.saturating_sub(2);
    let visible = state.visible();
    for (slot, row) in (list_top..list_bot).enumerate() {
        move_to(buf, 0, row);
        if let Some(idx) = visible.get(slot) {
            let item = &state.all[*idx];
            let selected = slot == state.cursor;
            if selected {
                bg(buf, BG_HILITE);
                fg(buf, ORANGE);
                buf.extend_from_slice(b" \xe2\x96\xb8 "); // ▸
            } else {
                buf.extend_from_slice(b"   ");
            }
            fg(buf, if selected { TAN } else { TAN });
            if selected { bold(buf); }
            let label_w = ((cols as usize) / 3).max(20).min(40);
            let label = truncate(&item.label, label_w);
            buf.extend_from_slice(label.as_bytes());
            for _ in label.chars().count()..label_w { buf.push(b' '); }
            reset(buf);
            if selected { bg(buf, BG_HILITE); }
            fg(buf, DIM_GREY);
            let attrs_summary = attrs_to_string(item, (cols as usize).saturating_sub(label_w + 6));
            buf.extend_from_slice(attrs_summary.as_bytes());
        }
        clear_eol(buf);
    }
    if visible.is_empty() {
        move_to(buf, 4, list_top);
        fg(buf, DIM_GREY);
        if state.all.is_empty() {
            buf.extend_from_slice("(no keys yet — press 'a' to add your first)".as_bytes());
        } else {
            buf.extend_from_slice(b"(no matches for filter)");
        }
        reset(buf);
    }
}

fn draw_footer(buf: &mut Vec<u8>, state: &State, cols: u16, rows: u16) {
    let row = rows.saturating_sub(2);
    move_to(buf, 0, row);
    bg(buf, BG_PANEL);
    fg(buf, DIM_GREY);
    for _ in 0..cols { buf.push(b' '); }
    move_to(buf, 1, row);
    fg(buf, TAN);
    buf.extend_from_slice(b" ");
    fg(buf, ORANGE); buf.extend_from_slice(b"\xe2\x86\xb5"); // ↵
    fg(buf, TAN); buf.extend_from_slice(b" copy   ");
    fg(buf, ORANGE); buf.extend_from_slice(b"space"); fg(buf, TAN); buf.extend_from_slice(b" reveal   ");
    fg(buf, ORANGE); buf.extend_from_slice(b"a"); fg(buf, TAN); buf.extend_from_slice(b" add   ");
    fg(buf, ORANGE); buf.extend_from_slice(b"d"); fg(buf, TAN); buf.extend_from_slice(b" delete   ");
    fg(buf, ORANGE); buf.extend_from_slice(b"/"); fg(buf, TAN); buf.extend_from_slice(b" filter   ");
    fg(buf, ORANGE); buf.extend_from_slice(b"q"); fg(buf, TAN); buf.extend_from_slice(b" quit");
    reset(buf);

    move_to(buf, 0, rows.saturating_sub(1));
    fg(buf, GREEN);
    let _ = cols;
    buf.extend_from_slice(b"  ");
    buf.extend_from_slice(state.status.as_bytes());
    clear_eol(buf);
    reset(buf);
}

fn draw_reveal(buf: &mut Vec<u8>, secret: &str, cols: u16, rows: u16) {
    let w = (cols as usize).min(80);
    let lines = wrap(secret, w.saturating_sub(4));
    let h = (lines.len() + 4) as u16;
    let x = (cols.saturating_sub(w as u16)) / 2;
    let y = (rows.saturating_sub(h)) / 2;
    // border + body
    fg(buf, ORANGE);
    move_to(buf, x, y);
    buf.push(b'+');
    for _ in 0..(w - 2) { buf.push(b'-'); }
    buf.push(b'+');
    for i in 1..(h - 1) {
        move_to(buf, x, y + i);
        bg(buf, BG_PANEL);
        fg(buf, ORANGE);
        buf.push(b'|');
        fg(buf, TAN);
        for _ in 0..(w - 2) { buf.push(b' '); }
        fg(buf, ORANGE);
        buf.push(b'|');
        reset(buf);
    }
    move_to(buf, x, y + h - 1);
    fg(buf, ORANGE);
    buf.push(b'+');
    for _ in 0..(w - 2) { buf.push(b'-'); }
    buf.push(b'+');
    // header
    move_to(buf, x + 2, y + 1);
    bold(buf); fg(buf, ORANGE);
    buf.extend_from_slice(b"Revealed secret");
    reset(buf);
    fg(buf, DIM_GREY);
    buf.extend_from_slice(b"  (press any key to hide)");
    // body
    for (i, ln) in lines.iter().enumerate() {
        move_to(buf, x + 2, y + 2 + i as u16);
        fg(buf, TAN);
        buf.extend_from_slice(ln.as_bytes());
    }
    reset(buf);
}

fn draw_add_prompt(buf: &mut Vec<u8>, stage: &AddStage, cols: u16, rows: u16) {
    let w = (cols as usize).min(70);
    let h: u16 = 7;
    let x = (cols.saturating_sub(w as u16)) / 2;
    let y = (rows.saturating_sub(h)) / 2;
    fg(buf, ORANGE);
    move_to(buf, x, y); buf.push(b'+'); for _ in 0..(w-2) { buf.push(b'-'); } buf.push(b'+');
    for i in 1..(h-1) {
        move_to(buf, x, y+i);
        bg(buf, BG_PANEL); fg(buf, ORANGE); buf.push(b'|');
        for _ in 0..(w-2) { buf.push(b' '); }
        buf.push(b'|'); reset(buf);
    }
    move_to(buf, x, y+h-1); fg(buf, ORANGE);
    buf.push(b'+'); for _ in 0..(w-2) { buf.push(b'-'); } buf.push(b'+');

    let title = match stage {
        AddStage::Name(_) => "Add a key  (step 1 of 2)",
        AddStage::Secret { .. } => "Add a key  (step 2 of 2)",
    };
    move_to(buf, x + 2, y + 1); bold(buf); fg(buf, ORANGE);
    buf.extend_from_slice(title.as_bytes()); reset(buf);

    move_to(buf, x + 2, y + 2); fg(buf, DIM_GREY);
    match stage {
        AddStage::Name(_) => {
            buf.extend_from_slice("What you'll call this key later, e.g. \"GitHub PAT\"".as_bytes());
        }
        AddStage::Secret { name, .. } => {
            buf.extend_from_slice(b"Storing: ");
            fg(buf, TAN);
            buf.extend_from_slice(name.as_bytes());
        }
    }
    reset(buf);

    move_to(buf, x + 2, y + 4); fg(buf, TAN); bold(buf);
    let (prompt_label, value, masked): (&str, &str, bool) = match stage {
        AddStage::Name(v) => ("Name", v.as_str(), false),
        AddStage::Secret { value, .. } => ("Secret (typing hidden)", value.as_str(), true),
    };
    buf.extend_from_slice(prompt_label.as_bytes());
    buf.extend_from_slice(b": "); reset(buf);
    fg(buf, TAN);
    if masked {
        for _ in value.chars() { buf.push(b'*'); }
    } else {
        buf.extend_from_slice(value.as_bytes());
    }
    bg(buf, ORANGE); buf.push(b' '); reset(buf);
    move_to(buf, x + 2, y + 5);
    fg(buf, DIM_GREY);
    buf.extend_from_slice(b"Enter = next   Esc = cancel");
    reset(buf);
}

fn draw_confirm_delete(buf: &mut Vec<u8>, state: &State, cols: u16, rows: u16) {
    let label = state.selected().map(|i| i.label.clone()).unwrap_or_default();
    let msg = format!("Delete \"{label}\"?  (y / n)");
    let w = (msg.len() + 6).min(cols as usize);
    let h: u16 = 3;
    let x = (cols.saturating_sub(w as u16)) / 2;
    let y = (rows.saturating_sub(h)) / 2;
    fg(buf, RED);
    move_to(buf, x, y); buf.push(b'+'); for _ in 0..(w-2) { buf.push(b'-'); } buf.push(b'+');
    move_to(buf, x, y+1);
    bg(buf, BG_PANEL); fg(buf, RED); buf.push(b'|');
    fg(buf, TAN);
    buf.extend_from_slice(b" ");
    buf.extend_from_slice(msg.as_bytes());
    for _ in (msg.len()+2)..(w-1) { buf.push(b' '); }
    fg(buf, RED); buf.push(b'|'); reset(buf);
    move_to(buf, x, y+2); fg(buf, RED);
    buf.push(b'+'); for _ in 0..(w-2) { buf.push(b'-'); } buf.push(b'+');
    reset(buf);
}

// ── helpers ─────────────────────────────────────────────────────────────────

fn attrs_to_string(item: &crate::secret::Item, max: usize) -> String {
    // Skip `name` (redundant with label) and any FDO/xdg machinery — we just
    // want to show the human-meaningful tags that a non-lkeys client (Git,
    // Chromium, etc.) might have set.
    let mut keys: Vec<&String> = item.attributes.keys()
        .filter(|k| !k.starts_with("xdg:") && k.as_str() != "name")
        .collect();
    keys.sort();
    let mut out = String::new();
    for k in keys {
        if !out.is_empty() { out.push_str(" · "); }
        out.push_str(&format!("{}={}", k, item.attributes[k]));
        if out.chars().count() >= max { break; }
    }
    truncate(&out, max)
}

fn truncate(s: &str, w: usize) -> String {
    let n = s.chars().count();
    if n <= w { return s.to_string(); }
    let mut out: String = s.chars().take(w.saturating_sub(1)).collect();
    out.push('…');
    out
}

fn wrap(s: &str, w: usize) -> Vec<String> {
    if w == 0 { return vec![s.to_string()]; }
    let mut out = Vec::new();
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let end = (i + w).min(chars.len());
        out.push(chars[i..end].iter().collect());
        i = end;
    }
    if out.is_empty() { out.push(String::new()); }
    out
}

// Re-export term helpers for main loop convenience.
pub use term::Term;

//! What shows under a Bluetooth device's row: what pairing it asks (a
//! code to check against the device's, a PIN or passkey to type here, a
//! code to type on it), a file it wants to send, and files on their way
//! to or from it. Each answers with a click, or Enter in its field.

use std::time::Duration;

use lntrn_kit::look;
use lntrn_math::{Rect, Vec2};
use lntrn_status::bluetooth::{Asked, Ended, Offer, Question, Reply, Transfer};
use lntrn_status::secret;
use lntrn_text::TextStyle;
use lntrn_ui::{TextOpts, Ui};

use super::super::kit::{self, LINE_H};
use super::Ask;

/// A question's strip, or an offer's; a transfer's; logical.
pub const STRIP_H: f64 = 96.0;
pub const TRANSFER_H: f64 = 64.0;
/// How long an ended transfer still shows.
pub const ENDED_SHOWS: Duration = Duration::from_secs(60);
/// Where a strip's insides start, past the row's picture; its field's
/// height; logical.
const INDENT: f64 = 52.0;
const FIELD_H: f64 = 46.0;
const CODE: f64 = 28.0;

/// What is typed in the Bluetooth popup, and what is open.
#[derive(Default)]
pub struct Entry {
    /// "Add a device…" is open: it scans, and is seen.
    pub adding: bool,
    /// The PIN or passkey being typed, and the question it's for.
    typed: String,
    typed_for: u64,
    /// A paired device (by path) whose Forget was pressed once, and when.
    pub(super) forgetting: Option<(String, f64)>,
    /// How far the nearby list is scrolled.
    pub(super) nearby_scroll: f64,
}

impl Entry {
    /// The popup goes: what was typed is wiped, adding stops.
    pub fn close(&mut self) {
        secret::wipe(&mut self.typed);
        (self.adding, self.typed_for, self.forgetting, self.nearby_scroll) = (false, 0, None, 0.0);
    }
}

/// `482913` as `482 913`, easier to check.
fn spaced(code: &str) -> String {
    if code.len() == 6 && code.is_ascii() { format!("{} {}", &code[..3], &code[3..]) } else { code.to_owned() }
}

/// A size in bytes as said: `2.3 MB`.
pub fn size(bytes: u64) -> String {
    let b = bytes as f64;
    match bytes {
        0..1_000 => format!("{bytes} B"),
        1_000..1_000_000 => format!("{:.0} KB", b / 1e3),
        1_000_000..1_000_000_000 => format!("{:.1} MB", b / 1e6),
        _ => format!("{:.1} GB", b / 1e9),
    }
}

/// The strip's line along its top, and the row under it for buttons or a
/// field, in `rect`.
fn parts(ui: &Ui, rect: Rect) -> (Rect, Rect) {
    let m = ui.m;
    let (left, right) = (rect.min.x + m.px(INDENT), rect.max.x - m.px(10.0));
    let line = Rect::new(Vec2::new(left, rect.min.y + m.px(2.0)), Vec2::new(right, rect.min.y + m.px(2.0 + LINE_H)));
    let row = Rect::new(Vec2::new(left, line.max.y + m.px(6.0)), Vec2::new(right, line.max.y + m.px(6.0 + FIELD_H)));
    (line, row)
}

/// Two buttons at the right end of `row`, `yes` left of `no`. Returns
/// which was clicked.
fn buttons(ui: &mut Ui, id: &str, row: Rect, yes: &str, no: &str, ready: bool) -> (bool, bool, f64) {
    let m = ui.m;
    let cy = row.center().y;
    let (said_no, no_rect) = kit::button(ui, ui.id(id).with("no"), row.max.x, cy, no);
    let ink = ready.then_some(ui.theme.accent);
    let (said_yes, yes_rect) = kit::button_with(ui, ui.id(id).with("yes"), no_rect.min.x - m.px(10.0), cy, yes, ink);
    (said_yes, said_no, yes_rect.min.x - m.px(10.0))
}

/// The strip for `q` in `rect`. Returns the person's answer, once given.
pub(super) fn question(ui: &mut Ui, entry: &mut Entry, rect: Rect, q: &Question) -> Option<Ask> {
    let m = ui.m;
    let (line, row) = parts(ui, rect);
    let (ink, dim) = (ui.theme.text, ui.theme.text_dim);
    let name = kit::name_style(ui);
    let answer = |reply| Some(Ask::Answer(q.id, reply));
    match &q.kind {
        Asked::Confirm(code) => {
            kit::line(ui, line, &format!("Does it show {}?", spaced(&format!("{code:06}"))), &name.clone().bold(), ink);
            let (yes, no, _) = buttons(ui, "question", row, "Pair", "Cancel", true);
            if yes { answer(Reply::Yes) } else if no { answer(Reply::No) } else { None }
        }
        Asked::Allow => {
            kit::line(ui, line, "It wants to pair with this computer", &name, ink);
            let (yes, no, _) = buttons(ui, "question", row, "Pair", "Turn down", true);
            if yes { answer(Reply::Yes) } else if no { answer(Reply::No) } else { None }
        }
        Asked::Show(code, typed) => {
            kit::line(ui, line, "Type this on it, then press Enter", &kit::dim_style(ui), dim);
            let code = spaced(code);
            let style = TextStyle::new(m.px(CODE) as f32).bold();
            ui.text_in_rect(&code, &style, row, ui.theme.accent);
            if *typed > 0 {
                let w = ui.measure(&code, &style) + m.px(16.0);
                let note = Rect::new(Vec2::new(row.min.x + w, row.min.y), row.max);
                kit::line(ui, note, &format!("{typed} typed"), &kit::dim_style(ui), dim);
            }
            None
        }
        Asked::Pin | Asked::Passkey => {
            let passkey = q.kind == Asked::Passkey;
            if entry.typed_for != q.id {
                secret::wipe(&mut entry.typed);
                entry.typed_for = q.id;
            }
            let said = if passkey { "The 6 digits it shows" } else { "The PIN it asks for (often 0000)" };
            kit::line(ui, line, said, &kit::dim_style(ui), dim);
            let ready = if passkey { entry.typed.len() == 6 } else { (1..=16).contains(&entry.typed.len()) };
            let (yes, no, left) = buttons(ui, "question", row, "Pair", "Cancel", ready);
            let id = ui.id("pin").with_u64(q.id);
            ui.request_focus_once(id);
            let field = Rect::new(row.min, Vec2::new(left, row.max.y));
            let r = ui.text_edit_core_with(id, field, &mut entry.typed, TextOpts { placeholder: if passkey { "123456" } else { "0000" }, ..TextOpts::default() });
            if passkey && r.changed {
                entry.typed.retain(|c| c.is_ascii_digit());
                entry.typed.truncate(6);
            }
            if no {
                return answer(Reply::No);
            }
            let ready = if passkey { entry.typed.len() == 6 } else { (1..=16).contains(&entry.typed.len()) };
            if !(ready && (yes || r.committed)) {
                return None;
            }
            let typed = std::mem::take(&mut entry.typed);
            if passkey { answer(Reply::Passkey(typed.parse().unwrap_or_default())) } else { answer(Reply::Pin(typed)) }
        }
    }
}

/// The strip for a file `o` offers, in `rect`. Returns the person's
/// answer, once given.
pub(super) fn offer(ui: &mut Ui, rect: Rect, o: &Offer) -> Option<Ask> {
    let (line, row) = parts(ui, rect);
    let name = kit::name_style(ui);
    kit::line(ui, line, &format!("Wants to send {} ({})", o.name, size(o.size)), &name, ui.theme.text);
    let (yes, no, _) = buttons(ui, "offer", row, "Accept", "Decline", true);
    if yes {
        Some(Ask::TakeFile(o.id, true))
    } else if no {
        Some(Ask::TakeFile(o.id, false))
    } else {
        None
    }
}

/// Whether `t` still shows: while it goes, and a while after it ends.
pub fn shows(t: &Transfer) -> bool {
    t.end.as_ref().is_none_or(|(_, at)| at.elapsed() < ENDED_SHOWS)
}

/// The strip for `t` in `rect`: what it is, and how far it got.
pub(super) fn transfer(ui: &mut Ui, rect: Rect, t: &Transfer) {
    let m = ui.m;
    let (line, _) = parts(ui, rect);
    let style = kit::dim_style(ui);
    let (ink, accent) = (ui.theme.text, ui.theme.accent);
    let (said, color) = match &t.end {
        None if t.incoming => (format!("Receiving {}", t.name), ink),
        None => (format!("Sending {}", t.name), ink),
        Some((Ended::Saved(_), _)) => (format!("Saved {} in Downloads", t.name), accent),
        Some((Ended::Sent, _)) => (format!("Sent {}", t.name), accent),
        Some((Ended::Failed(why), _)) => (format!("{}: {why}", t.name), look::BAD),
    };
    let share = if t.size > 0 { (t.done as f64 / t.size as f64).clamp(0.0, 1.0) } else { 0.0 };
    let mut room = line;
    if t.end.is_none() && t.size > 0 {
        let pct = format!("{}%", (share * 100.0).round());
        let w = ui.measure(&pct, &style);
        ui.text_right(&pct, &style, line, ui.theme.text_dim);
        room = Rect::new(line.min, Vec2::new(line.max.x - w - m.px(10.0), line.max.y));
    }
    kit::line(ui, room, &said, &style, color);
    if t.end.is_none() {
        let track = Rect::new(Vec2::new(line.min.x, line.max.y + m.px(8.0)), Vec2::new(line.max.x, line.max.y + m.px(16.0)));
        ui.draw.rounded_rect(track, track.height() * 0.5, look::TRACK);
        let fill = Rect::new(track.min, Vec2::new(track.min.x + (track.width() * share).max(track.height()), track.max.y));
        ui.draw.rounded_rect(fill, track.height() * 0.5, accent);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_and_sizes_read_easily() {
        assert_eq!((spaced("482913"), spaced("0000")), ("482 913".to_owned(), "0000".to_owned()));
        assert_eq!((size(512), size(48_200), size(2_340_000), size(4_700_000_000)), ("512 B".into(), "48 KB".into(), "2.3 MB".into(), "4.7 GB".into()));
    }
}

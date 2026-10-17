//! What is typed in the WiFi popup: the password of a network in range,
//! in a box that opens under its row, or a hidden network's name (and
//! password, if it has one), in a box that opens from the row under the
//! list. The password shows as dots until the eye is clicked. Enter
//! joins, Escape closes the box. A password that didn't work leaves the
//! box open, saying so, to try again; one that did, or the box closing,
//! wipes what was typed.

use lntrn_kit::look;
use lntrn_math::{Color, Rect, Vec2};
use lntrn_status::secret;
use lntrn_status::wifi::{self, Target, WifiState};
use lntrn_ui::{CursorIcon, Sense, TextOpts, Ui, WidgetId};

use super::super::kit::{self, LINE_H, ROW_H};
use super::Ask;
use crate::tray::glyphs::{Glyph, Glyphs};

/// The box under a network's row; the hidden network's, its row
/// included; logical.
pub const BOX_H: f64 = 92.0;
pub const HIDDEN_H: f64 = 200.0;
/// A field's height and the eye's width; between them and Join.
const FIELD_H: f64 = 46.0;
const EYE_W: f64 = 52.0;
const GAP: f64 = 8.0;
/// Where the fields start, past the row's picture, logical.
const INDENT: f64 = 52.0;
/// A hidden network's name, at most (an SSID's bytes).
const NAME_MAX: usize = 32;

/// Whose box is open.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Open {
    /// A network in range's, by iwd's path.
    Network(String),
    Hidden,
}

#[derive(Default)]
pub struct Entry {
    pub open: Option<Open>,
    /// A hidden network's name, and the password, as typed.
    name: String,
    password: String,
    /// The password shows as itself.
    shown: bool,
    /// The join sent, until it is known how it went.
    sent: Option<Target>,
    /// Typed since the last try failed: its reason goes.
    retyped: bool,
    /// Focus goes back to the password field next frame.
    refocus: bool,
    /// The box just opened: the list scrolls it into view.
    pub(super) reveal: bool,
    /// A saved network (by path) whose Forget was pressed once, and when:
    /// a second press soon after forgets it.
    pub(super) forgetting: Option<(String, f64)>,
}

impl Entry {
    /// Open the box `open` (closing any other), or close it when it is.
    pub fn toggle(&mut self, open: Open) {
        let again = self.open.as_ref() == Some(&open);
        self.close();
        if !again {
            (self.open, self.reveal) = (Some(open), true);
        }
    }

    /// Close the box, wiping what was typed.
    pub fn close(&mut self) {
        secret::wipe(&mut self.password);
        self.name.clear();
        (self.open, self.shown, self.sent, self.retyped, self.refocus, self.reveal) = (None, false, None, false, false, false);
    }

    /// Follow `s`: a join sent that ended well closes the box, and one
    /// that failed leaves it open to try again. A network gone (out of
    /// range, saved after all, WiFi off) takes its box with it.
    pub fn follow(&mut self, s: &WifiState) {
        if let Some(sent) = self.sent.clone()
            && s.joining.as_ref() != Some(&sent)
        {
            if s.failed.as_ref() == Some(&sent) {
                // Focused again from the field's next frame.
                (self.sent, self.retyped, self.refocus) = (None, false, true);
            } else {
                self.close();
            }
        }
        let gone = |path: &String| !s.networks.iter().any(|n| &n.path == path && n.asks_password());
        if self.sent.is_none() && matches!(&self.open, Some(Open::Network(path)) if gone(path)) {
            self.close();
        }
    }

    /// Whether `s`'s error is about this box's join, so the box says it
    /// and the popup's lines don't.
    pub fn claims(&self, s: &WifiState) -> bool {
        match (&self.open, &s.failed) {
            (Some(Open::Network(open)), Some(Target::Network(failed))) => open == failed,
            (Some(Open::Hidden), Some(Target::Hidden(_))) => true,
            _ => false,
        }
    }

    /// Why the last try failed, until something new is typed.
    fn trouble<'s>(&self, s: &'s WifiState) -> Option<&'s str> {
        if self.retyped || self.sent.is_some() || !self.claims(s) { None } else { s.error.as_deref() }
    }

    /// The box under the row of the network at `path`, in `rect`: its
    /// password field, the eye and Join, and a line under them.
    pub(super) fn password_box(&mut self, ui: &mut Ui, glyphs: &mut Glyphs, rect: Rect, path: &str, s: &WifiState) -> Option<Ask> {
        let m = ui.m;
        let (left, right) = (rect.min.x + m.px(INDENT), rect.max.x - m.px(10.0));
        let row = Rect::new(Vec2::new(left, rect.min.y + m.px(4.0)), Vec2::new(right, rect.min.y + m.px(4.0 + FIELD_H)));
        let id = ui.id("password").with(path);
        ui.request_focus_once(id);
        let fits = wifi::fits(&self.password);
        let (enter, join) = self.password_row(ui, glyphs, id, row, fits, "Password");
        let hint = if !self.password.is_empty() && !fits { "A WiFi password is 8 to 63 characters" } else { "Enter joins, Escape closes" };
        self.note(ui, Rect::new(Vec2::new(left, row.max.y + m.px(4.0)), Vec2::new(right, row.max.y + m.px(4.0 + LINE_H))), s, hint);
        let go = (enter || join) && wifi::fits(&self.password) && self.sent.is_none();
        go.then(|| {
            (self.sent, self.retyped) = (Some(Target::Network(path.to_owned())), false);
            Ask::JoinWith(path.to_owned(), self.password.clone())
        })
    }

    /// The row under the list that joins a hidden network, and its box
    /// under it while open: the name, the password, the eye and Join.
    pub(super) fn hidden(&mut self, ui: &mut Ui, glyphs: &mut Glyphs, rect: Rect, s: &WifiState) -> Option<Ask> {
        let m = ui.m;
        let id = ui.id("hidden");
        let head = Rect::from_min_size(rect.min, Vec2::new(rect.width(), m.px(ROW_H)));
        if self.open == Some(Open::Hidden) {
            ui.draw.rounded_rect(rect, m.px(12.0), Color::WHITE.fade(0.05));
        }
        if kit::row(ui, id, head, false).clicked {
            self.toggle(Open::Hidden);
            ui.state.request_rebuild = true;
        }
        let pic = Rect::from_min_size(Vec2::new(head.min.x + m.px(10.0), (head.center().y - m.px(14.0)).round()), Vec2::splat(m.px(28.0).round()));
        glyphs.draw(ui, Glyph::Plus, pic, ui.theme.text);
        let words = Rect::new(Vec2::new(pic.max.x + m.px(14.0), head.min.y), Vec2::new(head.max.x - m.px(10.0), head.max.y));
        kit::line(ui, words, "Join a hidden network…", &kit::name_style(ui), ui.theme.text);
        if self.open != Some(Open::Hidden) {
            return None;
        }

        let (left, right) = (rect.min.x + m.px(INDENT), rect.max.x - m.px(10.0));
        let name_at = Rect::new(Vec2::new(left, head.max.y + m.px(4.0)), Vec2::new(right, head.max.y + m.px(4.0 + FIELD_H)));
        let (name_id, password_id) = (id.with("name"), id.with("password"));
        ui.request_focus_once(name_id);
        let r = ui.text_edit_core_with(name_id, name_at, &mut self.name, TextOpts { placeholder: "Network name", ..TextOpts::default() });
        self.retyped |= r.changed;
        if r.committed {
            ui.state.focus = Some(password_id);
        }
        let name = self.name.trim().to_owned();
        let name_fits = !name.is_empty() && name.len() <= NAME_MAX;
        let password_fits = self.password.is_empty() || wifi::fits(&self.password);
        let row = Rect::new(Vec2::new(left, name_at.max.y + m.px(GAP)), Vec2::new(right, name_at.max.y + m.px(GAP + FIELD_H)));
        let (enter, join) = self.password_row(ui, glyphs, password_id, row, name_fits && password_fits, "Password, if any");
        let hint = match () {
            _ if name.len() > NAME_MAX => "A network's name is at most 32 letters",
            _ if !password_fits => "A WiFi password is 8 to 63 characters",
            _ => "No password for an open network",
        };
        self.note(ui, Rect::new(Vec2::new(left, row.max.y + m.px(4.0)), Vec2::new(right, row.max.y + m.px(4.0 + LINE_H))), s, hint);
        let go = (enter || join) && name_fits && password_fits && self.sent.is_none();
        go.then(|| {
            (self.sent, self.retyped) = (Some(Target::Hidden(name.clone())), false);
            Ask::JoinHidden(name, self.password.clone())
        })
    }

    /// The password's field across `row` (focused as `id`), the eye and
    /// Join at its right, Join lit while `ready`. Returns whether Enter
    /// was pressed in the field, and whether Join was clicked.
    fn password_row(&mut self, ui: &mut Ui, glyphs: &mut Glyphs, id: WidgetId, row: Rect, ready: bool, placeholder: &str) -> (bool, bool) {
        let m = ui.m;
        if std::mem::take(&mut self.refocus) {
            ui.state.focus = Some(id);
        }
        let label = if self.sent.is_some() { "Joining…" } else { "Join" };
        let ink = (ready && self.sent.is_none()).then_some(ui.theme.text);
        let (join, button) = kit::button_with(ui, id.with("join"), row.max.x, row.center().y, label, ink);
        let eye = Rect::new(Vec2::new(button.min.x - m.px(GAP + EYE_W), row.min.y), Vec2::new(button.min.x - m.px(GAP), row.max.y));
        if eye_button(ui, glyphs, id.with("eye"), eye, self.shown) {
            // The click took the focus: it goes back to the typing.
            (self.shown, self.refocus) = (!self.shown, true);
            ui.state.request_rebuild = true;
        }
        let field = Rect::new(row.min, Vec2::new(eye.min.x - m.px(GAP), row.max.y));
        let r = ui.text_edit_core_with(id, field, &mut self.password, TextOpts { placeholder, password: !self.shown, ..TextOpts::default() });
        self.retyped |= r.changed;
        (r.committed, join)
    }

    /// The line under the fields: why the last try failed, that it is
    /// joining, or else `hint`.
    fn note(&self, ui: &mut Ui, rect: Rect, s: &WifiState, hint: &str) {
        let style = kit::dim_style(ui);
        let (text, color) = match self.trouble(s) {
            Some(why) => (why, look::BAD),
            None if self.sent.is_some() => ("Joining…", ui.theme.text),
            None => (hint, ui.theme.text_dim),
        };
        kit::line(ui, rect, text, &style, color);
    }
}

/// The eye in `rect` that shows the password, or hides it again when it
/// is `shown`. Returns whether it was clicked.
fn eye_button(ui: &mut Ui, glyphs: &mut Glyphs, id: WidgetId, rect: Rect, shown: bool) -> bool {
    let m = ui.m;
    let r = ui.interact(id, rect, Sense::CLICK);
    if r.hovered {
        ui.state.cursor_icon = CursorIcon::Pointer;
    }
    let face = if r.held { 0.16 } else if r.hovered { 0.12 } else { 0.07 };
    ui.draw.rounded_rect(rect, m.px(12.0), Color::WHITE.fade(face));
    let pic = Rect::from_center_size(rect.center(), Vec2::splat(m.px(28.0).round())).round();
    glyphs.draw(ui, if shown { Glyph::EyeOff } else { Glyph::Eye }, pic, ui.theme.text);
    ui.tooltip(&r, if shown { "Hide the password" } else { "Show the password" });
    r.clicked
}

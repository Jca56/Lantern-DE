//! The dialogs that ask for a profile's name (saving a new one, renaming
//! one) and the ones that ask before saving over or deleting one. The
//! name field is ours, between the dialog's words and its buttons.

use lntrn_kit::{controls, look, small_style};
use lntrn_ui::{Action, Dialog, FILL, HostCx, ShellRequest, Ui};
use lntrn_math::Vec2;

use crate::app::App;
use crate::profiles::{self, NAME_MAX};

const NAME: &str = "profile-name";

/// What the open dialog is about.
#[derive(Clone, Debug, Default)]
pub struct Asking {
    /// What is typed in the name field.
    pub name: String,
    /// The profile it is about: the one renamed, saved over or deleted.
    pub target: Option<usize>,
}

/// Ask for a new profile's name.
pub fn new_profile(app: &mut App) -> ShellRequest {
    app.asking = Asking::default();
    ShellRequest::Dialog(Dialog::confirm("Save as a Profile", "Everything set on the Graphics, Fans and Processor pages, kept under a name.", "Save", Action::new("profile.new-go")).content(NAME))
}

pub fn rename(app: &mut App, i: usize) -> Option<ShellRequest> {
    let name = app.profiles.list.get(i)?.name.clone();
    app.asking = Asking { name: name.clone(), target: Some(i) };
    Some(ShellRequest::Dialog(Dialog::confirm(&format!("Rename \u{201C}{name}\u{201D}"), "", "Rename", Action::new("profile.rename-go")).content(NAME)))
}

pub fn save_over(app: &mut App, i: usize) -> Option<ShellRequest> {
    let name = app.profiles.list.get(i)?.name.clone();
    app.asking = Asking { name: String::new(), target: Some(i) };
    Some(ShellRequest::Dialog(Dialog::confirm(&format!("Save over \u{201C}{name}\u{201D}?"), "Its settings are replaced with the ones on the pages now.", "Save Over", Action::new("profile.update-go"))))
}

pub fn delete(app: &mut App, i: usize) -> Option<ShellRequest> {
    let name = app.profiles.list.get(i)?.name.clone();
    app.asking = Asking { name: String::new(), target: Some(i) };
    Some(ShellRequest::Dialog(Dialog::confirm(&format!("Delete \u{201C}{name}\u{201D}?"), "The profile is gone for good. What is applied now stays as it is.", "Delete", Action::new("profile.delete-go"))))
}

/// Whether the dialog button that runs `action` can be pressed yet.
pub fn ready(app: &App, action: &str) -> bool {
    match action {
        "profile.new-go" => app.profiles.can_name(&app.asking.name, None),
        "profile.rename-go" => app.profiles.can_name(&app.asking.name, app.asking.target),
        _ => true,
    }
}

/// Draw the widgets of the dialog `key`. `true` when one changed.
pub fn draw(app: &mut App, key: &str, ui: &mut Ui, cx: &mut HostCx) -> bool {
    if key != NAME {
        return false;
    }
    let m = ui.m;
    let field = ui.alloc(Vec2::new(FILL, m.px(controls::BUTTON_H)));
    let id = ui.id("name");
    ui.request_focus_once(id);
    let typed = controls::text_field(ui, id, field, &mut app.asking.name, "Its name");
    if typed.committed {
        cx.request(ShellRequest::DialogDefault);
    }
    // Why it can't be saved yet, when it can't.
    let small = small_style(ui);
    let said = ui.alloc(Vec2::new(FILL, f64::from(small.line_height())));
    let taken = app.profiles.find(&app.asking.name).filter(|i| Some(*i) != app.asking.target);
    if let Some(i) = taken {
        ui.text_in_rect(&format!("\u{201C}{}\u{201D} is taken.", app.profiles.list[i].name), &small, said, look::warn());
    } else if app.asking.name.chars().count() > NAME_MAX {
        ui.text_in_rect(&format!("Kept to its first {NAME_MAX} letters: {}", profiles::tidy(&app.asking.name)), &small, said, look::text_dim());
    }
    typed.changed
}

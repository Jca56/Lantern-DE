//! The Branches tab: every local branch with how far it is from the base
//! branch, whether it is on the remote, and its newest commit; and the
//! buttons that make a branch or merge two.

use lntrn_kit::controls::{self, Kind};
use lntrn_kit::{Keep, bits, elide, look, small_style};
use lntrn_math::{Rect, Vec2};
use lntrn_ui::{AreaCx, FILL, Ui};

use crate::app::App;
use crate::dialogs;
use crate::git::{self, Branch};
use crate::worker::Cmd;

const ROW_H: f64 = 86.0;

/// What a press here asks for.
enum Act {
    Switch(String),
    New,
    Merge,
}

pub fn draw(app: &mut App, ui: &mut Ui, cx: &mut AreaCx<()>) {
    let Some(repo) = &app.repo else { return };
    let idle = !app.busy;
    let mut act = None;
    ui.scroll_area("branches", None, |ui| {
        let m = ui.m;
        // The card's heading, with New Branch… and Merge… level with it.
        ui.space(m.px(6.0));
        let bar = ui.alloc(Vec2::new(FILL, m.px(controls::BUTTON_H)));
        let heading = small_style(ui).bold();
        ui.text_in_rect(&format!("BRANCHES · {}", repo.branches.len()), &heading, Rect::new(Vec2::new(bar.min.x + m.px(5.0), bar.min.y), bar.max), look::TEXT_DIM);
        let mut right = bar.max.x;
        for (key, text, kind, what) in [("new", "New Branch…", Kind::Primary, Act::New), ("merge", "Merge…", Kind::Plain, Act::Merge)] {
            let w = controls::button_width(ui, text);
            let rect = Rect::new(Vec2::new(right - w, bar.min.y), Vec2::new(right, bar.max.y));
            right -= w + m.px(10.0);
            let id = ui.id(key);
            if controls::button_of(ui, id, rect, text, kind, idle && (kind == Kind::Primary || repo.branches.len() > 1)) {
                act = Some(what);
            }
        }
        if repo.branches.is_empty() {
            lntrn_kit::note(ui, if repo.status.is_some() { "No branches yet: the first commit makes one." } else { "Reading the branches…" });
            return;
        }
        let names: Vec<String> = repo.branches.iter().map(|b| b.name.clone()).collect();
        let base = git::base_branch(&names).unwrap_or("");
        lntrn_kit::card(ui, "list", |c| {
            let h = c.ui.m.px(ROW_H);
            for (i, branch) in repo.branches.iter().enumerate() {
                if c.block(h, |ui, rect| row(ui, rect, i, branch, base, idle)) {
                    act = Some(Act::Switch(branch.name.clone()));
                }
            }
        });
        ui.space(m.px(20.0));
    });
    match act {
        Some(Act::Switch(name)) => app.send(Cmd::SwitchBranch(name), "Switching branch…"),
        Some(Act::New) => cx.request(dialogs::new_branch(app)),
        Some(Act::Merge) => cx.request(dialogs::merge(app)),
        None => {}
    }
}

/// One branch. `true` when its Switch button was pressed.
fn row(ui: &mut Ui, rect: Rect, i: usize, branch: &Branch, base: &str, idle: bool) -> bool {
    if !rect.intersects(&ui.clip()) {
        return false;
    }
    let m = ui.m;
    let accent = ui.theme.accent;
    let (style, small) = (ui.text_style(), small_style(ui));
    let (name_h, small_h) = (style.line_height() as f64, small.line_height() as f64);
    let top = (rect.center().y - (name_h + small_h + m.px(4.0)) * 0.5).round();
    let name_mid = top + name_h * 0.5;
    let left = rect.min.x + m.px(52.0);

    // What sits at the right: where it stands, or the way to go there.
    let mut right = rect.max.x - m.px(18.0);
    let mut pressed = false;
    if branch.is_current {
        let w = bits::badge_width(ui, "checked out");
        bits::badge(ui, right - w, rect.center().y, "checked out", accent);
        right -= w + m.px(12.0);
        ui.draw.circle(Vec2::new(rect.min.x + m.px(28.0), name_mid), m.px(7.0), accent);
    } else {
        let w = controls::button_width(ui, "Switch");
        let h = m.px(controls::BUTTON_H - 4.0);
        let id = ui.id("switch").with_index(i);
        pressed = controls::button_of(ui, id, Rect::from_min_size(Vec2::new(right - w, (rect.center().y - h * 0.5).round()), Vec2::new(w, h)), "Switch", Kind::Plain, idle);
        right -= w + m.px(12.0);
        ui.draw.ring(Vec2::new(rect.min.x + m.px(28.0), name_mid), m.px(7.0), m.px(2.0), look::TRACK);
    }

    // The badges after the name take their room first; the name gets
    // what is left.
    let mut badges: Vec<(String, lntrn_math::Color)> = Vec::new();
    if branch.name == base {
        badges.push(("base".to_owned(), look::TEXT_DIM));
    } else if branch.ahead > 0 || branch.behind > 0 {
        badges.push((format!("+{}", branch.ahead), look::GOOD));
        badges.push((format!("−{}", branch.behind), look::WARN));
    }
    if !branch.has_upstream {
        badges.push(("local only".to_owned(), look::WARN));
    }
    let mut badges_w = 0.0;
    for (text, _) in &badges {
        badges_w += bits::badge_width(ui, text) + m.px(8.0);
    }
    let room = (right - left - badges_w - m.px(8.0)).max(m.px(60.0));
    let name = elide(ui, &branch.name, &style, room, Keep::Start);
    ui.text_in_rect(&name, &style, Rect::from_min_size(Vec2::new(left, top), Vec2::new(room, name_h)), if branch.is_current { accent } else { look::TEXT });
    let mut x = left + ui.measure(&name, &style) + m.px(14.0);
    for (text, color) in &badges {
        x += bits::badge(ui, x, name_mid, text, *color) + m.px(8.0);
    }

    let said = if branch.last_commit.is_empty() { "No commits yet" } else { branch.last_commit.as_str() };
    let said = elide(ui, said, &small, right - left, Keep::Start);
    ui.text_in_rect(&said, &small, Rect::from_min_size(Vec2::new(left, top + name_h + m.px(4.0)), Vec2::new((right - left).max(0.0), small_h)), look::TEXT_DIM);
    pressed
}

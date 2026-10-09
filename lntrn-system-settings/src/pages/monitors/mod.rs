//! The Monitors page: every monitor the compositor has as a tile where
//! it sits on the desk, the picked one's settings under that, and
//! nothing done until Apply. What Apply does is on trial for a few
//! seconds and undoes itself unless it is kept (see `flow`). Apply sits
//! in the layout card, under the picture of what it will make, so it is
//! in view however long the settings under it get.
//!
//! - `flow`: the page's state, and how a change is applied, kept or put
//!   back.
//! - `draft`: a monitor as it is being edited, and the sums around it.
//! - `arrange`: where monitors can sit beside each other.
//! - `canvas`: the layout card's picture.
//! - `badge`: the numbers Identify puts on the monitors themselves.

mod arrange;
pub mod badge;
mod canvas;
mod draft;
#[cfg(test)]
mod fake;
mod flow;
#[cfg(test)]
mod page_tests;

use lntrn_math::{Rect, Vec2};
use lntrn_ui::{AreaCx, FILL, Ui};

pub use flow::MonitorsState;

use crate::config::Config;
use crate::kit::controls::{self, Kind};
use crate::kit::{self, bits};
use crate::look;
use crate::outputs::Link;

/// The buttons that decide things: bigger than the kit's own.
const BUTTON_H: f64 = 58.0;
const BUTTON_W: f64 = 190.0;
/// The room above and below them in the layout card.
const ACTIONS_PAD: f64 = 16.0;

pub fn draw(cfg: &mut Config, st: &mut MonitorsState, ui: &mut Ui, cx: &mut AreaCx<()>) -> bool {
    st.connect();
    if st.source.is_none() || st.link == Link::Connecting {
        kit::note(ui, "Asking the compositor what monitors there are…");
        return false;
    }
    if let Some(why) = trouble(st) {
        kit::note(ui, &why);
        ui.space(ui.m.px(10.0));
        if lone_button(ui, "Try Again") {
            st.reconnect();
        }
        return false;
    }
    if st.draft.is_empty() {
        kit::note(ui, "The compositor lists no monitors.");
        return false;
    }

    let on_trial = st.seconds_left(ui.now());
    let showed = st.showing();
    let (mut moved, mut changed) = (false, false);
    kit::caption(ui, "Layout");
    kit::card(ui, "layout", |c| {
        let (h, idle) = (c.ui.m.px(canvas::HEIGHT), st.idle());
        moved = c.block(h, |ui, rect| canvas::draw(ui, rect, &st.heads, &mut st.draft, &mut st.selected, &mut st.canvas, idle));
        if on_trial.is_none() {
            let h = c.ui.m.px(BUTTON_H + ACTIONS_PAD * 2.0);
            changed = c.block(h, |ui, rect| actions(ui, rect, st, cfg, cx));
        }
    });
    if moved {
        st.edited();
    }
    if let Some(left) = on_trial {
        changed |= trial(ui, st, cfg, left);
    }
    if let Some(notice) = &st.notice {
        ui.space(ui.m.px(6.0));
        if bits::banner(ui, "notice", &notice.text, if notice.good { look::GOOD } else { look::WARN }) {
            st.notice = None;
        }
    }
    if on_trial.is_none() {
        settings(ui, st);
    }
    if st.showing() != showed {
        ui.state.request_rebuild = true;
    }
    changed
}

/// Why there is nothing to show, when the compositor can't be asked.
fn trouble(st: &MonitorsState) -> Option<String> {
    match &st.link {
        Link::Missing => Some("This compositor doesn't let its monitors be set from here.".to_owned()),
        Link::Lost(why) => Some(format!("Lost touch with the compositor: {why}.")),
        Link::Connecting | Link::Ready => None,
    }
}

/// A button on a line of its own, at the left.
fn lone_button(ui: &mut Ui, text: &str) -> bool {
    let m = ui.m;
    let row = ui.alloc(Vec2::new(FILL, m.px(BUTTON_H)));
    let id = ui.id(text);
    controls::button(ui, id, Rect::from_min_size(row.min, Vec2::new(m.px(BUTTON_W), row.height())), text)
}

/// The picked monitor's card: whether it is on, its mode, its scale, and
/// what only the file knows of it. Which monitor that is, is picked on
/// its tile.
fn settings(ui: &mut Ui, st: &mut MonitorsState) {
    let many = st.draft.len() > 1;
    let i = st.selected.min(st.draft.len() - 1);
    let head = st.heads[i].clone();
    let size = draft::inches(head.physical_mm).map_or(String::new(), |inches| format!(" · {inches}″"));
    kit::caption(ui, &format!("Monitor {}{size}", st.label(i)));
    let mut edited = false;
    let mut refused = false;
    kit::card(ui, "monitor", |c| {
        if many {
            let mut on = st.draft[i].enabled;
            if c.switch("Use this monitor", "Off, the pointer and windows keep off it. Its panel stays lit.", &mut on) {
                // The last one on stays on: there would be nowhere to
                // switch it back from.
                if !on && st.draft.iter().filter(|d| d.enabled).count() == 1 {
                    refused = true;
                } else {
                    st.draft[i].enabled = on;
                    edited = true;
                }
            }
        }
        let d = &mut st.draft[i];
        if d.enabled && let Some(now) = draft::mode_of(&head, d) {
            let sizes = draft::resolutions(&head.modes);
            let labels: Vec<String> = sizes.iter().map(|r| draft::resolution_label(&head.modes, *r)).collect();
            let labels: Vec<&str> = labels.iter().map(String::as_str).collect();
            let mut pick = sizes.iter().position(|r| *r == (now.width, now.height)).unwrap_or(0);
            if c.dropdown("Resolution", "", &mut pick, &labels) {
                d.mode = draft::mode_at(&head.modes, sizes[pick], now.refresh).or(d.mode);
                edited = true;
            }
            let rates = draft::rates(&head.modes, (now.width, now.height));
            let labels: Vec<String> = rates.iter().map(|(r, _)| draft::rate_label(*r)).collect();
            let labels: Vec<&str> = labels.iter().map(String::as_str).collect();
            let mut pick = rates.iter().position(|(r, _)| *r == now.refresh).unwrap_or(0);
            if c.dropdown("Refresh rate", "", &mut pick, &labels) {
                d.mode = Some(rates[pick].1);
                edited = true;
            }
            let t = draft::tile(&head, d);
            let hint = format!("How big everything is drawn. The desktop on it comes to {} × {}.", t.w, t.h);
            edited |= c.slider("Scale", &hint, &mut d.scale, draft::SCALE_RANGE, draft::SCALE_STEP, kit::percent);
        }
        if many {
            let mut main = d.primary;
            if c.switch("Main monitor", "Where the Command Center lives.", &mut main) {
                // There is one main monitor, or none.
                for other in st.draft.iter_mut() {
                    other.primary = false;
                }
                st.draft[i].primary = main;
                edited = true;
            }
        }
        edited |= c.switch("Variable refresh", "A fullscreen game sets the pace, on a monitor that can follow.", &mut st.draft[i].vrr);
    });
    if edited {
        st.edited();
    }
    if refused {
        st.notice = Some(flow::Notice { text: "One monitor has to stay on.".to_owned(), good: false });
    }
}

/// The layout card's last row: Identify at the left, Reset and Apply at
/// the right, and between them what Apply is waiting on. `true` when
/// Apply wrote to the file.
fn actions(ui: &mut Ui, rect: Rect, st: &mut MonitorsState, cfg: &mut Config, cx: &mut AreaCx<()>) -> bool {
    let m = ui.m;
    let (pad, gap) = (m.px(kit::card::ROW_PAD), m.px(14.0));
    let row = Rect::new(Vec2::new(rect.min.x + pad, rect.min.y + m.px(ACTIONS_PAD)), Vec2::new(rect.max.x - pad, rect.max.y - m.px(ACTIONS_PAD)));
    // Three across, narrower when the card is.
    let w = m.px(BUTTON_W).min((row.width() - gap * 2.0) / 3.0).max(1.0);
    let at = |x: f64| Rect::from_min_size(Vec2::new(x, row.min.y), Vec2::new(w, row.height()));
    let (identify, reset, apply) = (at(row.min.x), at(row.max.x - w * 2.0 - gap), at(row.max.x - w));
    let (dirty, idle) = (st.dirty(), st.idle());

    let (identify_id, reset_id, apply_id) = (ui.id("identify"), ui.id("reset"), ui.id("apply"));
    if controls::button(ui, identify_id, identify, "Identify") {
        let on: Vec<(usize, String)> = st.draft.iter().enumerate().filter(|(_, d)| d.enabled).map(|(i, d)| (i, d.name.clone())).collect();
        st.identify.show(&on, cx);
    }
    let said = match (idle, dirty) {
        (false, _) => "Applying…",
        (true, true) => "Not applied yet",
        (true, false) => "",
    };
    let small = kit::small_style(ui);
    let words = Rect::new(Vec2::new(identify.max.x + gap, row.min.y), Vec2::new(reset.min.x - gap, row.max.y));
    if ui.measure(said, &small) <= words.width() {
        ui.text_right(said, &small, words, look::TEXT_DIM);
    }
    if controls::button_of(ui, reset_id, reset, "Reset", Kind::Plain, dirty && idle) {
        st.reset();
    }
    controls::button_of(ui, apply_id, apply, "Apply", Kind::Primary, dirty && idle) && st.apply(&mut cfg.monitors)
}

/// The setup on trial: keep it, or go back, with the seconds left
/// before it goes back by itself.
fn trial(ui: &mut Ui, st: &mut MonitorsState, cfg: &mut Config, left: u64) -> bool {
    let m = ui.m;
    ui.space(m.px(14.0));
    let panel = ui.alloc(Vec2::new(FILL, m.px(150.0)));
    let accent = ui.theme.accent;
    let radius = m.px(15.0);
    ui.draw.rounded_rect(panel, radius, accent.lerp(look::CARD, 0.86));
    ui.draw.stroke_rect(panel, m.px(3.0), radius, accent);
    let pad = m.px(26.0);
    let (w, gap, h) = (m.px(BUTTON_W), m.px(14.0), m.px(BUTTON_H));
    let top = (panel.center().y - h * 0.5).round();
    let keep = Rect::from_min_size(Vec2::new(panel.max.x - pad - w, top), Vec2::new(w, h));
    let back = Rect::from_min_size(Vec2::new(keep.min.x - gap - w, top), Vec2::new(w, h));

    let (mut title, body) = (kit::title_style(ui), ui.text_style());
    title.size = m.px(30.0) as f32;
    let (title_h, body_h) = (f64::from(title.line_height()), f64::from(body.line_height()));
    let text_w = (back.min.x - pad - (panel.min.x + pad)).max(1.0);
    let y = (panel.center().y - (title_h + body_h) * 0.5).round();
    ui.text_in_rect("Keep this setup?", &title, Rect::from_min_size(Vec2::new(panel.min.x + pad, y), Vec2::new(text_w, title_h)), look::TEXT);
    let counting = if left == 1 { "Going back to how it was in 1 second.".to_owned() } else { format!("Going back to how it was in {left} seconds.") };
    ui.text_in_rect(&counting, &body, Rect::from_min_size(Vec2::new(panel.min.x + pad, y + title_h), Vec2::new(text_w, body_h)), look::TEXT_DIM);

    let (back_id, keep_id) = (ui.id("go-back"), ui.id("keep"));
    if controls::button(ui, back_id, back, "Go Back") {
        st.go_back();
    }
    controls::button_of(ui, keep_id, keep, "Keep", Kind::Primary, true) && st.keep(&mut cfg.monitors)
}

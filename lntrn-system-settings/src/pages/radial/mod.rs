//! The ring a right click on the desktop opens: which buttons are on it,
//! in what order, and what each is called, shows and runs. A button
//! picked (on the little ring, or by its row) opens up under its row to
//! be edited. The ring has a file of its own (`config::radial`) rather than a section of
//! `lantern.toml`, so the page keeps its own state: the file is written
//! a moment after the last change, and the desktop redraws its ring as
//! the file lands.

mod apps;
mod icons;
mod ring;
#[cfg(test)]
mod tests;

use lntrn_app::lntrn_render::{DrawList, Gpu, ImageHandle, Images};
use lntrn_math::{Color, Rect, Vec2};
use lntrn_ui::{CursorIcon, IconFn, Key, Sense, Ui};

use self::apps::App;
use self::icons::Icons;
use crate::config::radial::{Action, MAX_SLOTS, Ring, Slot};
use crate::kit::card::ROW_PAD;
use crate::kit::{self, Card, Keep, controls};
use crate::look;

/// Seconds after the last change before the file is written.
const SAVE_DELAY: f64 = 0.3;
/// How often, at most, the file is checked for outside changes.
const DISK_CHECK: f64 = 1.0;
/// A button's row in the list and the picture on it, in logical pixels.
const ROW_H: f64 = 84.0;
const ICON: f64 = 48.0;
/// The search: its field and the button that lists everything.
const SEARCH_W: f64 = 440.0;

/// The buttons that are not an app: what the search calls each, the
/// icon it starts with, and what it does.
const ACTIONS: [(&str, &str, Action); 2] = [("New Folder", "folders/Standard/lntrn-folder-desktop.svg", Action::NewFolder), ("Refresh Desktop", "spark-menu-restart.svg", Action::Refresh)];

/// What a button that runs nothing does, in a line.
fn does(action: Action) -> &'static str {
    match action {
        Action::Launch => "Runs its command.",
        Action::NewFolder => "Makes a folder where the ring was opened.",
        Action::Refresh => "Looks at the desktop's folder again.",
    }
}

#[derive(Default)]
pub struct RadialState {
    /// The file, read when the page first shows.
    ring: Option<Ring>,
    /// The button opened up for editing, when one is.
    selected: Option<usize>,
    icons: Icons,
    /// What is installed, listed again each time the search opens.
    apps: Vec<App>,
    query: String,
    search_had_focus: bool,
    /// Counts the changes to which buttons there are and where: the
    /// edit fields are new ones after each, so one button's undo never
    /// reaches into another.
    generation: u64,
    /// When the last change not yet written was made.
    dirty_since: Option<f64>,
    last_disk_check: f64,
}

impl RadialState {
    /// Read the file again, dropping unsaved edits.
    pub fn reload(&mut self) {
        if self.ring.is_some() {
            self.ring = Some(Ring::load());
            self.generation += 1;
            self.dirty_since = None;
        }
    }

    /// Write a pending change once the user has paused, and pick up
    /// edits made to the file by someone else while we sit clean.
    /// Returns how soon to be called again, while a write is waiting.
    pub fn housekeeping(&mut self, now: f64) -> Result<Option<f64>, String> {
        let Some(ring) = &mut self.ring else { return Ok(None) };
        if let Some(since) = self.dirty_since {
            let waited = now - since;
            if waited < SAVE_DELAY {
                return Ok(Some(SAVE_DELAY - waited));
            }
            self.dirty_since = None;
            return ring.save().map(|()| None);
        }
        if now - self.last_disk_check >= DISK_CHECK {
            self.last_disk_check = now;
            if ring.changed_on_disk() {
                *ring = Ring::load();
                self.generation += 1;
            }
        }
        Ok(None)
    }

    /// Hand the buttons' pictures to the GPU. Returns `true` when any
    /// went up, so the frame is rebuilt with them showing.
    pub fn upload(&mut self, gpu: &Gpu, images: &mut Images) -> bool {
        let Some(ring) = &self.ring else { return false };
        self.icons.upload(gpu, images, |name| ring.slots.iter().any(|s| s.icon == name))
    }
}

/// What a click on the page asks for.
enum Edit {
    /// Open this button up (from the little ring).
    Select(usize),
    /// Open it up, or close it when it is the open one (from its row).
    Toggle(usize),
    /// Swap with the button before it, round the ring.
    Earlier(usize),
    Later(usize),
    Remove(usize),
    Add(Slot),
}

pub fn draw(st: &mut RadialState, ui: &mut Ui) {
    let now = ui.now();
    let RadialState { ring, selected, icons, apps, query, search_had_focus, generation, dirty_since, .. } = st;
    let ring = ring.get_or_insert_with(Ring::load);
    let count = ring.slots.len();
    *selected = selected.filter(|i| *i < count);
    let mut edit = None;
    let mut changed = false;

    if let Some(why) = ring.unreadable.clone() {
        let text = format!("desktop-radial.json could not be read ({why}), so this is the ring the desktop falls back to. The next change here writes a good file over it.");
        if kit::bits::banner(ui, "unreadable", &text, look::WARN) {
            ring.unreadable = None;
        }
    }

    kit::caption(ui, "Ring");
    kit::card(ui, "ring", |c| {
        let buttons: Vec<ring::Button> = ring.slots.iter().map(|s| ring::Button { image: icons.get(&s.icon), label: &s.label, live: s.live() }).collect();
        let h = ring::height(c.ui.m);
        if let Some(i) = c.block(h, |ui, rect| ring::draw(ui, rect, &buttons, *selected)) {
            edit = Some(Edit::Select(i));
        }
    });
    kit::note(ui, "Clockwise from the top, the way it opens on the desktop. The button in the middle opens the Command Center.");

    kit::caption(ui, &format!("Buttons: {count} of {MAX_SLOTS}"));
    let fields = generation.to_string();
    kit::card(ui, "buttons", |c| {
        for (i, slot) in ring.slots.iter_mut().enumerate() {
            let image = icons.get(&slot.icon);
            let h = c.ui.m.px(ROW_H);
            c.ui.push_index(i);
            let clicked = c.block(h, |ui, rect| slot_row(ui, rect, slot, image, i, count, *selected == Some(i)));
            if *selected == Some(i) {
                c.ui.push_id(&fields);
                changed |= edit_rows(c, slot);
                c.ui.pop_id();
            }
            c.ui.pop_id();
            if clicked.is_some() {
                edit = clicked;
            }
        }
        if count >= MAX_SLOTS {
            c.value("The ring is full", "Eight are as many as fit round it. Take one off to add another.", "");
        } else if let Some(slot) = add_row(c, apps, query, search_had_focus) {
            edit = Some(Edit::Add(slot));
        }
    });

    kit::note(ui, "Click a button to open it up: its name, what it runs and its icon can all be changed.");

    if let Some(edit) = edit {
        let moved = !matches!(edit, Edit::Select(_) | Edit::Toggle(_));
        match edit {
            Edit::Select(i) => *selected = Some(i),
            Edit::Toggle(i) => *selected = (*selected != Some(i)).then_some(i),
            Edit::Earlier(i) | Edit::Later(i) => {
                let to = if matches!(edit, Edit::Earlier(_)) { (i + count - 1) % count } else { (i + 1) % count };
                ring.slots.swap(i, to);
                // The open one stays open, wherever the swap put it.
                *selected = selected.map(|s| if s == i { to } else if s == to { i } else { s });
            }
            Edit::Remove(i) => {
                ring.slots.remove(i);
                *selected = selected.filter(|s| *s != i).map(|s| if s > i { s - 1 } else { s });
            }
            // Open, so what the app gave it can be changed at once.
            Edit::Add(slot) => {
                ring.slots.push(slot);
                *selected = Some(count);
            }
        }
        if moved {
            *generation += 1;
            changed = true;
        }
        ui.state.request_rebuild = true;
    }
    if changed {
        *dirty_since = Some(now);
    }
}

/// The rows a button opens up into: what it is called, what it runs
/// and its icon. `true` when one changed.
fn edit_rows(c: &mut Card, slot: &mut Slot) -> bool {
    let mut changed = c.text("Name", "What the pill under it says.", &mut slot.label, "Terminal").changed;
    match slot.action {
        Action::Launch => changed |= c.text("Command", "The program, then what it is handed. Quotes keep words together.", &mut slot.command, "lntrn-terminal").changed,
        other => c.value("Does", "", does(other)),
    }
    changed | c.text("Icon", "An icon's name, or the path of a picture.", &mut slot.icon, "firefox").changed
}

/// One button's row: its picture, its name over what it runs, then the
/// buttons that move it round the ring and take it off. The picture and
/// the words are the click target that opens it up for editing.
fn slot_row(ui: &mut Ui, rect: Rect, slot: &Slot, image: Option<ImageHandle>, index: usize, count: usize, picked: bool) -> Option<Edit> {
    let m = ui.m;
    let accent = ui.theme.accent;
    let (pad, gap, side) = (m.px(ROW_PAD), m.px(10.0), m.px(controls::BUTTON_H));
    // The last button can't leave (an empty ring is the defaults on the
    // desktop), and alone it has nowhere to move to.
    let tools_w = if count > 1 { side * 3.0 + gap * 2.0 } else { 0.0 };
    let tools_x = rect.max.x - pad - tools_w;

    let body = Rect::new(rect.min, Vec2::new(tools_x - gap, rect.max.y));
    let id = ui.id("pick");
    let mut r = ui.interact(id, body, Sense::CLICK);
    ui.focusable(id, body);
    ui.key_click(id, &mut r);
    if r.hovered {
        ui.state.cursor_icon = CursorIcon::Pointer;
    }
    let mut out = r.clicked.then_some(Edit::Toggle(index));

    let lit = Rect::new(Vec2::new(rect.min.x + m.px(8.0), rect.min.y), Vec2::new(rect.max.x - m.px(8.0), rect.max.y));
    if picked {
        ui.draw.rounded_rect(lit, m.px(10.0), accent.fade(0.16));
    } else if r.hovered {
        ui.draw.rounded_rect(lit, m.px(10.0), Color::WHITE.fade(0.04));
    }

    let icon = Rect::from_center_size(Vec2::new(rect.min.x + pad + m.px(ICON) * 0.5, rect.center().y), Vec2::splat(m.px(ICON))).round();
    match image {
        Some(image) => {
            let fit = icon.width() / image.width.max(image.height).max(1) as f64;
            let size = Vec2::new(image.width as f64 * fit, image.height as f64 * fit);
            ui.draw.image(Rect::from_center_size(icon.center(), size).round(), image, 0.0, Color::WHITE);
        }
        None => {
            ui.draw.circle(icon.center(), icon.width() * 0.5, look::BUTTON);
            ui.draw.ring(icon.center(), icon.width() * 0.5, m.px(2.0), look::TRACK);
        }
    }

    let style = ui.text_style();
    // A command is set in the monospace; what a desktop action does, in words.
    let (what, under, ink) = match slot.action {
        Action::Launch if !slot.live() => ("Nothing to run yet, so the desktop leaves it off.", kit::small_style(ui), look::WARN),
        Action::Launch => (slot.command.as_str(), kit::mono_style(ui), look::TEXT_DIM),
        other => (does(other), kit::small_style(ui), look::TEXT_DIM),
    };
    let (label_h, under_h) = (style.line_height() as f64, under.line_height() as f64);
    let text_x = icon.max.x + m.px(18.0);
    let text_w = (body.max.x - text_x).max(1.0);
    let top = (rect.center().y - (label_h + under_h) * 0.5).round();
    let (name, name_ink) = if slot.label.trim().is_empty() { ("No name", look::TEXT_DIM) } else { (slot.label.as_str(), look::TEXT) };
    let shown = kit::elide(ui, name, &style, text_w, Keep::Start);
    ui.text_in_rect(&shown, &style, Rect::from_min_size(Vec2::new(text_x, top), Vec2::new(text_w, label_h)), name_ink);
    let shown = kit::elide(ui, what, &under, text_w, Keep::Start);
    ui.text_in_rect(&shown, &under, Rect::from_min_size(Vec2::new(text_x, top + label_h), Vec2::new(text_w, under_h)), ink);
    ui.focus_ring(id, body);

    if count > 1 {
        let y = (rect.center().y - side * 0.5).round();
        let tools: [(&str, IconFn, &str, Edit); 3] = [("earlier", chevron_up, "Move it earlier round the ring", Edit::Earlier(index)), ("later", chevron_down, "Move it later round the ring", Edit::Later(index)), ("remove", cross, "Take it off the ring", Edit::Remove(index))];
        for (n, (name, glyph, tip, edit)) in tools.into_iter().enumerate() {
            let at = Rect::from_min_size(Vec2::new(tools_x + n as f64 * (side + gap), y), Vec2::splat(side));
            let tool = ui.id(name);
            if controls::glyph_button(ui, tool, at, glyph, tip) {
                out = Some(edit);
            }
        }
    }
    out
}

/// The row that adds a button: a search of what is installed, narrowed
/// as it is typed in, with a button that lists everything. Returns the
/// button a pick makes: an app's name, icon and command, ready to edit.
fn add_row(c: &mut Card, apps: &mut Vec<App>, query: &mut String, had_focus: &mut bool) -> Option<Slot> {
    let id = c.ui.id("add");
    let w = c.ui.m.px(SEARCH_W);
    c.row("Add an app", "Search what is installed here.", w, |ui, slot| {
        let m = ui.m;
        let h = m.px(controls::BUTTON_H);
        let rect = Rect::from_min_size(Vec2::new(slot.min.x, (slot.center().y - h * 0.5).round()), Vec2::new(slot.width(), h));
        let field = Rect::new(rect.min, Vec2::new(rect.max.x - h - m.px(8.0), rect.max.y));
        let button = Rect::new(Vec2::new(rect.max.x - h, rect.min.y), rect.max);

        // Down opens the list; the field would otherwise swallow the key.
        let down = ui.state.has_focus(id) && ui.state.take_key(|k| k.key == Key::ArrowDown && k.mods.is_empty()).is_some();
        let typed = controls::text_field(ui, id, field, query, "Search apps");
        let arrived = typed.focused && !*had_focus;
        *had_focus = typed.focused;
        let was_open = *ui.state.open(id);
        let mut open = was_open;
        if controls::glyph_button(ui, id.with("list"), button, chevron_down, "Everything installed") {
            open = !was_open;
        }
        if down || arrived || typed.changed {
            open = true;
        }
        if open && !was_open {
            *apps = apps::installed();
            ui.state.request_rebuild = true;
        }

        let mut picked = None;
        if open {
            let needle = query.trim().to_lowercase();
            let wanted = |name: &str| needle.is_empty() || name.to_lowercase().contains(&needle);
            let found: Vec<Slot> = apps.iter().filter(|a| wanted(&a.name)).map(|a| Slot::launch(&a.name, &a.icon, &a.command)).chain(ACTIONS.iter().filter(|a| wanted(a.0)).map(|(name, icon, action)| Slot { label: (*name).to_owned(), icon: (*icon).to_owned(), action: *action, command: String::new() })).collect();
            if typed.committed {
                // Enter takes the first of what is left.
                picked = found.first().cloned();
            } else if !found.is_empty() {
                // Two apps of one name are told apart by what they run.
                let twin = |s: &Slot| found.iter().filter(|o| o.label == s.label).count() > 1;
                let shown: Vec<String> = found.iter().map(|s| if twin(s) { format!("{}  ·  {}", s.label, brief(&s.command)) } else { s.label.clone() }).collect();
                let names: Vec<&str> = shown.iter().map(String::as_str).collect();
                let res = ui.popup_list(id, rect, &names, None);
                picked = res.picked.map(|i| found[i].clone());
                open = !res.closed;
            }
        }
        if picked.is_some() {
            open = false;
            query.clear();
            let caret = ui.state.text_edit(id);
            (caret.cursor, caret.anchor) = (0, 0);
        }
        *ui.state.open(id) = open;
        picked
    })
}

/// A command cut to what tells it from another in a list.
fn brief(command: &str) -> String {
    const MOST: usize = 40;
    if command.chars().count() <= MOST { command.to_owned() } else { command.chars().take(MOST - 1).chain(['…']).collect() }
}

fn chevron(d: &mut DrawList, rect: Rect, color: Color, w: f64, tip: f64) {
    let (c, s) = (rect.center(), rect.width().min(rect.height()) * 0.5);
    d.polyline(&[c + Vec2::new(-0.8 * s, -0.4 * s * tip), c + Vec2::new(0.0, 0.4 * s * tip), c + Vec2::new(0.8 * s, -0.4 * s * tip)], w, color, false);
}

fn chevron_up(d: &mut DrawList, rect: Rect, color: Color, w: f64) {
    chevron(d, rect, color, w, -1.0);
}

fn chevron_down(d: &mut DrawList, rect: Rect, color: Color, w: f64) {
    chevron(d, rect, color, w, 1.0);
}

fn cross(d: &mut DrawList, rect: Rect, color: Color, w: f64) {
    let (c, s) = (rect.center(), rect.width().min(rect.height()) * 0.35);
    d.line(c - Vec2::splat(s), c + Vec2::splat(s), w, color);
    d.line(c + Vec2::new(-s, s), c + Vec2::new(s, -s), w, color);
}

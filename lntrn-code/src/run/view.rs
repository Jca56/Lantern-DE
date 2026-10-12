//! Run on screen: the button at the right end of the status bar.

use lntrn_math::{Rect, Vec2};
use lntrn_text::TextStyle;
use lntrn_ui::{CursorIcon, Sense, Ui};

use super::Ask;
use crate::app::App;

/// The most characters of a file's or a crate's name the button shows.
const NAME_CAP: usize = 28;

impl App {
    /// The Run button, right-aligned in the status bar's `space`: a
    /// triangle and what would run, or a square and Stop while the last
    /// run is going. A click runs or stops; a right click asks for the
    /// Run menu. Returns the width it took.
    pub(crate) fn draw_run_button(&mut self, ui: &mut Ui, space: Rect) -> f64 {
        let running = self.run_active();
        let plan = self.run_plan(ui.state.now);
        let (m, theme) = (ui.m, ui.theme);
        let label = match (running, plan.as_ref().map(|p| p.label.as_str())) {
            (true, _) => "Stop".to_owned(),
            (false, Some(name)) if name.chars().count() > NAME_CAP => format!("Run {}…", name.chars().take(NAME_CAP).collect::<String>()),
            (false, Some(name)) => format!("Run {name}"),
            (false, None) => "Run".to_owned(),
        };
        // The bar's own text size, heavier: this one is for pressing.
        let style = TextStyle::new((m.text_size * 0.85).round().max(10.0)).bold();
        // As tall as the bar lets it be: the bigger target.
        let inset = m.px(1.0);
        let icon = ((space.height() - inset * 2.0) * 0.5).round().max(4.0);
        let w = m.pad + icon + m.gap + ui.measure(&label, &style) + m.pad;
        // Clear of the window's corner, which is grabbed to resize it.
        let right = space.max.x - m.pad;
        let face = Rect::new(Vec2::new((right - w).max(space.min.x), space.min.y + inset), Vec2::new(right, (space.max.y - inset).max(space.min.y + inset)));
        let resp = ui.interact(ui.id("run"), face, Sense::CLICK);
        if resp.hovered {
            ui.state.cursor_icon = CursorIcon::Pointer;
        }
        ui.button_face(face, &resp);
        let c = Vec2::new(face.min.x + m.pad + icon * 0.5, face.center().y);
        let half = icon * 0.5;
        if running {
            ui.draw.rounded_rect(Rect::from_center_size(c, Vec2::splat(icon * 0.9)), m.px(2.0), theme.close);
        } else {
            let ink = if plan.is_some() { theme.accent } else { theme.text_dim };
            ui.draw.triangle(Vec2::new(c.x - half * 0.8, c.y - half), Vec2::new(c.x - half * 0.8, c.y + half), Vec2::new(c.x + half, c.y), ink);
        }
        let text = Rect::new(Vec2::new(face.min.x + m.pad + icon + m.gap, face.min.y), Vec2::new(face.max.x - m.pad * 0.5, face.max.y));
        ui.text_in_rect(&label, &style, text, if running || plan.is_some() { theme.text } else { theme.text_dim });
        let tip = match (&plan, running, &self.run.last) {
            (_, true, Some(last)) => format!("Stop `{}` (Shift+F5)", last.shown),
            (Some(p), _, _) => format!("`{}` in {} (F5). Right-click for more.", p.shown, p.cwd.display()),
            _ => "Nothing here to run. Right-click to set a command.".to_owned(),
        };
        ui.tooltip(&resp, &tip);
        if resp.clicked {
            self.run.pending = Some(if running { Ask::Stop } else { Ask::Start });
            ui.state.request_rebuild = true;
        }
        let at = ui.state.right_press_pos;
        if ui.state.right_pressed && face.contains(at) && !ui.state.shielded(ui.layer(), at) {
            self.run.menu_at = Some(at);
            ui.state.request_rebuild = true;
        }
        space.max.x - face.min.x + m.pad
    }
}

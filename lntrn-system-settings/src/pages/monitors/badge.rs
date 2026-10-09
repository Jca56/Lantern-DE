//! Identify: for a few seconds every monitor wears its number, so the
//! tiles in the layout card can be told apart on the desk. Each number
//! is a small layer surface of its own, put on its monitor by name
//! (Lantern UI's U086) and drawn by an editor that is nothing else:
//! [`Editor::Badge`](crate::app::Editor) in a window of one area.

use std::time::{Duration, Instant};

use lntrn_math::{Rect, Vec2};
use lntrn_ui::{AreaCx, Layer, LayerConfig, NewWindow, ShellRequest, Ui};

use crate::kit;
use crate::look;

/// How long the numbers stay up.
const SHOW: Duration = Duration::from_secs(3);
/// A badge's size, logical: read from across the room.
const SIZE: (u32, u32) = (420, 380);
/// The most monitors that get a badge: one editor id each.
pub const MAX: usize = 16;

/// The name a badge's editor goes by in a window's layout.
pub fn editor_id(index: u8) -> String {
    format!("badge-{index}")
}

/// Which badge an editor id names.
pub fn index_of(id: &str) -> Option<u8> {
    id.strip_prefix("badge-")?.parse().ok().filter(|i| usize::from(*i) < MAX)
}

#[derive(Default)]
pub struct Identify {
    /// When the badges come down. A window's clock is its own, so this
    /// is the wall's.
    until: Option<Instant>,
    /// What each badge says under its number, by its index.
    names: Vec<String>,
}

impl Identify {
    fn showing(&self) -> bool {
        self.until.is_some_and(|t| Instant::now() < t)
    }

    /// Put a badge on each monitor of `on` (its index among all of them,
    /// and its name). Asked again while they are up, they stay up longer.
    pub fn show(&mut self, on: &[(usize, String)], cx: &mut AreaCx<()>) {
        let already = self.showing();
        self.until = Some(Instant::now() + SHOW);
        if already {
            return;
        }
        self.names.clear();
        for (index, name) in on.iter().filter(|(i, _)| *i < MAX) {
            if self.names.len() <= *index {
                self.names.resize(index + 1, String::new());
            }
            self.names[*index] = name.clone();
            let layer = LayerConfig::new("lntrn-identify", SIZE).layer(Layer::Overlay).output(name.as_str());
            cx.request(ShellRequest::OpenWindow(NewWindow::single(format!("Monitor {}", index + 1), &editor_id(*index as u8)).layer(layer)));
        }
    }

    /// A badge's whole surface: the number, the monitor's name under it,
    /// on a solid plate ringed in the accent. It closes itself when the
    /// time is up.
    pub fn draw(&self, index: u8, ui: &mut Ui, cx: &mut AreaCx<()>) {
        let left = self.until.map_or(Duration::ZERO, |t| t.saturating_duration_since(Instant::now()));
        if left.is_zero() {
            cx.request(ShellRequest::CloseWindow);
            return;
        }
        ui.state.request_redraw_after(left.as_secs_f64() + 0.02);
        let m = ui.m;
        // The whole surface, past the padding a body is usually given.
        let plate = ui.clip();
        let accent = ui.theme.accent;
        // Solid whatever the windows' transparency is: it has to be read.
        ui.draw.rect(plate, look::BG);
        ui.draw.stroke_rect(plate.shrink(m.px(5.0)), m.px(8.0), m.px(22.0), accent);
        let mut big = kit::title_style(ui);
        big.size = (plate.height() * 0.55) as f32;
        let mut name = kit::title_style(ui);
        name.size = m.px(34.0) as f32;
        let name_h = f64::from(name.line_height());
        let number_h = f64::from(big.line_height());
        let top = (plate.center().y - (number_h + name_h) * 0.5).round();
        ui.text_centered(&(u32::from(index) + 1).to_string(), &big, Rect::from_min_size(Vec2::new(plate.min.x, top), Vec2::new(plate.width(), number_h)), accent);
        let said = self.names.get(usize::from(index)).map_or("", String::as_str);
        ui.text_centered(said, &name, Rect::from_min_size(Vec2::new(plate.min.x, top + number_h), Vec2::new(plate.width(), name_h)), look::TEXT);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_badge_is_found_by_its_editor_id() {
        assert_eq!(editor_id(0), "badge-0");
        assert_eq!((index_of("badge-0"), index_of("badge-7")), (Some(0), Some(7)));
        assert_eq!(index_of(&editor_id(MAX as u8 - 1)), Some(MAX as u8 - 1));
        // Not a badge, not a number, or more than there are.
        assert_eq!((index_of("Settings"), index_of("badge-x"), index_of("badge-16"), index_of("badge-999")), (None, None, None, None));
    }
}

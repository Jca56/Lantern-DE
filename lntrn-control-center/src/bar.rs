//! The bar: where it sits on the screen and what it is painted with. It
//! floats, like Plasma 6's panel: a plate held `gap` away from the bottom
//! edge of its monitor (the same gap snapped windows keep) and a little
//! more from the left and right, with rounded corners. The surface is
//! exactly the plate, so nothing invisible around it catches a click.

use lntrn_kit::desktop::{Desktop, Style};
use lntrn_kit::look;
use lntrn_math::Color;
use lntrn_ui::{Anchor, KeyboardMode, LayerConfig, Margin, Theme, Ui};

/// What the compositor knows the surface as.
pub const NAMESPACE: &str = "lntrn-control-center";
/// The plate's height, logical px: room for 18px text and big targets.
pub const HEIGHT: u32 = 64;
/// The plate's corner radius, logical px.
const RADIUS: f64 = 16.0;
/// How much further than `gap` the plate keeps from the side edges.
const SIDE_EXTRA: i32 = 8;
/// The Fox style's plate: the Command Center's panel (Fox Dark's `bg`).
const FOX_PLATE: Color = Color::hex(0x181818);

/// The layer surface of a bar floating `gap` px off the bottom edge (and
/// [`SIDE_EXTRA`] more off the sides), on the monitor called `output`
/// (the compositor's pick when `None`).
pub fn layer(gap: i32, output: Option<&str>) -> LayerConfig {
    // The zone is the plate alone: the compositor adds the bottom margin
    // to it, so windows stop at the plate's top edge.
    let cfg = LayerConfig::bar(NAMESPACE, Anchor::BOTTOM, HEIGHT)
        .margin(Margin { top: 0, right: gap + SIDE_EXTRA, bottom: gap, left: gap + SIDE_EXTRA })
        // A click on the bar leaves the keyboard with the window it was in.
        .keyboard(KeyboardMode::None);
    match output {
        Some(name) => cfg.output(name),
        None => cfg,
    }
}

/// The monitor the bar goes on: the primary, by name, when it is plugged
/// in. Otherwise `None`, and the compositor puts it where it puts things:
/// a named monitor that isn't there would get no bar at all (U086).
pub fn home(primary: Option<&str>) -> Option<String> {
    primary.filter(|name| connected(name)).map(str::to_owned)
}

/// Whether the kernel sees a monitor on the connector the compositor
/// calls `name`: `/sys/class/drm/card1-DP-1/status` says `connected`.
fn connected(name: &str) -> bool {
    let Ok(dir) = std::fs::read_dir("/sys/class/drm") else { return false };
    dir.flatten().any(|entry| {
        let file = entry.file_name();
        let ours = file.to_str().and_then(|f| f.strip_prefix("card")).and_then(|f| f.split_once('-')).is_some_and(|(_, connector)| connector == name);
        ours && std::fs::read_to_string(entry.path().join("status")).is_ok_and(|s| s.trim() == "connected")
    })
}

/// The shell's theme: the kit's, with the desktop's accent, but nothing
/// of the shell's own painted behind or around the plate. The clear stays
/// see-through past the rounded corners, and the outline the shell rings
/// its one area with is gone: the bar draws its own edge.
pub fn theme(desktop: &Desktop) -> Theme {
    Theme { bg: Color::TRANSPARENT, border_width: 0.0, ..look::theme(desktop.accent) }
}

/// What the plate is painted with.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Look {
    pub plate: Color,
    /// A hairline round the plate, so it reads against a dark wallpaper.
    pub edge: Color,
}

impl Look {
    /// The window background the desktop has, chosen or its style's own.
    pub fn of(desktop: &Desktop) -> Look {
        let plate = desktop.background.unwrap_or(match desktop.style {
            Style::Lantern => look::BG,
            Style::Fox => FOX_PLATE,
        });
        Look { plate, edge: plate.lerp(Color::WHITE, 0.10) }
    }
}

/// Paint the plate over the whole surface.
pub fn draw(ui: &mut Ui, look: Look, gap: i32) {
    let plate = ui.clip();
    // Flush with the screen's edges, round corners would look chipped.
    let radius = if gap > 0 { ui.m.px(RADIUS) } else { 0.0 };
    let edge = ui.m.px(1.0);
    ui.draw.rounded_rect(plate, radius, look.plate);
    ui.draw.stroke_rect(plate, edge, radius, look.edge);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bar_floats_and_reserves_its_own_height() {
        let cfg = layer(8, Some("DP-1"));
        assert_eq!((cfg.namespace.as_str(), cfg.anchor, cfg.size), (NAMESPACE, Anchor::BOTTOM, (0, HEIGHT)));
        assert_eq!(cfg.margin, Margin { top: 0, right: 16, bottom: 8, left: 16 });
        assert_eq!(cfg.exclusive_zone, HEIGHT as i32, "the compositor adds the bottom margin itself");
        assert_eq!(cfg.keyboard, KeyboardMode::None);
        assert_eq!(cfg.output.as_deref(), Some("DP-1"));
        assert!(cfg.check().is_ok());
        assert_eq!(layer(0, None).output, None);
    }

    #[test]
    fn a_monitor_that_isnt_there_is_left_to_the_compositor() {
        assert_eq!(home(Some("NOT-A-CONNECTOR-9")), None);
        assert_eq!(home(None), None);
    }

    #[test]
    fn the_plate_wears_the_chosen_background_else_the_styles() {
        let chosen = Desktop { background: Some(Color::hex(0x0E0E0E)), ..Desktop::default() };
        assert_eq!(Look::of(&chosen).plate, Color::hex(0x0E0E0E));
        assert_eq!(Look::of(&Desktop { style: Style::Fox, ..Desktop::default() }).plate, FOX_PLATE);
        assert_eq!(Look::of(&Desktop::default()).plate, look::BG);
    }

    #[test]
    fn nothing_of_the_shells_is_painted_round_the_plate() {
        let t = theme(&Desktop::default());
        assert_eq!((t.bg.a, t.border_width), (0.0, 0.0));
        assert_eq!(t.accent, look::GOLD);
    }
}

//! The bar: where it sits on the screen and what it is painted with. It
//! floats, like Plasma 6's panel: a plate held `gap` away from the bottom
//! edge of its monitor (the same gap snapped windows keep) and a little
//! more from the left and right, with rounded corners. While a window
//! fills the monitor it docks, like a taskbar: the full width, flush with
//! the bottom, square. The surface holds both places, so the plate glides
//! between them inside it (`morph`); what the plate doesn't cover lets
//! clicks through (U092), and the space kept from windows changes once a
//! glide, not every frame.

use lntrn_kit::desktop::{Desktop, Style};
use lntrn_kit::look;
use lntrn_math::{Color, Rect, Vec2};
use lntrn_ui::{Anchor, KeyboardMode, LayerConfig, Metrics, Theme, Ui};

/// What the compositor knows the surface as.
pub const NAMESPACE: &str = "lntrn-control-center";
/// The plate's height, logical px: room for 18px text and big targets.
pub const HEIGHT: u32 = 64;
/// The floating plate's corner radius, logical px.
const RADIUS: f64 = 16.0;
/// How much further than `gap` the floating plate keeps from the sides.
const SIDE_EXTRA: i32 = 8;
/// The Fox style's plate: the Command Center's panel (Fox Dark's `bg`).
const FOX_PLATE: Color = Color::hex(0x181818);

/// The layer surface: the bottom `HEIGHT + gap` of the monitor called
/// `output` (the compositor's pick when `None`), across its whole width,
/// keeping windows above the plate whether it floats or is `docked`.
pub fn layer(gap: i32, docked: bool, output: Option<&str>) -> LayerConfig {
    let gap = gap.max(0) as u32;
    // Floating, the gap under the plate is kept from windows too.
    let zone = if docked { HEIGHT } else { HEIGHT + gap };
    let cfg = LayerConfig::bar(NAMESPACE, Anchor::BOTTOM, HEIGHT + gap)
        .exclusive(zone as i32)
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
/// see-through past the plate, and the outline the shell rings its one
/// area with is gone: the bar draws its own edge.
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

/// Where the plate is, in the window's physical pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Plate {
    pub rect: Rect,
    pub radius: f64,
    /// 0 floating to 1 docked.
    pub t: f64,
}

/// The plate in a surface filling `surface`, at `t` of the way from
/// floating to docked.
pub fn plate(surface: Rect, m: Metrics, gap: i32, t: f64) -> Plate {
    let gap = f64::from(gap.max(0));
    let side = m.px(gap + f64::from(SIDE_EXTRA));
    // The surface's own pixels per logical one: its buffer is rounded to
    // whole pixels (94 for 72 logical at 1.3x), so this, not the scale, is
    // what puts an edge where the compositor puts the window above it.
    let logical = f64::from(HEIGHT) + gap;
    let per = if surface.height() > 0.0 { surface.height() / logical } else { m.scale };
    let below = (gap * per).round();
    let floating = Rect::new(Vec2::new(surface.min.x + side, surface.min.y), Vec2::new(surface.max.x - side, surface.max.y - below));
    // Docked, the top is rounded up the screen: over the window's last
    // part pixel rather than a hairline short of it.
    let docked = Rect::new(Vec2::new(surface.min.x, surface.min.y + (gap * per).floor()), surface.max);
    let at = |a: f64, b: f64| a + (b - a) * t;
    let rect = Rect::new(Vec2::new(at(floating.min.x, docked.min.x), at(floating.min.y, docked.min.y)), Vec2::new(at(floating.max.x, docked.max.x), at(floating.max.y, docked.max.y))).round();
    // Sitting on the screen's bottom edge, round corners would look chipped.
    let radius = if below > 0.0 { m.px(RADIUS) * (1.0 - t) } else { 0.0 };
    Plate { rect, radius, t }
}

/// Paint the plate.
pub fn draw(ui: &mut Ui, look: Look, plate: Plate) {
    let edge = ui.m.px(1.0);
    let Plate { rect, radius, t } = plate;
    ui.draw.rounded_rect(rect, radius, look.plate);
    // Floating, a hairline all round; docked, along the top alone, like a
    // taskbar's.
    if t < 1.0 {
        ui.draw.stroke_rect(rect, edge, radius, look.edge.fade(1.0 - t));
    }
    if t > 0.0 {
        ui.draw.rect(Rect::new(rect.min, Vec2::new(rect.max.x, rect.min.y + edge)), look.edge.fade(t));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lntrn_ui::Margin;

    fn r(x0: f64, y0: f64, x1: f64, y1: f64) -> Rect {
        Rect::new(Vec2::new(x0, y0), Vec2::new(x1, y1))
    }

    #[test]
    fn the_surface_holds_both_places_and_keeps_windows_above_the_plate() {
        let floating = layer(8, false, Some("DP-1"));
        assert_eq!((floating.namespace.as_str(), floating.anchor, floating.size), (NAMESPACE, Anchor::BOTTOM, (0, HEIGHT + 8)));
        assert_eq!(floating.margin, Margin::default());
        assert_eq!(floating.exclusive_zone, (HEIGHT + 8) as i32, "the gap under the plate too");
        assert_eq!(floating.keyboard, KeyboardMode::None);
        assert_eq!(floating.output.as_deref(), Some("DP-1"));
        assert!(floating.check().is_ok());
        let docked = layer(8, true, None);
        assert_eq!((docked.size, docked.exclusive_zone, docked.output), ((0, HEIGHT + 8), HEIGHT as i32, None));
    }

    #[test]
    fn the_plate_glides_from_inset_and_round_to_full_width_and_square() {
        let m = Theme::default().metrics(1.0);
        let surface = r(0.0, 0.0, 1000.0, 72.0);
        assert_eq!(plate(surface, m, 8, 0.0), Plate { rect: r(16.0, 0.0, 984.0, 64.0), radius: 16.0, t: 0.0 });
        assert_eq!(plate(surface, m, 8, 1.0), Plate { rect: r(0.0, 8.0, 1000.0, 72.0), radius: 0.0, t: 1.0 });
        assert_eq!(plate(surface, m, 8, 0.5).rect, r(8.0, 4.0, 992.0, 68.0));
        // No gap: on the bottom edge from the start, so square.
        assert_eq!(plate(r(0.0, 0.0, 1000.0, 64.0), m, 0, 0.0), Plate { rect: r(8.0, 0.0, 992.0, 64.0), radius: 0.0, t: 0.0 });
    }

    #[test]
    fn at_a_fractional_scale_the_docked_top_never_falls_short_of_the_window() {
        // 72 logical at 1.3x is a 94-pixel buffer: the window's bottom edge
        // lands 10.44 pixels down it.
        let m = Theme::default().metrics(1.3);
        let surface = r(0.0, 0.0, 3840.0, 94.0);
        assert_eq!(plate(surface, m, 8, 1.0).rect, r(0.0, 10.0, 3840.0, 94.0));
        assert_eq!(plate(surface, m, 8, 0.0).rect, r(21.0, 0.0, 3819.0, 84.0));
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

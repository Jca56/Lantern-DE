//! The bar: where it sits on the screen and what it is painted with. It
//! floats, like Plasma 6's panel: a plate held off the bottom, left and
//! right edges of its monitor, with rounded corners, its [`Shape`] what
//! System Settings sets in `[control_center]`. While a window
//! fills the monitor it docks, like a taskbar: the full width, flush with
//! the bottom, square. The surface holds both places, so the plate glides
//! between them inside it (`morph`); what the plate doesn't cover lets
//! clicks through (U092), and the space kept from windows changes once a
//! glide, not every frame.

use lntrn_kit::desktop::Desktop;
use lntrn_kit::look::Palette;
use lntrn_math::{Color, Rect, Vec2};
use lntrn_ui::{Anchor, KeyboardMode, LayerConfig, Metrics, Theme, Ui};

use crate::hide;

/// What the compositor knows the surface as.
pub const NAMESPACE: &str = "lntrn-control-center";
/// What the Control Center wears whatever the window style: Fox's
/// charcoal, and no window background the user chose either.
pub const PALETTE: Palette = Palette::FOX;

/// The bar's size and where it floats, logical px: `[control_center]`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Shape {
    /// The plate's height: room for 18px text and big targets.
    pub height: u32,
    /// The floating plate's corner radius; docked it is square.
    pub radius: u32,
    /// Between the floating plate and the bottom of the screen.
    pub below: u32,
    /// Between the floating plate and the left and right edges.
    pub sides: u32,
}

impl Shape {
    /// Each key's range, as System Settings' sliders offer it.
    pub const HEIGHTS: (u32, u32) = (40, 120);
    pub const RADII: (u32, u32) = (0, 32);
    pub const BELOW: (u32, u32) = (0, 40);
    pub const SIDES: (u32, u32) = (0, 120);

    /// Held to the ranges above.
    pub fn clamped(self) -> Shape {
        let hold = |v: u32, (lo, hi): (u32, u32)| v.clamp(lo, hi);
        Shape { height: hold(self.height, Shape::HEIGHTS), radius: hold(self.radius, Shape::RADII), below: hold(self.below, Shape::BELOW), sides: hold(self.sides, Shape::SIDES) }
    }
}

impl Default for Shape {
    fn default() -> Self {
        Shape { height: 64, radius: 16, below: 8, sides: 16 }
    }
}

/// The layer surface: the bottom `height + below` of the monitor called
/// `output` (the compositor's pick when `None`), across its whole width,
/// keeping windows above the plate whether it floats or is `docked`.
pub fn layer(shape: Shape, docked: bool, output: Option<&str>) -> LayerConfig {
    let cfg = LayerConfig::bar(NAMESPACE, Anchor::BOTTOM, shape.height + shape.below)
        .exclusive(zone(shape, docked) as i32)
        // A click on the bar leaves the keyboard with the window it was in.
        .keyboard(KeyboardMode::None);
    match output {
        Some(name) => cfg.output(name),
        None => cfg,
    }
}

/// [`layer`] as auto hide has it: the bar's full height, or only the strip
/// along the bottom edge while it is out of sight, keeping `zone` logical
/// px from windows.
pub fn layer_with(shape: Shape, docked: bool, output: Option<&str>, full: bool, zone: u32) -> LayerConfig {
    let cfg = layer(shape, docked, output).exclusive(zone as i32);
    if full { cfg } else { cfg.size((0, hide::STRIP)) }
}

/// The space the whole bar keeps from windows, logical px. Floating, the
/// gap under the plate is kept too.
pub fn zone(shape: Shape, docked: bool) -> u32 {
    if docked { shape.height } else { shape.height + shape.below }
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
    monitors().any(|connector| connector == name)
}

/// Whether the kernel sees any monitor plugged in.
pub fn any_monitor() -> bool {
    monitors().next().is_some()
}

/// The connectors (`DP-1`) the kernel sees a monitor on.
fn monitors() -> impl Iterator<Item = String> {
    std::fs::read_dir("/sys/class/drm").into_iter().flatten().flatten().filter_map(|entry| {
        let file = entry.file_name();
        let connector = file.to_str()?.strip_prefix("card")?.split_once('-')?.1.to_owned();
        std::fs::read_to_string(entry.path().join("status")).is_ok_and(|s| s.trim() == "connected").then_some(connector)
    })
}

/// The shell's theme: the kit's in [`PALETTE`] with the desktop's accent,
/// but nothing of the shell's own painted behind or around the plate. The
/// clear stays see-through past the plate, and the outline the shell rings
/// its one area with is gone: the bar draws its own edge.
pub fn theme(desktop: &Desktop) -> Theme {
    Theme { bg: Color::TRANSPARENT, border_width: 0.0, ..PALETTE.theme(desktop.accent) }
}

/// What the plate is painted with.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Look {
    pub plate: Color,
    /// A hairline round the plate, so it reads against a dark wallpaper.
    pub edge: Color,
    /// See-through as the windows are: the compositor blurs what's behind
    /// (and a panel casts no shadow, which would show through it).
    pub glass: bool,
}

impl Look {
    /// [`PALETTE`]'s window colour, as see-through as the windows.
    pub fn of(desktop: &Desktop) -> Look {
        let plate = PALETTE.bg;
        let glass = desktop.opacity < 0.99;
        let alpha = if glass { desktop.opacity } else { 1.0 };
        Look { plate: plate.with_alpha(alpha), edge: plate.lerp(Color::WHITE, 0.10).with_alpha(alpha.max(0.6)), glass }
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

impl Plate {
    /// Moved `dy` window pixels down: on its way out of sight.
    pub fn lowered(self, dy: f64) -> Plate {
        Plate { rect: self.rect.translate(Vec2::new(0.0, dy)), ..self }
    }
}

/// The surface's own pixels per logical one: its buffer is rounded to
/// whole pixels (94 for 72 logical at 1.3x), so this, not the scale, is
/// what puts an edge where the compositor puts the window above it.
pub fn per_logical(surface: Rect, m: Metrics, shape: Shape) -> f64 {
    let logical = f64::from(shape.height + shape.below);
    if surface.height() > 0.0 { surface.height() / logical } else { m.scale }
}

/// The plate in a surface filling `surface`, at `t` of the way from
/// floating to docked.
pub fn plate(surface: Rect, m: Metrics, shape: Shape, t: f64) -> Plate {
    let gap = f64::from(shape.below);
    let side = m.px(f64::from(shape.sides));
    let per = per_logical(surface, m, shape);
    let below = (gap * per).round();
    let floating = Rect::new(Vec2::new(surface.min.x + side, surface.min.y), Vec2::new(surface.max.x - side, surface.max.y - below));
    // Docked, the top is rounded up the screen: over the window's last
    // part pixel rather than a hairline short of it.
    let docked = Rect::new(Vec2::new(surface.min.x, surface.min.y + (gap * per).floor()), surface.max);
    let at = |a: f64, b: f64| a + (b - a) * t;
    let rect = Rect::new(Vec2::new(at(floating.min.x, docked.min.x), at(floating.min.y, docked.min.y)), Vec2::new(at(floating.max.x, docked.max.x), at(floating.max.y, docked.max.y))).round();
    // Sitting on the screen's bottom edge, round corners would look chipped.
    let radius = if below > 0.0 { m.px(f64::from(shape.radius)) * (1.0 - t) } else { 0.0 };
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
    use lntrn_kit::desktop::Style;
    use lntrn_kit::look;
    use lntrn_ui::Margin;

    fn r(x0: f64, y0: f64, x1: f64, y1: f64) -> Rect {
        Rect::new(Vec2::new(x0, y0), Vec2::new(x1, y1))
    }

    #[test]
    fn the_surface_holds_both_places_and_keeps_windows_above_the_plate() {
        let shape = Shape::default();
        let floating = layer(shape, false, Some("DP-1"));
        assert_eq!((floating.namespace.as_str(), floating.anchor, floating.size), (NAMESPACE, Anchor::BOTTOM, (0, 72)));
        assert_eq!(floating.margin, Margin::default());
        assert_eq!(floating.exclusive_zone, 72, "the gap under the plate too");
        assert_eq!(floating.keyboard, KeyboardMode::None);
        assert_eq!(floating.output.as_deref(), Some("DP-1"));
        assert!(floating.check().is_ok());
        let docked = layer(Shape { height: 90, below: 20, ..shape }, true, None);
        assert_eq!((docked.size, docked.exclusive_zone, docked.output), ((0, 110), 90, None));
        // Auto hide: all the way up it is the same surface; out of sight,
        // a strip that keeps nothing from windows.
        assert_eq!(layer_with(shape, false, Some("DP-1"), true, 72), floating);
        let hidden = layer_with(shape, true, None, false, 0);
        assert_eq!((hidden.size, hidden.exclusive_zone, hidden.anchor), ((0, hide::STRIP), 0, Anchor::BOTTOM));
        assert!(hidden.check().is_ok());
    }

    #[test]
    fn the_plate_glides_from_inset_and_round_to_full_width_and_square() {
        let m = Theme::default().metrics(1.0);
        let shape = Shape::default();
        let surface = r(0.0, 0.0, 1000.0, 72.0);
        assert_eq!(plate(surface, m, shape, 0.0), Plate { rect: r(16.0, 0.0, 984.0, 64.0), radius: 16.0, t: 0.0 });
        assert_eq!(plate(surface, m, shape, 1.0), Plate { rect: r(0.0, 8.0, 1000.0, 72.0), radius: 0.0, t: 1.0 });
        assert_eq!(plate(surface, m, shape, 0.5).rect, r(8.0, 4.0, 992.0, 68.0));
        // No gap below: on the bottom edge from the start, so square.
        let flush = Shape { below: 0, sides: 8, ..shape };
        assert_eq!(plate(r(0.0, 0.0, 1000.0, 64.0), m, flush, 0.0), Plate { rect: r(8.0, 0.0, 992.0, 64.0), radius: 0.0, t: 0.0 });
    }

    #[test]
    fn a_shape_is_held_to_what_settings_offers() {
        assert_eq!(Shape { height: 5, radius: 99, below: 1000, sides: 7 }.clamped(), Shape { height: 40, radius: 32, below: 40, sides: 7 });
        assert_eq!(Shape::default().clamped(), Shape::default());
    }

    #[test]
    fn at_a_fractional_scale_the_docked_top_never_falls_short_of_the_window() {
        // 72 logical at 1.3x is a 94-pixel buffer: the window's bottom edge
        // lands 10.44 pixels down it.
        let m = Theme::default().metrics(1.3);
        let surface = r(0.0, 0.0, 3840.0, 94.0);
        assert_eq!(plate(surface, m, Shape::default(), 1.0).rect, r(0.0, 10.0, 3840.0, 94.0));
        assert_eq!(plate(surface, m, Shape::default(), 0.0).rect, r(21.0, 0.0, 3819.0, 84.0));
    }

    #[test]
    fn a_monitor_that_isnt_there_is_left_to_the_compositor() {
        assert_eq!(home(Some("NOT-A-CONNECTOR-9")), None);
        assert_eq!(home(None), None);
    }

    #[test]
    fn the_plate_keeps_fox_whatever_the_style_and_background() {
        let chosen = Desktop { style: Style::Lantern, background: Some(Color::hex(0x0E0E0E)), ..Desktop::default() };
        assert_eq!(Look::of(&chosen).plate, Palette::FOX.bg);
        assert_eq!(Look::of(&Desktop::default()).plate, Palette::FOX.bg);
        assert_eq!(theme(&chosen).text, Palette::FOX.text);
    }

    #[test]
    fn nothing_of_the_shells_is_painted_round_the_plate() {
        let t = theme(&Desktop::default());
        assert_eq!((t.bg.a, t.border_width), (0.0, 0.0));
        assert_eq!(t.accent, look::GOLD);
    }
}

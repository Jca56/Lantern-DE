//! The tray's pictures: small line drawings on a 24-unit grid, white, so
//! they take any colour they are drawn in. Each is rendered by our own
//! SVG renderer at exactly the pixels it shows at (crisp at any scale,
//! where the draw list's lines have no anti-aliasing), asked for while a
//! frame is built and handed to the GPU after it, then drawn from the
//! next frame on.

use std::collections::HashMap;

use lntrn_app::lntrn_render::{Gpu, ImageHandle, Images};
use lntrn_math::{Color, Rect};
use lntrn_ui::Ui;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Glyph {
    /// WiFi's fan with this many of its four parts lit.
    Wifi(u8),
    WifiOff,
    /// A wired network.
    Wired,
    Bluetooth,
    /// A speaker with this many of its three waves.
    Speaker(u8),
    SpeakerMuted,
    Sun,
    Gamepad,
    Bolt,
    Lock,
    Headset,
    Phone,
    Mouse,
    Keyboard,
    Computer,
    /// A password shown as itself, and hidden as dots.
    Eye,
    EyeOff,
    Plus,
}

/// What unlit parts of a picture show at: there, but faint.
const UNLIT: f64 = 0.28;
/// An eye's outline, lid to lid.
const EYE: &str = "M2 12C5 6.5 8.5 5 12 5C15.5 5 19 6.5 22 12C19 17.5 15.5 19 12 19C8.5 19 5 17.5 2 12Z";

impl Glyph {
    /// The picture for a Bluetooth device's icon name (`audio-headset`,
    /// `input-mouse`…): BlueZ's, from the device's class.
    pub fn for_device(icon: &str) -> Glyph {
        match icon {
            i if i.starts_with("audio") => Glyph::Headset,
            "phone" => Glyph::Phone,
            "input-gaming" => Glyph::Gamepad,
            "input-mouse" | "input-tablet" => Glyph::Mouse,
            "input-keyboard" => Glyph::Keyboard,
            "computer" => Glyph::Computer,
            _ => Glyph::Bluetooth,
        }
    }

    fn body(self) -> String {
        let lit = |on: bool| if on { 1.0 } else { UNLIT };
        match self {
            Glyph::Wifi(bars) => {
                // Arcs round the dot below, a quarter turn each.
                let arc = |r: f64, on: bool| {
                    let (dx, dy) = (r * 0.7071, 19.5 - r * 0.7071);
                    format!(r#"<path opacity="{}" d="M{:.2} {dy:.2}A{r} {r} 0 0 1 {:.2} {dy:.2}"/>"#, lit(on), 12.0 - dx, 12.0 + dx)
                };
                format!(r##"<circle opacity="{}" cx="12" cy="19.5" r="1.7" fill="#fff" stroke="none"/>{}{}{}"##, lit(bars >= 1), arc(5.0, bars >= 2), arc(9.5, bars >= 3), arc(14.0, bars >= 4))
            }
            Glyph::WifiOff => format!(
                r##"<g opacity="{UNLIT}"><circle cx="12" cy="19.5" r="1.7" fill="#fff" stroke="none"/><path d="M8.46 15.96A5 5 0 0 1 15.54 15.96M5.28 12.78A9.5 9.5 0 0 1 18.72 12.78M2.1 9.6A14 14 0 0 1 21.9 9.6"/></g><path d="M3.5 3.5l17 17"/>"##
            ),
            Glyph::Wired => r#"<rect x="9" y="2.5" width="6" height="5" rx="1"/><rect x="2.5" y="16.5" width="6" height="5" rx="1"/><rect x="15.5" y="16.5" width="6" height="5" rx="1"/><path d="M12 7.5v4.5M5.5 16.5V12h13v4.5"/>"#.to_owned(),
            Glyph::Bluetooth => r#"<path d="M6.5 7l11 10L12 22V2l5.5 5L6.5 17"/>"#.to_owned(),
            Glyph::Speaker(waves) => {
                let wave = |r: f64, on: bool| {
                    let (x, dy) = (11.0 + r * 0.7071, r * 0.7071);
                    format!(r#"<path opacity="{}" d="M{x:.2} {:.2}A{r} {r} 0 0 1 {x:.2} {:.2}"/>"#, lit(on), 12.0 - dy, 12.0 + dy)
                };
                format!(r##"<path d="M3 9h3.5L11 5v14l-4.5-4H3z" fill="#fff"/>{}{}{}"##, wave(3.5, waves >= 1), wave(6.5, waves >= 2), wave(9.5, waves >= 3))
            }
            Glyph::SpeakerMuted => r##"<path d="M3 9h3.5L11 5v14l-4.5-4H3z" fill="#fff"/><path d="M15.5 9.5l5 5M20.5 9.5l-5 5"/>"##.to_owned(),
            Glyph::Sun => {
                let rays: String = (0..8)
                    .map(|i| {
                        let a = f64::from(i) * std::f64::consts::FRAC_PI_4;
                        let (s, c) = a.sin_cos();
                        format!("M{:.2} {:.2}L{:.2} {:.2}", 12.0 + c * 7.2, 12.0 + s * 7.2, 12.0 + c * 9.6, 12.0 + s * 9.6)
                    })
                    .collect();
                format!(r#"<circle cx="12" cy="12" r="4"/><path d="{rays}"/>"#)
            }
            Glyph::Gamepad => r##"<path d="M7.5 7h9a5 5 0 0 1 4.9 4l.9 5.2a2.8 2.8 0 0 1-5 2.1L15.5 16h-7l-1.8 2.3a2.8 2.8 0 0 1-5-2.1l.9-5.2A5 5 0 0 1 7.5 7z"/><path d="M7.5 10v4M5.5 12h4"/><circle cx="15.5" cy="11" r="1.1" fill="#fff" stroke="none"/><circle cx="17.8" cy="13.4" r="1.1" fill="#fff" stroke="none"/>"##.to_owned(),
            Glyph::Bolt => r##"<path d="M13.5 2L4.5 13.5h6.5L10 22l9.5-12h-7z" fill="#fff" stroke-width="1"/>"##.to_owned(),
            Glyph::Lock => r#"<rect x="5" y="11" width="14" height="10" rx="2"/><path d="M8 11V8a4 4 0 0 1 8 0v3"/>"#.to_owned(),
            Glyph::Headset => r#"<path d="M4 15v-3a8 8 0 0 1 16 0v3"/><rect x="2.5" y="14" width="5" height="7" rx="2"/><rect x="16.5" y="14" width="5" height="7" rx="2"/>"#.to_owned(),
            Glyph::Phone => r#"<rect x="6.5" y="2.5" width="11" height="19" rx="2.5"/><path d="M11 18.5h2"/>"#.to_owned(),
            Glyph::Mouse => r#"<rect x="6" y="3" width="12" height="18" rx="6"/><path d="M12 7v4"/>"#.to_owned(),
            Glyph::Keyboard => r#"<rect x="2.5" y="6" width="19" height="12" rx="2"/><path d="M6 10h1M9.5 10h1M13.5 10h1M17 10h1M8 14h8"/>"#.to_owned(),
            Glyph::Computer => r#"<rect x="3" y="4" width="18" height="12" rx="2"/><path d="M8 20h8M12 16v4"/>"#.to_owned(),
            Glyph::Eye => format!(r#"<path d="{EYE}"/><circle cx="12" cy="12" r="3"/>"#),
            Glyph::EyeOff => format!(r#"<g opacity="{UNLIT}"><path d="{EYE}"/><circle cx="12" cy="12" r="3"/></g><path d="M3.5 3.5l17 17"/>"#),
            Glyph::Plus => r#"<path d="M12 5v14M5 12h14"/>"#.to_owned(),
        }
    }

    /// The whole drawing: white lines, round ends, on the 24 grid.
    pub fn svg(self) -> String {
        format!(r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#fff" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">{}</svg>"##, self.body())
    }
}

/// Each picture by the pixels it is drawn at: on the GPU, or asked for.
#[derive(Default)]
pub struct Glyphs {
    ready: HashMap<(Glyph, u32), Option<ImageHandle>>,
    wanted: Vec<(Glyph, u32)>,
}

impl Glyphs {
    /// Draw `glyph` filling the square `rect` in `color`; from the next
    /// frame on, the first time it is asked at that size.
    pub fn draw(&mut self, ui: &mut Ui, glyph: Glyph, rect: Rect, color: Color) {
        let px = rect.width().round().max(4.0) as u32;
        match self.ready.get(&(glyph, px)) {
            Some(Some(handle)) => ui.draw.image(rect, *handle, 0.0, color),
            Some(None) => {}
            None => {
                if !self.wanted.contains(&(glyph, px)) {
                    self.wanted.push((glyph, px));
                }
            }
        }
    }

    /// Render what frames asked for and hand it to the GPU. Returns
    /// whether any went up, so the frame is built again with it.
    pub fn upload(&mut self, gpu: &Gpu, images: &mut Images) -> bool {
        let any = !self.wanted.is_empty();
        for (glyph, px) in std::mem::take(&mut self.wanted) {
            let handle = lntrn_svg::render(&glyph.svg(), px).map(|image| images.add(gpu, &image));
            self.ready.insert((glyph, px), handle);
        }
        any
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [Glyph; 23] = [
        Glyph::Wifi(0),
        Glyph::Wifi(1),
        Glyph::Wifi(2),
        Glyph::Wifi(3),
        Glyph::Wifi(4),
        Glyph::WifiOff,
        Glyph::Wired,
        Glyph::Bluetooth,
        Glyph::Speaker(0),
        Glyph::Speaker(3),
        Glyph::SpeakerMuted,
        Glyph::Sun,
        Glyph::Gamepad,
        Glyph::Bolt,
        Glyph::Lock,
        Glyph::Headset,
        Glyph::Phone,
        Glyph::Mouse,
        Glyph::Keyboard,
        Glyph::Computer,
        Glyph::Eye,
        Glyph::EyeOff,
        Glyph::Plus,
    ];

    #[test]
    fn every_picture_renders_and_draws_something() {
        for glyph in ALL {
            let image = lntrn_svg::render(&glyph.svg(), 36).unwrap_or_else(|| panic!("{glyph:?} renders"));
            assert_eq!((image.width, image.height), (36, 36));
            let inked = image.rgba.chunks(4).filter(|px| px[3] > 0).count();
            assert!(inked > 30, "{glyph:?} draws something ({inked} px)");
        }
    }

    #[test]
    fn more_bars_light_more_of_the_fan() {
        let ink = |g: Glyph| lntrn_svg::render(&g.svg(), 48).unwrap().rgba.chunks(4).map(|px| u64::from(px[3])).sum::<u64>();
        assert!(ink(Glyph::Wifi(1)) < ink(Glyph::Wifi(2)) && ink(Glyph::Wifi(3)) < ink(Glyph::Wifi(4)));
    }

    #[test]
    fn devices_get_the_picture_of_their_kind() {
        assert_eq!(Glyph::for_device("audio-headphones"), Glyph::Headset);
        assert_eq!(Glyph::for_device("input-gaming"), Glyph::Gamepad);
        assert_eq!(Glyph::for_device(""), Glyph::Bluetooth);
    }
}

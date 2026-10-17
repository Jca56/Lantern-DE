//! Blur behind layer surfaces: the rects a surface asked for (`blur:` on
//! the workspace socket, in its own logical pixels: the bar's plate, its
//! popups), placed on the output and slotted into the frame just behind
//! the surface, the way a window's blur sits just behind the window. What
//! they blur is everything under the top layers: the wallpaper, the
//! bottom layers and every window.

use smithay::utils::{Logical, Point, Rectangle};

use crate::workspace_ipc::BlurRect;

/// A backdrop a layer surface asked for: the index in the frame's
/// elements it goes in at (just behind the surface's own), and its rect on
/// the output (logical, the output's own) with its corners' radius.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LayerBackdrop {
    pub behind: usize,
    pub rect: Rectangle<i32, Logical>,
    pub radius: f32,
}

/// `rects` on a surface whose top-left is at `at` on the output, each to
/// go in at `behind`.
pub fn placed(rects: &[BlurRect], at: Point<i32, Logical>, behind: usize) -> impl Iterator<Item = LayerBackdrop> + '_ {
    rects.iter().map(move |r| LayerBackdrop { behind, rect: Rectangle::new(r.rect.loc + at, r.rect.size), radius: r.radius as f32 })
}

/// Put `element` in `elements` at `at`, the no-capture slots from there on
/// moving along one with what they name.
pub fn insert<T>(elements: &mut Vec<T>, nocapture: &mut [usize], at: usize, element: T) {
    let at = at.min(elements.len());
    elements.insert(at, element);
    for slot in nocapture.iter_mut().filter(|slot| **slot >= at) {
        *slot += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_surfaces_rects_land_where_it_is() {
        let plate = BlurRect { rect: Rectangle::new((16, 8).into(), (1888, 64).into()), radius: 16 };
        let placed: Vec<_> = placed(&[plate], (0, 1008).into(), 3).collect();
        assert_eq!(placed, [LayerBackdrop { behind: 3, rect: Rectangle::new((16, 1016).into(), (1888, 64).into()), radius: 16.0 }]);
    }

    #[test]
    fn a_backdrop_slotted_in_moves_what_comes_after_it() {
        let mut elements = vec!['a', 'b', 'c'];
        let mut nocapture = [0, 2];
        insert(&mut elements, &mut nocapture, 1, 'x');
        assert_eq!(elements, ['a', 'x', 'b', 'c']);
        assert_eq!(nocapture, [0, 3], "still naming a and c");
    }
}

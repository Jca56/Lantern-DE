//! Which way a video frame faces, from its stream's `image-orientation` tag.

/// How a picture must be turned to stand the right way up, from the
/// stream's `image-orientation` tag: cameras and phones held upright record
/// on their side and say so there (a ZV-E10 in portrait: `rotate-270`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Orientation {
    /// Quarter turns clockwise, 0-3.
    pub quarter_turns: u8,
    /// After the turn, mirrored left to right.
    pub mirror: bool,
}

impl Orientation {
    /// `rotate-N` / `flip-rotate-N` (N in degrees clockwise); None for
    /// anything else.
    pub fn from_tag(tag: &str) -> Option<Self> {
        let (mirror, degrees) = match tag.strip_prefix("flip-") {
            Some(rest) => (true, rest),
            None => (false, tag),
        };
        let quarter_turns = match degrees.strip_prefix("rotate-")? {
            "0" => 0,
            "90" => 1,
            "180" => 2,
            "270" => 3,
            _ => return None,
        };
        Some(Self { quarter_turns, mirror })
    }

    /// Width and height trade places on screen.
    pub fn swaps_axes(self) -> bool {
        self.quarter_turns % 2 == 1
    }

    /// Bits 0-1 the turns, bit 2 the mirror: small enough for an atomic.
    pub fn pack(self) -> u8 {
        self.quarter_turns | (self.mirror as u8) << 2
    }

    pub fn unpack(bits: u8) -> Self {
        Self { quarter_turns: bits & 3, mirror: bits & 4 != 0 }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_every_tag_gstreamer_writes() {
        assert_eq!(Orientation::from_tag("rotate-0"), Some(Orientation::default()));
        let sony = Orientation::from_tag("rotate-270").unwrap();
        assert_eq!((sony.quarter_turns, sony.mirror), (3, false));
        assert!(sony.swaps_axes());
        let flipped = Orientation::from_tag("flip-rotate-90").unwrap();
        assert_eq!((flipped.quarter_turns, flipped.mirror), (1, true));
        assert!(!Orientation::from_tag("flip-rotate-180").unwrap().swaps_axes());
        assert_eq!(Orientation::from_tag("rotate-45"), None);
        assert_eq!(Orientation::from_tag("sideways"), None);
    }

    #[test]
    fn packs_and_unpacks() {
        for turns in 0..4 {
            for mirror in [false, true] {
                let o = Orientation { quarter_turns: turns, mirror };
                assert_eq!(Orientation::unpack(o.pack()), o);
            }
        }
    }
}

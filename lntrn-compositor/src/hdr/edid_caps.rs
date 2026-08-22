//! HDR capability detection from a display's EDID.
//!
//! We read the CTA-861 HDR static metadata block (which EOTFs the panel
//! supports, its desired content luminance range) plus the display's color
//! primaries / white point via `libdisplay-info`. The result feeds two things:
//!   1. the settings app (so the HDR toggle only appears on capable displays), and
//!   2. the DRM `HDR_OUTPUT_METADATA` infoframe we send when HDR is enabled
//!      (`hdr::drm_props`), which wants the panel's real primaries + luminance.
//!
//! Nothing here is NVIDIA- or connector-specific — capability is decided purely
//! by what the plugged-in display advertises, so the same binary does the right
//! thing on the laptop's built-in panel and the desktop's external monitors.

use libdisplay_info::info::Info;

/// A CIE 1931 (x, y) chromaticity coordinate.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Chromaticity {
    pub x: f32,
    pub y: f32,
}

/// Rec.2020 primaries + D65 white, used as a sane default when the EDID doesn't
/// carry color-primary data (rare, but some panels omit it).
impl Chromaticity {
    const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

/// Rec.2020 R/G/B primaries (BT.2020 container — what an HDR signal uses).
pub const BT2020_PRIMARIES: [Chromaticity; 3] = [
    Chromaticity::new(0.708, 0.292), // R
    Chromaticity::new(0.170, 0.797), // G
    Chromaticity::new(0.131, 0.046), // B
];
/// D65 white point.
pub const D65_WHITE: Chromaticity = Chromaticity::new(0.3127, 0.3290);

/// HDR capabilities + mastering parameters extracted from a display's EDID.
///
/// `primaries` / `white_point` describe the *panel's* actual gamut (from the
/// EDID color-primaries block when present, else Rec.2020), which is what we put
/// in the mastering-display section of the HDR infoframe.
#[derive(Debug, Clone)]
pub struct HdrCaps {
    /// Display advertises the PQ (SMPTE ST.2084) EOTF — i.e. HDR10-capable.
    pub pq_supported: bool,
    /// Display advertises the HLG EOTF.
    pub hlg_supported: bool,
    /// Display advertises BT.2020 RGB signal colorimetry.
    pub bt2020_supported: bool,
    /// Peak luminance the display wants content mastered to, in nits.
    pub max_luminance: f32,
    /// Minimum (black) luminance, in nits.
    pub min_luminance: f32,
    /// Max frame-average light level the display targets, in nits.
    pub max_frame_avg_luminance: f32,
    /// Panel R/G/B primaries (CIE 1931).
    pub primaries: [Chromaticity; 3],
    /// Panel white point (CIE 1931).
    pub white_point: Chromaticity,
}

impl HdrCaps {
    /// True when the display can actually do HDR (supports PQ or HLG). We gate
    /// the settings toggle and the DRM-prop path on this.
    pub fn is_hdr_capable(&self) -> bool {
        self.pq_supported || self.hlg_supported
    }
}

/// Detect HDR capabilities from a parsed EDID. Returns `None` only when the
/// display reports no HDR static metadata at all (pure-SDR panel) — in that case
/// the toggle never appears and we never touch the connector's color props.
pub fn detect(info: &Info) -> Option<HdrCaps> {
    let hdr = info.hdr_static_metadata();

    // No HDR static metadata block, or it only flags traditional SDR → not HDR.
    if !hdr.pq && !hdr.hlg {
        return None;
    }

    // Luminance: EDID gives "desired content" luminance the panel wants. Fall
    // back to a conservative HDR10 baseline if a field is zero/unreported.
    let max_luminance = if hdr.desired_content_max_luminance > 0.0 {
        hdr.desired_content_max_luminance
    } else {
        1000.0
    };
    let min_luminance = if hdr.desired_content_min_luminance > 0.0 {
        hdr.desired_content_min_luminance
    } else {
        0.0
    };
    let max_frame_avg_luminance = if hdr.desired_content_max_frame_avg_luminance > 0.0 {
        hdr.desired_content_max_frame_avg_luminance
    } else {
        max_luminance
    };

    // Color primaries: prefer the EDID's reported chromaticities; default to
    // Rec.2020 when the panel omits them.
    let cp = info.default_color_primaries();
    let (primaries, white_point) = if cp.has_primaries {
        let p = [
            Chromaticity {
                x: cp.primary[0].x,
                y: cp.primary[0].y,
            },
            Chromaticity {
                x: cp.primary[1].x,
                y: cp.primary[1].y,
            },
            Chromaticity {
                x: cp.primary[2].x,
                y: cp.primary[2].y,
            },
        ];
        let w = if cp.has_default_white_point {
            Chromaticity {
                x: cp.default_white.x,
                y: cp.default_white.y,
            }
        } else {
            D65_WHITE
        };
        (p, w)
    } else {
        (BT2020_PRIMARIES, D65_WHITE)
    };

    let colorimetry = info.supported_signal_colorimetry();

    Some(HdrCaps {
        pq_supported: hdr.pq,
        hlg_supported: hdr.hlg,
        bt2020_supported: colorimetry.bt2020_rgb,
        max_luminance,
        min_luminance,
        max_frame_avg_luminance,
        primaries,
        white_point,
    })
}

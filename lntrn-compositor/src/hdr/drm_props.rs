//! DRM connector-property signalling for HDR.
//!
//! To put a display into HDR mode we set three connector properties:
//!   * `HDR_OUTPUT_METADATA` — a blob holding `struct hdr_output_metadata`
//!     (EOTF = PQ, mastering primaries/luminance from the panel's EDID). The
//!     driver turns this into a DRM InfoFrame / SDP and the panel switches to
//!     HDR.
//!   * `Colorspace` — set to `BT2020_RGB` when HDR is on (else `Default`).
//!   * `max bpc` — request 10 bits-per-channel when HDR is on.
//!
//! These are set with the legacy `set_property` ioctl on the surface's DRM fd
//! (the `DrmSurface` implements `ControlDevice`). On atomic drivers the kernel
//! translates legacy connector-property sets into an atomic commit. Smithay's
//! `DrmCompositor` owns the atomic page-flips and gives us no hook to inject
//! connector props into them, so this separate set is the pragmatic path —
//! mirroring how the project keeps `vrr` minimal. It degrades gracefully: a
//! driver that doesn't expose the property simply logs and no-ops.

use smithay::backend::drm::DrmSurface;
use smithay::reexports::drm::control::{connector, property, Device as ControlDevice};
use tracing::{info, warn};

use super::edid_caps::{Chromaticity, HdrCaps};

// EOTF values for hdr_metadata_infoframe.eotf (CTA-861).
const EOTF_TRADITIONAL_SDR: u8 = 0;
const EOTF_SMPTE_ST2084_PQ: u8 = 2;
const STATIC_METADATA_TYPE1: u8 = 0;

/// `struct hdr_output_metadata` + `struct hdr_metadata_infoframe`, laid out to
/// match the kernel uapi (`drm_mode.h`) byte-for-byte so the blob is valid.
#[repr(C)]
#[derive(Clone, Copy)]
struct HdrOutputMetadata {
    metadata_type: u32,
    // union { struct hdr_metadata_infoframe hdmi_metadata_type1; }
    eotf: u8,
    infoframe_metadata_type: u8,
    display_primaries: [ChromaticityU16; 3],
    white_point: ChromaticityU16,
    max_display_mastering_luminance: u16,
    min_display_mastering_luminance: u16,
    max_cll: u16,
    max_fall: u16,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct ChromaticityU16 {
    x: u16,
    y: u16,
}

/// CIE chromaticity → the infoframe's 16-bit 0.00002-unit encoding (0xC350 = 1.0).
fn chroma_u16(c: Chromaticity) -> ChromaticityU16 {
    let enc = |v: f32| (v.clamp(0.0, 1.0) * 50000.0).round() as u16;
    ChromaticityU16 { x: enc(c.x), y: enc(c.y) }
}

impl HdrOutputMetadata {
    fn from_caps(caps: &HdrCaps) -> Self {
        Self {
            metadata_type: 0, // HDMI_STATIC_METADATA_TYPE1 descriptor id
            eotf: EOTF_SMPTE_ST2084_PQ,
            infoframe_metadata_type: STATIC_METADATA_TYPE1,
            display_primaries: [
                chroma_u16(caps.primaries[0]),
                chroma_u16(caps.primaries[1]),
                chroma_u16(caps.primaries[2]),
            ],
            white_point: chroma_u16(caps.white_point),
            // Mastering luminance: max in 1-nit units, min in 0.0001-nit units.
            max_display_mastering_luminance: caps.max_luminance.round() as u16,
            min_display_mastering_luminance: (caps.min_luminance * 10000.0).round() as u16,
            max_cll: caps.max_luminance.round() as u16,
            max_fall: caps.max_frame_avg_luminance.round() as u16,
        }
    }

    fn as_bytes(&self) -> &[u8] {
        // Safe: #[repr(C)] POD with no padding-sensitive reads; we only read it.
        unsafe {
            std::slice::from_raw_parts(
                self as *const _ as *const u8,
                std::mem::size_of::<Self>(),
            )
        }
    }
}

/// Property handles for the three HDR connector props, resolved once and cached
/// per output. Any may be `None` if the driver doesn't expose it.
#[derive(Clone, Default)]
pub struct HdrPropHandles {
    pub metadata: Option<property::Handle>,
    pub colorspace: Option<property::Handle>,
    pub max_bpc: Option<property::Handle>,
    /// Raw enum value for Colorspace = BT2020_RGB, if that enum exists.
    pub colorspace_bt2020: Option<property::RawValue>,
    /// Raw enum value for Colorspace = Default.
    pub colorspace_default: Option<property::RawValue>,
}

impl HdrPropHandles {
    pub fn any(&self) -> bool {
        self.metadata.is_some() || self.colorspace.is_some() || self.max_bpc.is_some()
    }
}

/// Enumerate the connector's properties and resolve the HDR-relevant handles.
/// Logs what was found so we can confirm driver support on real hardware.
pub fn resolve_props(surface: &DrmSurface, conn: connector::Handle) -> HdrPropHandles {
    let mut out = HdrPropHandles::default();
    let Ok(props) = surface.get_properties(conn) else {
        warn!("HDR: could not read connector properties");
        return out;
    };
    let Ok(map) = props.as_hashmap(surface) else {
        return out;
    };

    if let Some(info) = map.get("HDR_OUTPUT_METADATA") {
        out.metadata = Some(info.handle());
    }
    if let Some(info) = map.get("max bpc") {
        out.max_bpc = Some(info.handle());
    }
    if let Some(info) = map.get("Colorspace") {
        out.colorspace = Some(info.handle());
        if let property::ValueType::Enum(values) = info.value_type() {
            let (raws, enums) = values.values();
            for (raw, ev) in raws.iter().zip(enums.iter()) {
                match ev.name().to_str() {
                    Ok("BT2020_RGB") => out.colorspace_bt2020 = Some(*raw),
                    Ok("Default") => out.colorspace_default = Some(*raw),
                    _ => {}
                }
            }
        }
    }

    info!(
        "HDR connector props: metadata={} colorspace={} max_bpc={} bt2020_enum={}",
        out.metadata.is_some(),
        out.colorspace.is_some(),
        out.max_bpc.is_some(),
        out.colorspace_bt2020.is_some(),
    );
    out
}

/// Apply (or clear) HDR signalling on a connector. `enable` true → PQ/BT.2020,
/// 10 bpc; false → restore SDR defaults. Best-effort per property.
pub fn set_hdr_metadata(
    surface: &DrmSurface,
    conn: connector::Handle,
    handles: &HdrPropHandles,
    caps: &HdrCaps,
    enable: bool,
) {
    // HDR_OUTPUT_METADATA blob.
    if let Some(prop) = handles.metadata {
        if enable {
            let meta = HdrOutputMetadata::from_caps(caps);
            match surface.create_property_blob(meta.as_bytes()) {
                Ok(blob) => {
                    let raw: property::RawValue = blob.into();
                    if let Err(e) = surface.set_property(conn, prop, raw) {
                        warn!("HDR: set HDR_OUTPUT_METADATA failed: {e}");
                    } else {
                        info!(
                            "HDR: metadata committed (eotf=PQ, max={}nits)",
                            meta.max_display_mastering_luminance
                        );
                    }
                }
                Err(e) => warn!("HDR: create metadata blob failed: {e}"),
            }
        } else {
            // 0 clears the metadata (driver reverts to SDR signalling).
            let _ = surface.set_property(conn, prop, 0);
        }
    }

    // Colorspace enum.
    if let Some(prop) = handles.colorspace {
        let val = if enable { handles.colorspace_bt2020 } else { handles.colorspace_default };
        if let Some(v) = val {
            if let Err(e) = surface.set_property(conn, prop, v) {
                warn!("HDR: set Colorspace failed: {e}");
            }
        }
    }

    // max bpc: 10 for HDR, 8 for SDR.
    if let Some(prop) = handles.max_bpc {
        let bpc = if enable { 10 } else { 8 };
        if let Err(e) = surface.set_property(conn, prop, bpc) {
            warn!("HDR: set max bpc failed: {e}");
        }
    }

    let _ = EOTF_TRADITIONAL_SDR; // kept for documentation/future HLG path
}

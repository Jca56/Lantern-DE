//! Device capabilities — the vendor-agnostic contract.
//!
//! A connected peripheral is a [`Device`]; it hands back whichever
//! capability interfaces it actually supports (`None` otherwise). The UI,
//! daemon and CLI only ever touch these traits, so they work the same
//! against any backend.

use crate::error::Error;

/// A 24-bit RGB color.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Rgb {
    pub fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }
    pub fn hex(&self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
    }
}

/// A DPI capability range: `min..=max` in increments of `step`.
#[derive(Clone, Copy, Debug)]
pub struct DpiRange {
    pub min: u16,
    pub max: u16,
    pub step: u16,
}

impl DpiRange {
    /// Clamp `dpi` into range and snap it to the nearest step.
    pub fn snap(&self, dpi: u16) -> u16 {
        let step = self.step.max(1);
        let clamped = dpi.clamp(self.min, self.max);
        let snapped = ((clamped - self.min + step / 2) / step) * step + self.min;
        snapped.min(self.max)
    }
}

/// What kind of peripheral this is — drives UI grouping/icons.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeviceKind {
    Mouse,
    Keyboard,
    Headset,
    Other,
}

impl DeviceKind {
    pub fn label(self) -> &'static str {
        match self {
            DeviceKind::Mouse => "mouse",
            DeviceKind::Keyboard => "keyboard",
            DeviceKind::Headset => "headset",
            DeviceKind::Other => "device",
        }
    }
}

/// A connected peripheral. Returns the capability interfaces it supports.
pub trait Device {
    fn name(&self) -> &str;
    fn kind(&self) -> DeviceKind;

    /// Lighting control, if the device has addressable LEDs.
    fn lighting(&mut self) -> Option<&mut dyn Lighting> {
        None
    }
    /// DPI control, if the device is a sensor-bearing pointer.
    fn dpi(&mut self) -> Option<&mut dyn Dpi> {
        None
    }
}

/// Addressable RGB lighting, organized into zones.
pub trait Lighting {
    /// Number of independently-addressable LED zones.
    fn zone_count(&mut self) -> u8;
    /// Set one zone to a solid color.
    fn set_fixed(&mut self, zone: u8, color: Rgb) -> Result<(), Error>;
    /// Set every zone to the same solid color.
    fn set_all(&mut self, color: Rgb) -> Result<(), Error> {
        for zone in 0..self.zone_count() {
            self.set_fixed(zone, color)?;
        }
        Ok(())
    }
}

/// Pointer DPI / sensitivity control.
pub trait Dpi {
    fn get(&mut self) -> Result<u16, Error>;
    fn set(&mut self, dpi: u16) -> Result<(), Error>;
    fn range(&mut self) -> Result<DpiRange, Error>;
}

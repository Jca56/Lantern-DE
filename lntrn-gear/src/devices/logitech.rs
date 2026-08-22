//! Logitech driver: maps HID++ 2.0 features onto the [`crate::caps`]
//! interfaces. Because HID++ is feature-discovered, this one driver
//! covers Logitech mice, keyboards and headsets — whatever features a
//! given device exposes, we light up the matching capability.

use std::path::Path;

use crate::caps::{Device, DeviceKind, Dpi, DpiRange, Lighting, Rgb};
use crate::error::Error;
use crate::transport::hidpp::{HidppDevice, DEV_WIRED};

const F_DEVICE_NAME: u16 = 0x0005;
const F_ADJUSTABLE_DPI: u16 = 0x2201;
const F_COLOR_LED_EFFECTS: u16 = 0x8070;
/// Effect id 0x0001 is the conventional "fixed/static color" effect.
const EFFECT_FIXED: u16 = 0x0001;

pub struct LogitechDevice {
    hid: HidppDevice,
    name: String,
    kind: DeviceKind,
    /// Resolved feature indices, `None` when unsupported.
    dpi_idx: Option<u8>,
    led_idx: Option<u8>,
}

impl LogitechDevice {
    /// Bring up a Logitech HID++ device on `path`. `Ok(None)` means the
    /// node doesn't answer HID++ (e.g. it's the wrong HID interface).
    pub fn open(path: &Path) -> Result<Option<Self>, Error> {
        let mut hid = HidppDevice::open(path)?;

        // Find the responsive device index — wired (0xff) first, then a
        // receiver slot. All-wired setups only ever need 0xff.
        let mut live = false;
        for di in [DEV_WIRED, 0x01] {
            hid.set_device_index(di);
            if hid.ping().is_ok() {
                live = true;
                break;
            }
        }
        if !live {
            return Ok(None);
        }

        let resolve = |hid: &mut HidppDevice, id: u16| match hid.feature_index(id) {
            Ok(0) | Err(_) => None,
            Ok(i) => Some(i),
        };
        let dpi_idx = resolve(&mut hid, F_ADJUSTABLE_DPI);
        let led_idx = resolve(&mut hid, F_COLOR_LED_EFFECTS);
        let name = resolve(&mut hid, F_DEVICE_NAME)
            .and_then(|i| read_device_name(&mut hid, i).ok())
            .unwrap_or_else(|| "Logitech device".to_string());
        let kind = guess_kind(&name, dpi_idx.is_some());

        Ok(Some(Self {
            hid,
            name,
            kind,
            dpi_idx,
            led_idx,
        }))
    }
}

impl Device for LogitechDevice {
    fn name(&self) -> &str {
        &self.name
    }
    fn kind(&self) -> DeviceKind {
        self.kind
    }
    fn lighting(&mut self) -> Option<&mut dyn Lighting> {
        self.led_idx.map(|_| self as &mut dyn Lighting)
    }
    fn dpi(&mut self) -> Option<&mut dyn Dpi> {
        self.dpi_idx.map(|_| self as &mut dyn Dpi)
    }
}

impl Lighting for LogitechDevice {
    fn zone_count(&mut self) -> u8 {
        let Some(idx) = self.led_idx else { return 0 };
        // 0x8070 getInfo (fn 0x00) → zone count at payload[0].
        self.hid
            .call(idx, 0x00, &[0, 0, 0])
            .ok()
            .and_then(|r| r.first().copied())
            .unwrap_or(0)
    }

    fn set_fixed(&mut self, zone: u8, color: Rgb) -> Result<(), Error> {
        let idx = self.led_idx.ok_or(Error::Unsupported)?;
        // Find this zone's "fixed" effect by enumerating, not guessing.
        let effect_count = self
            .hid
            .call(idx, 0x01, &[zone, 0, 0])? // getZoneInfo
            .get(3)
            .copied()
            .unwrap_or(0);
        let mut fixed = None;
        for e in 0..effect_count.min(16) {
            // getZoneEffectInfo (fn 0x02) → effect id at payload[2..4].
            let r = self.hid.call(idx, 0x02, &[zone, e, 0])?;
            let id = ((r.get(2).copied().unwrap_or(0) as u16) << 8)
                | r.get(3).copied().unwrap_or(0) as u16;
            if id == EFFECT_FIXED {
                fixed = Some(e);
                break;
            }
        }
        let effect = fixed.ok_or(Error::Unsupported)?;
        // setZoneEffect (fn 0x03), long report: [zone, effect, R, G, B, 0…].
        let params = [
            zone, effect, color.r, color.g, color.b, 0, 0, 0, 0, 0, 0, 0, 0,
        ];
        self.hid.call(idx, 0x03, &params)?;
        Ok(())
    }
}

impl Dpi for LogitechDevice {
    fn get(&mut self) -> Result<u16, Error> {
        let idx = self.dpi_idx.ok_or(Error::Unsupported)?;
        // getSensorDpi (fn 0x02) → [sensor, dpi_hi, dpi_lo, …] big-endian.
        let r = self.hid.call(idx, 0x02, &[0, 0, 0])?;
        Ok(((r.get(1).copied().unwrap_or(0) as u16) << 8) | r.get(2).copied().unwrap_or(0) as u16)
    }

    fn set(&mut self, dpi: u16) -> Result<(), Error> {
        let idx = self.dpi_idx.ok_or(Error::Unsupported)?;
        let dpi = self.range()?.snap(dpi);
        // setSensorDpi (fn 0x03): [sensor, dpi_hi, dpi_lo].
        self.hid
            .call(idx, 0x03, &[0, (dpi >> 8) as u8, dpi as u8])?;
        Ok(())
    }

    fn range(&mut self) -> Result<DpiRange, Error> {
        let idx = self.dpi_idx.ok_or(Error::Unsupported)?;
        // getSensorDpiList (fn 0x01): [sensor, then BE u16 values];
        // a 0xE0xx value is a "step" marker, 0x0000 terminates.
        let p = self.hid.call(idx, 0x01, &[0, 0, 0])?;
        let mut vals = Vec::new();
        let mut i = 1;
        while i + 1 < p.len() {
            let v = ((p[i] as u16) << 8) | p[i + 1] as u16;
            if v == 0 {
                break;
            }
            vals.push(v);
            i += 2;
        }
        if vals.len() == 3 && (vals[1] & 0xe000) == 0xe000 {
            Ok(DpiRange {
                min: vals[0],
                max: vals[2],
                step: vals[1] & 0x1fff,
            })
        } else if let (Some(&min), Some(&max)) = (vals.iter().min(), vals.iter().max()) {
            Ok(DpiRange { min, max, step: 50 })
        } else {
            Err(Error::Unsupported)
        }
    }
}

/// 0x0005 DeviceName: `getCount` (fn 0) then `getDeviceName` (fn 1) chunks.
fn read_device_name(hid: &mut HidppDevice, idx: u8) -> Result<String, Error> {
    let len = hid
        .call(idx, 0x00, &[0, 0, 0])?
        .first()
        .copied()
        .unwrap_or(0) as usize;
    let mut name = String::new();
    while name.len() < len {
        let start = name.len() as u8;
        let chunk = hid.call(idx, 0x01, &[start, 0, 0])?;
        let mut added = 0;
        for &b in &chunk {
            if b == 0 || name.len() >= len {
                break;
            }
            name.push(b as char);
            added += 1;
        }
        if added == 0 {
            break;
        }
    }
    Ok(name)
}

fn guess_kind(name: &str, has_dpi: bool) -> DeviceKind {
    let n = name.to_lowercase();
    if n.contains("mouse") {
        DeviceKind::Mouse
    } else if n.contains("keyboard") || n.contains("kbd") {
        DeviceKind::Keyboard
    } else if n.contains("headset") || n.contains("headphone") {
        DeviceKind::Headset
    } else if has_dpi {
        DeviceKind::Mouse
    } else {
        DeviceKind::Other
    }
}

//! HID++ 2.0 transport over a hidraw node — pure std.
//!
//! HID++ is asynchronous: a request report is written, and the device
//! sends responses *and* unsolicited events on the same fd. A reader
//! thread pumps every incoming report onto an `mpsc` channel; a request
//! writes its report then waits for the response whose device index +
//! feature index + software id match (skipping events and mouse input
//! reports).
//!
//! Reports are "short" (id `0x10`, 7 bytes) or "long" (id `0x11`, 20
//! bytes): `[report_id, device_index, feature_index, (function<<4)|sw_id,
//! params...]`.

use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::Path;
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};

use crate::error::Error;

pub const SHORT_REPORT_ID: u8 = 0x10;
pub const LONG_REPORT_ID: u8 = 0x11;
const SHORT_LEN: usize = 7;
const LONG_LEN: usize = 20;

/// Software id (4 bits, 1..=15) stamped into requests so we pick our own
/// responses out of the report stream.
const SW_ID: u8 = 0x0a;

/// Device index for a directly-connected (wired) device. Receiver-paired
/// devices use 0x01..=0x06 instead.
pub const DEV_WIRED: u8 = 0xff;

const TIMEOUT: Duration = Duration::from_millis(500);

/// The IFeatureSet feature — enumerate the device's whole feature table.
pub const F_FEATURE_SET: u16 = 0x0001;

pub struct HidppDevice {
    write: File,
    rx: Receiver<Vec<u8>>,
    dev_index: u8,
}

impl HidppDevice {
    /// Open a hidraw node for HID++. Spawns a reader thread that lives for
    /// the process (it blocks on `read`; the OS reaps it at exit).
    pub fn open(path: &Path) -> std::io::Result<Self> {
        let write = OpenOptions::new().read(true).write(true).open(path)?;
        let mut read = write.try_clone()?;
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let mut buf = [0u8; 64];
            loop {
                match read.read(&mut buf) {
                    Ok(n) if n > 0 => {
                        if tx.send(buf[..n].to_vec()).is_err() {
                            break;
                        }
                    }
                    Ok(_) => {}
                    Err(_) => break,
                }
            }
        });
        Ok(Self {
            write,
            rx,
            dev_index: DEV_WIRED,
        })
    }

    pub fn set_device_index(&mut self, idx: u8) {
        self.dev_index = idx;
    }

    /// Send a request; return the response payload (bytes after the 4-byte
    /// header). `params` length picks short vs long report.
    pub fn call(
        &mut self,
        feature_index: u8,
        function: u8,
        params: &[u8],
    ) -> Result<Vec<u8>, Error> {
        let (report_id, len) = if params.len() <= 3 {
            (SHORT_REPORT_ID, SHORT_LEN)
        } else {
            (LONG_REPORT_ID, LONG_LEN)
        };
        let mut req = vec![0u8; len];
        req[0] = report_id;
        req[1] = self.dev_index;
        req[2] = feature_index;
        req[3] = (function << 4) | (SW_ID & 0x0f);
        for (i, b) in params.iter().enumerate() {
            if 4 + i < len {
                req[4 + i] = *b;
            }
        }
        self.write.write_all(&req)?;

        let end = Instant::now() + TIMEOUT;
        loop {
            let remaining = end
                .checked_duration_since(Instant::now())
                .ok_or(Error::Timeout)?;
            let resp = self
                .rx
                .recv_timeout(remaining)
                .map_err(|_| Error::Timeout)?;
            // HID++ reports only — skip mouse input reports.
            if resp.is_empty() || (resp[0] != SHORT_REPORT_ID && resp[0] != LONG_REPORT_ID) {
                continue;
            }
            if resp.len() < 4 {
                continue;
            }
            // Error: [id, dev, 0xff, errored_feature, func|sw, code]
            if resp[2] == 0xff
                && resp.len() >= 6
                && resp[1] == self.dev_index
                && resp[3] == feature_index
                && (resp[4] & 0x0f) == SW_ID
            {
                return Err(Error::Protocol(resp[5]));
            }
            if resp[1] == self.dev_index && resp[2] == feature_index && (resp[3] & 0x0f) == SW_ID {
                return Ok(resp[4..].to_vec());
            }
            // else: event or foreign response — keep waiting.
        }
    }

    /// Root ping (feature 0x00, fn 0x01). Returns the protocol version
    /// `(major, minor)` if the device answers with our echoed marker.
    pub fn ping(&mut self) -> Result<(u8, u8), Error> {
        const MARKER: u8 = 0x5a;
        let r = self.call(0x00, 0x01, &[0x00, 0x00, MARKER])?;
        if r.len() >= 3 && r[2] == MARKER {
            Ok((r[0], r[1]))
        } else {
            Err(Error::Timeout)
        }
    }

    /// Root.getFeature(featureId) → the feature's index, or 0 if absent.
    pub fn feature_index(&mut self, feature_id: u16) -> Result<u8, Error> {
        let r = self.call(0x00, 0x00, &[(feature_id >> 8) as u8, feature_id as u8, 0])?;
        Ok(r.first().copied().unwrap_or(0))
    }
}

// ── Feature-table enumeration (diagnostics) ─────────────────────────────────

pub struct FeatureRow {
    pub index: u8,
    pub id: u16,
    pub type_flags: u8,
}

/// Enumerate the full feature table via IFeatureSet (`0x0001`).
pub fn enumerate(dev: &mut HidppDevice) -> Result<Vec<FeatureRow>, Error> {
    let fs_index = dev.feature_index(F_FEATURE_SET)?;
    if fs_index == 0 {
        return Ok(Vec::new());
    }
    let count = dev
        .call(fs_index, 0x00, &[0, 0, 0])?
        .first()
        .copied()
        .unwrap_or(0);
    let mut rows = vec![FeatureRow {
        index: 0,
        id: 0x0000,
        type_flags: 0,
    }];
    for i in 1..=count {
        let r = dev.call(fs_index, 0x01, &[i, 0, 0])?;
        let id =
            ((r.first().copied().unwrap_or(0) as u16) << 8) | r.get(1).copied().unwrap_or(0) as u16;
        rows.push(FeatureRow {
            index: i,
            id,
            type_flags: r.get(2).copied().unwrap_or(0),
        });
    }
    Ok(rows)
}

/// Best-effort human name for a HID++ feature id.
pub fn feature_name(id: u16) -> &'static str {
    match id {
        0x0000 => "ROOT",
        0x0001 => "FEATURE_SET",
        0x0003 => "DEVICE_FW_VERSION",
        0x0005 => "DEVICE_NAME",
        0x1000 => "BATTERY_UNIFIED",
        0x1300 => "LED_CONTROL",
        0x1b04 => "REPROG_CONTROLS_V4",
        0x2201 => "ADJUSTABLE_DPI",
        0x8060 => "REPORT_RATE",
        0x8070 => "COLOR_LED_EFFECTS",
        0x8071 => "RGB_EFFECTS",
        0x8100 => "ONBOARD_PROFILES",
        0x8110 => "MOUSE_BUTTON_SPY",
        _ => "(unknown)",
    }
}

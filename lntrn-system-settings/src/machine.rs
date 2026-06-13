//! Runtime hardware-capability detection — laptop vs desktop.
//!
//! Power settings differ by chassis: a laptop has a system battery and a lid
//! switch, a desktop has neither. Rather than hardcode per-machine assumptions
//! we probe sysfs/procfs once and cache the answer, then the Power panel and
//! sidebar show only the controls that make sense for the current hardware.
//!
//! Detection is deliberately conservative about what counts as a *system*
//! battery: wireless mice/keyboards expose their own `power_supply` entries
//! (e.g. `hidpp_battery_0`) with `type=Battery`, so a naive scan would flag a
//! desktop with a Logitech mouse as a laptop. We exclude `scope=Device`
//! batteries, which is how the kernel tags peripheral power supplies.

use std::sync::OnceLock;

use crate::wayland::Panel;

/// True if the machine has a built-in **system** battery (i.e. it's a laptop).
/// Peripheral batteries (`scope=Device`, e.g. HID++ mice) don't count.
pub(crate) fn has_battery() -> bool {
    static CACHE: OnceLock<bool> = OnceLock::new();
    *CACHE.get_or_init(|| {
        let Ok(dir) = std::fs::read_dir("/sys/class/power_supply") else {
            return false;
        };
        for entry in dir.flatten() {
            let path = entry.path();
            let kind = std::fs::read_to_string(path.join("type")).unwrap_or_default();
            if kind.trim() != "Battery" {
                continue;
            }
            // `scope` is "System" / "Device" / "Unknown"; absent on some older
            // ACPI batteries (treat absent as a real system battery). Only
            // peripheral supplies report "Device" — skip those.
            let scope = std::fs::read_to_string(path.join("scope")).unwrap_or_default();
            if scope.trim() != "Device" {
                return true;
            }
        }
        false
    })
}

/// True if the machine has a lid switch (laptop).
pub(crate) fn has_lid() -> bool {
    static CACHE: OnceLock<bool> = OnceLock::new();
    *CACHE.get_or_init(|| {
        // Modern kernels expose the lid as an input switch device.
        if let Ok(devs) = std::fs::read_to_string("/proc/bus/input/devices") {
            if devs.contains("Lid Switch") {
                return true;
            }
        }
        // Fallback: legacy ACPI procfs button interface.
        std::fs::read_dir("/proc/acpi/button/lid")
            .map(|mut d| d.next().is_some())
            .unwrap_or(false)
    })
}

/// Whether a power sub-panel applies to the current hardware. Battery controls
/// are hidden on a desktop; everything else is universal.
pub(crate) fn panel_available(panel: Panel) -> bool {
    match panel {
        Panel::Battery => has_battery(),
        _ => true,
    }
}

/// Display label for a panel, adapted to hardware. The "Lid & Idle" panel drops
/// the "Lid &" prefix on a lid-less desktop since only the idle rows remain.
pub(crate) fn panel_display_label(panel: Panel, fallback: &'static str) -> &'static str {
    match panel {
        Panel::LidIdle if !has_lid() => "Idle",
        _ => fallback,
    }
}

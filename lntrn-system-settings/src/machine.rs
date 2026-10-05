//! What the hardware has: a laptop shows lid and battery settings, a
//! desktop hides them. Probed once from sysfs and procfs.

use std::sync::OnceLock;

/// True if the machine has a built-in *system* battery. Wireless mice and
/// keyboards expose their own `power_supply` entries with `type=Battery`,
/// tagged `scope=Device`; those don't count.
pub fn has_battery() -> bool {
    static CACHE: OnceLock<bool> = OnceLock::new();
    *CACHE.get_or_init(|| {
        let Ok(dir) = std::fs::read_dir("/sys/class/power_supply") else { return false };
        for entry in dir.flatten() {
            let path = entry.path();
            let kind = std::fs::read_to_string(path.join("type")).unwrap_or_default();
            if kind.trim() != "Battery" {
                continue;
            }
            // `scope` is "System", "Device" or "Unknown", and absent on
            // some older ACPI batteries (a real one). Only peripherals
            // report "Device".
            let scope = std::fs::read_to_string(path.join("scope")).unwrap_or_default();
            if scope.trim() != "Device" {
                return true;
            }
        }
        false
    })
}

/// True if the machine has a lid switch.
pub fn has_lid() -> bool {
    static CACHE: OnceLock<bool> = OnceLock::new();
    *CACHE.get_or_init(|| {
        if let Ok(devs) = std::fs::read_to_string("/proc/bus/input/devices")
            && devs.contains("Lid Switch")
        {
            return true;
        }
        std::fs::read_dir("/proc/acpi/button/lid").map(|mut d| d.next().is_some()).unwrap_or(false)
    })
}

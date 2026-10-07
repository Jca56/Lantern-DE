//! The graphics cards. NVIDIA's are asked through its own library
//! (`nvml.rs`); the rest are read from what their driver puts in sysfs,
//! which for AMD is nearly everything and for Intel is little more than
//! a name.

use std::fs;
use std::path::{Path, PathBuf};

use super::nvml::Nvml;
use super::{push, read_number};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Gpu {
    pub name: String,
    /// The driver, and its version where it says: "nvidia 615.71.09".
    pub driver: String,
    /// Percent busy. `None` when the driver won't say.
    pub usage: Option<f32>,
    /// Video memory in bytes: `(used, total)`.
    pub memory: Option<(u64, u64)>,
    pub celsius: Option<f32>,
    pub watts: Option<f32>,
    /// The most it is allowed to draw.
    pub watts_limit: Option<f32>,
    /// Each fan, percent of its fastest.
    pub fans: Vec<u32>,
    pub core_mhz: Option<u32>,
    pub memory_mhz: Option<u32>,
    /// `usage` over time, oldest first.
    pub usage_history: Vec<f32>,
    /// Percent of the video memory in use over time.
    pub memory_history: Vec<f32>,
}

impl Gpu {
    pub fn memory_percent(&self) -> Option<f32> {
        self.memory.filter(|(_, total)| *total > 0).map(|(used, total)| (used as f64 / total as f64 * 100.0) as f32)
    }
}

/// A card found in sysfs: its `device` folder and what to call it.
struct SysCard {
    dir: PathBuf,
    name: String,
    driver: String,
}

/// The name of PCI device `vendor:device` (hex, as sysfs writes them)
/// from the `pci.ids` list, when the machine has one.
fn pci_name(ids: &str, vendor: &str, device: &str) -> Option<String> {
    let (vendor, device) = (vendor.to_ascii_lowercase(), device.to_ascii_lowercase());
    if vendor.len() != 4 || device.len() != 4 {
        return None;
    }
    // A maker's line starts at the margin with its number.
    let mut lines = ids.lines().skip_while(|l| !l.strip_prefix(&vendor).is_some_and(|rest| rest.starts_with(' ')));
    lines.next()?;
    // The vendor's devices follow it, each behind one tab.
    lines.take_while(|l| l.starts_with('\t') || l.starts_with('#') || l.is_empty()).find_map(|l| l.strip_prefix('\t')?.strip_prefix(&device).filter(|rest| rest.starts_with(' ')).map(|rest| rest.trim().to_owned()))
}

fn vendor_name(id: &str) -> &'static str {
    match id.to_ascii_lowercase().as_str() {
        "10de" => "NVIDIA",
        "1002" | "1022" => "AMD",
        "8086" => "Intel",
        _ => "",
    }
}

/// What to call the card `vendor:device`: a name from the list, with
/// the maker in front unless the list already has it there.
fn card_name(ids: &str, vendor: &str, device: &str) -> String {
    let maker = vendor_name(vendor);
    match pci_name(ids, vendor, device) {
        // "TGL GT2 [Iris Xe Graphics]": what is in the brackets is the name people know.
        Some(listed) => {
            let known = listed.rsplit_once('[').and_then(|(_, b)| b.strip_suffix(']')).unwrap_or(&listed);
            if maker.is_empty() || known.starts_with(maker) { known.to_owned() } else { format!("{maker} {known}") }
        }
        None if maker.is_empty() => "Graphics card".to_owned(),
        None => format!("{maker} graphics"),
    }
}

/// The cards in `/sys/class/drm` driven by anything but `skip`.
fn sys_cards(skip: Option<&str>) -> Vec<SysCard> {
    let Ok(dir) = fs::read_dir("/sys/class/drm") else { return Vec::new() };
    let ids = ["/usr/share/hwdata/pci.ids", "/usr/share/misc/pci.ids", "/usr/share/pci.ids"].iter().find_map(|p| fs::read_to_string(p).ok()).unwrap_or_default();
    let mut names: Vec<String> = dir.flatten().filter_map(|e| e.file_name().into_string().ok()).filter(|n| n.strip_prefix("card").is_some_and(|d| !d.is_empty() && d.bytes().all(|b| b.is_ascii_digit()))).collect();
    names.sort();
    names
        .iter()
        .filter_map(|card| {
            let dir = PathBuf::from(format!("/sys/class/drm/{card}/device"));
            let uevent = fs::read_to_string(dir.join("uevent")).ok()?;
            let field = |key: &str| uevent.lines().find_map(|l| l.strip_prefix(key)).unwrap_or("").trim();
            let driver = field("DRIVER=");
            if driver.is_empty() || Some(driver) == skip {
                return None;
            }
            let (vendor, device) = field("PCI_ID=").split_once(':').unwrap_or(("", ""));
            Some(SysCard { name: card_name(&ids, vendor, device), driver: driver.to_owned(), dir })
        })
        .collect()
}

/// The first of a card's hwmon folders: where amdgpu and the like put
/// its temperature and power.
fn hwmon(dir: &Path) -> Option<PathBuf> {
    fs::read_dir(dir.join("hwmon")).ok()?.flatten().map(|e| e.path()).next()
}

fn read_sys(card: &SysCard) -> Gpu {
    let file = |name: &str| card.dir.join(name).to_string_lossy().into_owned();
    let hw = hwmon(&card.dir);
    let sensor = |name: &str| hw.as_ref().and_then(|h| read_number::<f64>(&h.join(name).to_string_lossy()));
    Gpu {
        name: card.name.clone(),
        driver: card.driver.clone(),
        usage: read_number::<f32>(&file("gpu_busy_percent")).map(|u| u.clamp(0.0, 100.0)),
        memory: read_number::<u64>(&file("mem_info_vram_used")).zip(read_number::<u64>(&file("mem_info_vram_total"))).filter(|(_, total)| *total > 0),
        celsius: sensor("temp1_input").map(|t| (t / 1000.0) as f32),
        // Microwatts.
        watts: sensor("power1_average").or(sensor("power1_input")).map(|p| (p / 1e6) as f32),
        watts_limit: sensor("power1_cap").map(|p| (p / 1e6) as f32).filter(|w| *w > 0.0),
        fans: sensor("pwm1").map(|p| (p / 255.0 * 100.0).round() as u32).into_iter().collect(),
        // Hertz.
        core_mhz: sensor("freq1_input").map(|f| (f / 1e6) as u32).filter(|m| *m > 0),
        ..Gpu::default()
    }
}

pub struct GpuSampler {
    nvml: Option<Nvml>,
    nvml_names: Vec<String>,
    sys: Vec<SysCard>,
    /// Usage and memory histories, a pair per card in the order they
    /// are listed.
    histories: Vec<(Vec<f32>, Vec<f32>)>,
}

impl GpuSampler {
    pub fn new() -> Self {
        let nvml = Nvml::open();
        let nvml_names = nvml.as_ref().map(Nvml::names).unwrap_or_default();
        // With the library speaking for NVIDIA's cards, sysfs is only
        // asked about the others.
        let sys = sys_cards(nvml.is_some().then_some("nvidia"));
        let histories = vec![(Vec::new(), Vec::new()); nvml_names.len() + sys.len()];
        Self { nvml, nvml_names, sys, histories }
    }

    pub fn sample(&mut self) -> Vec<Gpu> {
        let mut gpus: Vec<Gpu> = Vec::with_capacity(self.histories.len());
        if let Some(nvml) = &self.nvml {
            let driver = format!("nvidia {}", nvml.driver()).trim().to_owned();
            for (i, name) in self.nvml_names.iter().enumerate() {
                let r = nvml.read(i);
                gpus.push(Gpu { name: name.clone(), driver: driver.clone(), usage: r.usage, memory: r.memory, celsius: r.celsius, watts: r.watts, watts_limit: r.watts_limit, fans: r.fans, core_mhz: r.core_mhz, memory_mhz: r.memory_mhz, ..Gpu::default() });
            }
        }
        gpus.extend(self.sys.iter().map(read_sys));
        for (gpu, (usage, memory)) in gpus.iter_mut().zip(&mut self.histories) {
            if let Some(u) = gpu.usage {
                push(usage, u);
            }
            if let Some(m) = gpu.memory_percent() {
                push(memory, m);
            }
            gpu.usage_history = usage.clone();
            gpu.memory_history = memory.clone();
        }
        gpus
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const IDS: &str = "# a comment\n1002  Advanced Micro Devices, Inc. [AMD/ATI]\n\t73bf  Navi 21 [Radeon RX 6800/6800 XT / 6900 XT]\n\t\t1002 0e3a  Radeon RX 6900 XT\n10de  NVIDIA Corporation\n\t2204  GA102 [GeForce RTX 3090]\n# mid comment\n\t2208  GA102 [GeForce RTX 3080 Ti]\n8086  Intel Corporation\n\t9a49  TigerLake-LP GT2 [Iris Xe Graphics]\n\t2208  Something Else Entirely\n";

    #[test]
    fn a_card_is_named_from_the_list_by_what_people_call_it() {
        assert_eq!(card_name(IDS, "10DE", "2208"), "NVIDIA GeForce RTX 3080 Ti");
        assert_eq!(card_name(IDS, "1002", "73bf"), "AMD Radeon RX 6800/6800 XT / 6900 XT");
        assert_eq!(card_name(IDS, "8086", "9a49"), "Intel Iris Xe Graphics");
        // The same device number under another maker is another device.
        assert_eq!(pci_name(IDS, "8086", "2208").as_deref(), Some("Something Else Entirely"));
        // Not in the list, no list, a maker nobody knows.
        assert_eq!(card_name(IDS, "10de", "ffff"), "NVIDIA graphics");
        assert_eq!(card_name("", "8086", "9a49"), "Intel graphics");
        assert_eq!(card_name(IDS, "1234", "1111"), "Graphics card");
    }

    #[test]
    fn this_machines_cards_are_read_and_their_histories_grow() {
        let mut s = GpuSampler::new();
        let first = s.sample();
        let again = s.sample();
        assert_eq!(first.len(), again.len());
        for gpu in &again {
            assert!(!gpu.name.is_empty() && !gpu.driver.is_empty());
            // A card that says how busy it is has said so twice now.
            assert!(gpu.usage.is_none() || gpu.usage_history.len() == 2);
        }
    }
}

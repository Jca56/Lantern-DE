//! The graphics cards. NVIDIA's are found on the PCI bus and asked
//! through its own library (`nvml.rs`); the rest are read from what their
//! driver puts in sysfs, which for AMD is nearly everything and for Intel
//! is little more than a name.
//!
//! A card that is asleep (a laptop's, powered down while nothing uses
//! it) is left asleep: anything asked of it would wake it. A bar looking
//! every second would also keep an idle card from ever falling asleep,
//! so it can [`GpuSampler::let_sleep`]: a card that may sleep and has sat
//! idle a while is then only looked at now and then, with NVIDIA's
//! library let go between looks, until it sleeps or gets to work.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use super::nvml::Nvml;
use super::{push, read, read_number, trim};

/// Idle this long, a card that may sleep is looked at only every
/// [`IDLE_LOOK`].
const IDLE_FOR: Duration = Duration::from_secs(10);
const IDLE_LOOK: Duration = Duration::from_secs(30);
/// Between tries at NVIDIA's library when it won't open.
const NVML_RETRY: Duration = Duration::from_secs(30);
const NVIDIA: &str = "0x10de";

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Gpu {
    pub name: String,
    /// The driver, and its version where it says: "nvidia 615.71.09".
    pub driver: String,
    /// Powered down while nothing uses it: nothing below is read.
    pub asleep: bool,
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

/// The card a bar's one monitor shows: the first that says how busy it
/// is, or is asleep (and so isn't busy at all). None of an Intel-only
/// machine's say.
pub fn shown(gpus: &[Gpu]) -> Option<&Gpu> {
    gpus.iter().find(|g| g.usage.is_some() || g.asleep)
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

fn pci_ids() -> String {
    ["/usr/share/hwdata/pci.ids", "/usr/share/misc/pci.ids", "/usr/share/pci.ids"].iter().find_map(|p| fs::read_to_string(p).ok()).unwrap_or_default()
}

/// Where a card's readings come from.
enum Source {
    /// NVIDIA's library, by the card's PCI address; `index` is its
    /// number there while the library is open.
    Nvml { bus: String, index: Option<usize> },
    /// Sysfs: the card's `device` folder.
    Sys(PathBuf),
}

/// A card, and what is kept of it between looks.
struct Card {
    source: Source,
    name: String,
    driver: String,
    /// Its PCI device's folder: whether it sleeps, and may.
    pci: PathBuf,
    /// Its driver puts it to sleep while nothing uses it, and it isn't
    /// the card the screens start on (which never sleeps).
    may_sleep: bool,
    /// Since when it has read idle, and when it was last read.
    idle_since: Option<Instant>,
    read_at: Option<Instant>,
    /// What it read last.
    last: Gpu,
    usage: Vec<f32>,
    memory: Vec<f32>,
}

impl Card {
    fn new(source: Source, name: String, driver: String, pci: PathBuf) -> Card {
        let file = |f: &str| read(&pci.join(f).to_string_lossy());
        let may_sleep = file("power/control").as_deref() == Some("auto") && file("boot_vga").as_deref() != Some("1");
        Card { source, name, driver, pci, may_sleep, idle_since: None, read_at: None, last: Gpu::default(), usage: Vec::new(), memory: Vec::new() }
    }

    fn asleep(&self) -> bool {
        read(&self.pci.join("power/runtime_status").to_string_lossy()).as_deref() == Some("suspended")
    }

    /// Whether to read it at `now`: always while it works; now and then
    /// once it has sat idle, when it may sleep and is let to.
    fn due(&self, now: Instant, let_sleep: bool) -> bool {
        let resting = let_sleep && self.may_sleep && self.idle_since.is_some_and(|t| now.duration_since(t) >= IDLE_FOR);
        !resting || self.read_at.is_none_or(|t| now.duration_since(t) >= IDLE_LOOK)
    }
}

/// The NVIDIA cards on the PCI bus that its own driver drives (not
/// nouveau), by address.
fn nvidia_cards(ids: &str) -> Vec<Card> {
    let mut found: Vec<(String, PathBuf)> = fs::read_dir("/sys/bus/pci/devices")
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let dir = e.path();
            let file = |f: &str| read(&dir.join(f).to_string_lossy()).unwrap_or_default();
            // 0x03: a display controller.
            let ours = file("vendor") == NVIDIA && file("class").starts_with("0x03") && fs::read_link(dir.join("driver")).is_ok_and(|d| d.file_name().is_some_and(|n| n == "nvidia"));
            ours.then(|| (e.file_name().to_string_lossy().into_owned(), dir))
        })
        .collect();
    found.sort();
    found
        .into_iter()
        .map(|(bus, dir)| {
            let device = read(&dir.join("device").to_string_lossy()).unwrap_or_default();
            let name = card_name(ids, "10de", device.trim_start_matches("0x"));
            Card::new(Source::Nvml { bus, index: None }, name, "nvidia".to_owned(), dir)
        })
        .collect()
}

/// The cards in `/sys/class/drm` driven by anything but NVIDIA's driver.
fn sys_cards(ids: &str) -> Vec<Card> {
    let Ok(dir) = fs::read_dir("/sys/class/drm") else { return Vec::new() };
    let mut names: Vec<String> = dir.flatten().filter_map(|e| e.file_name().into_string().ok()).filter(|n| n.strip_prefix("card").is_some_and(|d| !d.is_empty() && d.bytes().all(|b| b.is_ascii_digit()))).collect();
    names.sort();
    names
        .iter()
        .filter_map(|card| {
            let dir = PathBuf::from(format!("/sys/class/drm/{card}/device"));
            let uevent = fs::read_to_string(dir.join("uevent")).ok()?;
            let field = |key: &str| uevent.lines().find_map(|l| l.strip_prefix(key)).unwrap_or("").trim();
            let driver = field("DRIVER=");
            if driver.is_empty() || driver == "nvidia" {
                return None;
            }
            let (vendor, device) = field("PCI_ID=").split_once(':').unwrap_or(("", ""));
            let pci = fs::canonicalize(&dir).unwrap_or_else(|_| dir.clone());
            Some(Card::new(Source::Sys(dir), card_name(ids, vendor, device), driver.to_owned(), pci))
        })
        .collect()
}

/// The first of a card's hwmon folders: where amdgpu and the like put
/// its temperature and power.
fn hwmon(dir: &Path) -> Option<PathBuf> {
    fs::read_dir(dir.join("hwmon")).ok()?.flatten().map(|e| e.path()).next()
}

fn read_sys(dir: &Path) -> Gpu {
    let file = |name: &str| dir.join(name).to_string_lossy().into_owned();
    let hw = hwmon(dir);
    let sensor = |name: &str| hw.as_ref().and_then(|h| read_number::<f64>(&h.join(name).to_string_lossy()));
    Gpu {
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
    /// The library's driver version, once it has said.
    nvml_driver: String,
    /// When the library last wouldn't open.
    nvml_failed: Option<Instant>,
    /// NVIDIA's first, then the rest, in an order that holds.
    cards: Vec<Card>,
    let_sleep: bool,
    /// How many looks the histories keep.
    cap: usize,
}

impl GpuSampler {
    /// Find the cards; their histories keep the last `cap` looks.
    pub fn new(cap: usize) -> Self {
        let ids = pci_ids();
        let mut cards = nvidia_cards(&ids);
        cards.extend(sys_cards(&ids));
        Self { nvml: None, nvml_driver: String::new(), nvml_failed: None, cards, let_sleep: false, cap }
    }

    /// Look at an idle card that may sleep only now and then, so it can.
    pub fn let_sleep(mut self, on: bool) -> Self {
        self.let_sleep = on;
        self
    }

    /// Keep `cap` looks from now on.
    pub fn set_cap(&mut self, cap: usize) {
        self.cap = cap;
        for card in &mut self.cards {
            trim(&mut card.usage, cap);
            trim(&mut card.memory, cap);
        }
    }

    /// Open NVIDIA's library and learn which of its cards is which.
    fn open_nvml(&mut self, now: Instant) {
        if self.nvml_failed.is_some_and(|t| now.duration_since(t) < NVML_RETRY) {
            return;
        }
        let Some(nvml) = Nvml::open() else {
            self.nvml_failed = Some(now);
            return;
        };
        let (names, buses) = (nvml.names(), nvml.buses());
        // A driver that won't say where its cards are lists them in bus
        // order, as they were found.
        let mut order = 0;
        for card in &mut self.cards {
            let Source::Nvml { bus, index } = &mut card.source else { continue };
            *index = buses.iter().position(|b| b == bus).or_else(|| buses.iter().all(String::is_empty).then_some(order)).filter(|i| *i < names.len());
            if let Some(i) = *index {
                card.name.clone_from(&names[i]);
            }
            order += 1;
        }
        self.nvml_driver = nvml.driver().to_owned();
        self.nvml = Some(nvml);
    }

    pub fn sample(&mut self) -> Vec<Gpu> {
        let now = Instant::now();
        let asleep: Vec<bool> = self.cards.iter().map(Card::asleep).collect();
        let due: Vec<bool> = self.cards.iter().zip(&asleep).map(|(c, a)| !a && c.due(now, self.let_sleep)).collect();
        let nvidia = |c: &Card| matches!(c.source, Source::Nvml { .. });
        // Open only for a card awake and due. Let go once no card needs
        // it, where holding it could keep a card from sleeping.
        let wanted = self.cards.iter().zip(&due).any(|(c, d)| *d && nvidia(c));
        if wanted && self.nvml.is_none() {
            self.open_nvml(now);
        } else if !wanted && self.let_sleep && self.nvml.is_some() && self.cards.iter().filter(|c| nvidia(c)).all(|c| c.may_sleep) {
            self.nvml = None;
        }
        let mut gpus = Vec::with_capacity(self.cards.len());
        for (i, card) in self.cards.iter_mut().enumerate() {
            if asleep[i] {
                card.last = Gpu { asleep: true, ..Gpu::default() };
                card.idle_since = None;
            } else if due[i] {
                card.last = match &card.source {
                    Source::Nvml { index: Some(index), .. } => match &self.nvml {
                        Some(nvml) => {
                            let r = nvml.read(*index);
                            Gpu { usage: r.usage, memory: r.memory, celsius: r.celsius, watts: r.watts, watts_limit: r.watts_limit, fans: r.fans, core_mhz: r.core_mhz, memory_mhz: r.memory_mhz, ..Gpu::default() }
                        }
                        None => Gpu::default(),
                    },
                    Source::Nvml { index: None, .. } => Gpu::default(),
                    Source::Sys(dir) => read_sys(dir),
                };
                card.read_at = Some(now);
                let idle = card.last.usage.is_some_and(|u| u < 1.0);
                card.idle_since = if idle { card.idle_since.or(Some(now)) } else { None };
            }
            if matches!(card.source, Source::Nvml { .. }) && !self.nvml_driver.is_empty() {
                card.driver = format!("nvidia {}", self.nvml_driver);
            }
            // Asleep, it is busy with nothing; between looks, as it was.
            let usage = if card.last.asleep { Some(0.0) } else { card.last.usage };
            if let Some(u) = usage {
                push(&mut card.usage, u, self.cap);
            }
            if let Some(m) = card.last.memory_percent() {
                push(&mut card.memory, m, self.cap);
            }
            gpus.push(Gpu { name: card.name.clone(), driver: card.driver.clone(), usage_history: card.usage.clone(), memory_history: card.memory.clone(), ..card.last.clone() });
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
    fn the_bar_shows_the_first_card_that_says_how_busy_it_is() {
        let intel = Gpu { name: "Intel".into(), ..Gpu::default() };
        let nvidia = Gpu { name: "NVIDIA".into(), usage: Some(3.0), ..Gpu::default() };
        let sleeping = Gpu { name: "NVIDIA".into(), asleep: true, ..Gpu::default() };
        assert_eq!(shown(&[intel.clone(), nvidia.clone()]).map(|g| g.usage), Some(Some(3.0)));
        assert!(shown(&[sleeping, intel.clone()]).is_some_and(|g| g.asleep));
        assert_eq!(shown(&[intel]), None, "an Intel-only machine shows none");
    }

    #[test]
    fn an_idle_card_that_may_sleep_is_looked_at_now_and_then() {
        let now = Instant::now();
        let mut card = Card { source: Source::Sys(PathBuf::new()), name: String::new(), driver: String::new(), pci: PathBuf::new(), may_sleep: true, idle_since: None, read_at: Some(now), last: Gpu::default(), usage: Vec::new(), memory: Vec::new() };
        assert!(card.due(now, true), "working: every look");
        card.idle_since = Some(now - IDLE_FOR);
        assert!(!card.due(now, true), "idle a while: rests");
        assert!(card.due(now + IDLE_LOOK, true), "but is looked at now and then");
        assert!(card.due(now, false), "unless sleeping isn't let");
        card.may_sleep = false;
        assert!(card.due(now, true), "a card that can't sleep is always looked at");
    }

    #[test]
    fn this_machines_cards_are_read_and_their_histories_grow() {
        let mut s = GpuSampler::new(8);
        let first = s.sample();
        let again = s.sample();
        assert_eq!(first.len(), again.len());
        for gpu in &again {
            assert!(!gpu.name.is_empty() && !gpu.driver.is_empty());
            // A card that says how busy it is has said so twice now.
            assert!(gpu.usage.is_none() || gpu.asleep || gpu.usage_history.len() == 2);
        }
    }
}

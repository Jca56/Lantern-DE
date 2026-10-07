//! What the machine is, which doesn't change while it runs: its name,
//! what it runs, what is in it. Gathered once, when the sampler starts.

use std::collections::BTreeSet;
use std::fs;

use super::read;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Facts {
    pub hostname: String,
    /// The distribution, as it names itself: "Gentoo Linux".
    pub os: String,
    pub kernel: String,
    pub shell: String,
    pub cpu: String,
    /// Physical cores; zero when the kernel doesn't say.
    pub cores: usize,
    pub threads: usize,
    /// Maker and model of the mainboard; empty when it doesn't say.
    pub board: String,
    /// Each connected screen: "3840×2160 (DP-1)".
    pub displays: Vec<String>,
    /// How many packages are installed, and whose count that is:
    /// `(1203, "Portage")`.
    pub packages: Option<(usize, &'static str)>,
}

/// The value of `key` in an `os-release` file, out of its quotes
/// (either kind: Gentoo writes single ones).
pub fn os_release(text: &str, key: &str) -> Option<String> {
    text.lines().find_map(|l| l.strip_prefix(key)?.strip_prefix('=')).map(|v| v.trim().trim_matches(['"', '\'']).to_owned()).filter(|v| !v.is_empty())
}

/// A processor's name as `/proc/cpuinfo` has it, without the trademark
/// signs and the clock speed it is sold at.
pub fn cpu_model(cpuinfo: &str) -> String {
    let raw = cpuinfo.lines().find(|l| l.starts_with("model name")).and_then(|l| l.split_once(':')).map(|(_, v)| v.trim()).unwrap_or("");
    let name = raw.replace("(R)", "").replace("(TM)", "").replace("(tm)", "").replace(" CPU", "").replace(" Processor", "");
    let name = name.split(" @ ").next().unwrap_or("");
    name.split_ascii_whitespace().collect::<Vec<_>>().join(" ")
}

/// How many cores and threads `/proc/cpuinfo` lists. Cores are told
/// apart by which package and which core in it a thread sits on; a
/// kernel that doesn't say gets zero cores.
pub fn cpu_counts(cpuinfo: &str) -> (usize, usize) {
    let mut cores = BTreeSet::new();
    let (mut threads, mut package) = (0, "");
    for line in cpuinfo.lines() {
        let Some((key, value)) = line.split_once(':') else { continue };
        match key.trim() {
            "processor" => threads += 1,
            "physical id" => package = value.trim(),
            "core id" => {
                cores.insert((package, value.trim()));
            }
            _ => {}
        }
    }
    (cores.len(), threads)
}

/// A mainboard's maker without the company suffixes, then its model.
pub fn board_name(vendor: &str, model: &str) -> String {
    let mut maker = vendor.trim().trim_end_matches('.').trim();
    for suffix in [" CO., LTD", " Co., Ltd", " COMPUTER INC", " Computer Inc", " Corporation", " CORP", " Inc", " INC"] {
        maker = maker.strip_suffix(suffix).unwrap_or(maker).trim_end_matches([',', ' ']);
    }
    format!("{maker} {}", model.trim()).trim().to_owned()
}

/// The screens plugged in, from the connectors under `/sys/class/drm`:
/// the mode each lists first is its own.
fn displays() -> Vec<String> {
    let Ok(dir) = fs::read_dir("/sys/class/drm") else { return Vec::new() };
    let mut found: Vec<String> = dir
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            // "card0-DP-1": the connector's name follows the card's.
            let connector = name.strip_prefix("card")?.split_once('-')?.1.to_owned();
            let file = |f: &str| read(&e.path().join(f).to_string_lossy());
            if file("status")? != "connected" {
                return None;
            }
            let modes = file("modes")?;
            let mode = modes.lines().next()?.trim();
            Some(format!("{} ({connector})", mode.replace('x', "×")))
        })
        .collect();
    found.sort();
    found
}

/// How many packages the distribution's own manager has installed.
fn packages() -> Option<(usize, &'static str)> {
    let folders = |path: &str| fs::read_dir(path).ok().map(|d| d.flatten().filter(|e| e.file_type().is_ok_and(|t| t.is_dir())).map(|e| e.path()).collect::<Vec<_>>());
    // Portage: a folder per category, a folder per package in each.
    if let Some(categories) = folders("/var/db/pkg") {
        let count: usize = categories.iter().filter_map(|c| folders(&c.to_string_lossy())).map(|p| p.len()).sum();
        if count > 0 {
            return Some((count, "Portage"));
        }
    }
    folders("/var/lib/pacman/local").map(|p| (p.len(), "pacman")).filter(|(n, _)| *n > 0)
}

impl Facts {
    pub fn gather() -> Facts {
        let cpuinfo = fs::read_to_string("/proc/cpuinfo").unwrap_or_default();
        let (cores, threads) = cpu_counts(&cpuinfo);
        let dmi = |file: &str| read(&format!("/sys/devices/virtual/dmi/id/{file}")).unwrap_or_default();
        Facts {
            hostname: read("/proc/sys/kernel/hostname").unwrap_or_default(),
            os: os_release(&fs::read_to_string("/etc/os-release").unwrap_or_default(), "PRETTY_NAME").unwrap_or_else(|| "Linux".to_owned()),
            kernel: read("/proc/sys/kernel/osrelease").unwrap_or_default(),
            shell: std::env::var("SHELL").ok().and_then(|s| s.rsplit('/').next().map(str::to_owned)).unwrap_or_default(),
            cpu: cpu_model(&cpuinfo),
            cores,
            threads,
            board: board_name(&dmi("board_vendor"), &dmi("board_name")),
            displays: displays(),
            packages: packages(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_distribution_is_named_out_of_either_kind_of_quotes() {
        let gentoo = "NAME='Gentoo'\nID='gentoo'\nPRETTY_NAME='Gentoo Linux'\n";
        assert_eq!(os_release(gentoo, "PRETTY_NAME").as_deref(), Some("Gentoo Linux"));
        assert_eq!(os_release("PRETTY_NAME=\"Arch Linux\"\n", "PRETTY_NAME").as_deref(), Some("Arch Linux"));
        // `NAME` is not the end of `PRETTY_NAME`, and nothing is nothing.
        assert_eq!(os_release(gentoo, "NAME").as_deref(), Some("Gentoo"));
        assert_eq!(os_release("PRETTY_NAME=\n", "PRETTY_NAME"), None);
    }

    #[test]
    fn a_processor_is_named_plainly_and_its_cores_are_counted_once() {
        assert_eq!(cpu_model("model name\t: Intel(R) Core(TM) i7-14700K\n"), "Intel Core i7-14700K");
        assert_eq!(cpu_model("model name\t: Intel(R) Core(TM) i5-8250U CPU @ 1.60GHz\n"), "Intel Core i5-8250U");
        assert_eq!(cpu_model("model name\t: AMD Ryzen 7 5800X 8-Core Processor\n"), "AMD Ryzen 7 5800X 8-Core");
        assert_eq!(cpu_model(""), "");
        // Two threads on core 0, one on core 1, and core 0 of a second package.
        let info = "processor: 0\nphysical id: 0\ncore id: 0\n\nprocessor: 1\nphysical id: 0\ncore id: 0\n\nprocessor: 2\nphysical id: 0\ncore id: 1\n\nprocessor: 3\nphysical id: 1\ncore id: 0\n";
        assert_eq!(cpu_counts(info), (3, 4));
        assert_eq!(cpu_counts("processor: 0\nprocessor: 1\n"), (0, 2));
    }

    #[test]
    fn a_mainboard_loses_its_company_suffixes() {
        assert_eq!(board_name("ASUSTeK COMPUTER INC.", "ROG STRIX Z790-E"), "ASUSTeK ROG STRIX Z790-E");
        assert_eq!(board_name("Micro-Star International Co., Ltd.", "B550"), "Micro-Star International B550");
        assert_eq!(board_name("", "X570"), "X570");
        assert_eq!(board_name("", ""), "");
    }

    #[test]
    fn this_machine_is_described() {
        let f = Facts::gather();
        assert!(!f.hostname.is_empty() && !f.kernel.is_empty() && !f.os.is_empty());
        assert!(f.threads >= 1 && f.cores <= f.threads);
    }
}

//! Request dispatch for lntrn-afterglowd: owns the hardware handles and
//! turns protocol lines into NVML/sysfs calls.

use lntrn_afterglow::hw::cpu::Cpu;
use lntrn_afterglow::hw::nvml::{Nvml, CLOCK_GRAPHICS, CLOCK_MEM};
use lntrn_afterglow::hw::rapl::{Limit, Rapl};
use lntrn_afterglow::profile::Profile;
use lntrn_afterglow::protocol::{err, Fields};

pub struct Hw {
    pub nvml: Option<Nvml>,
    pub rapl: Option<Rapl>,
    pub cpu: Cpu,
}

impl Hw {
    pub fn init() -> Hw {
        let nvml = match Nvml::load() {
            Ok(n) => n,
            Err(e) => {
                eprintln!("afterglowd: NVML unavailable: {e}");
                None
            }
        };
        Hw {
            nvml,
            rapl: Rapl::discover(),
            cpu: Cpu::discover(),
        }
    }

    pub fn handle(&mut self, line: &str) -> String {
        let mut parts = line.split_whitespace();
        match parts.next() {
            Some("ping") => "ok".to_string(),
            Some("info") => self.info_line(),
            Some("snap") => self.snap_line(),
            Some("cpu_tuning") => self.cpu_tuning_line(),
            Some("reset") => match &self.nvml {
                Some(n) => done(n.reset_all()),
                None => err("no NVIDIA GPU"),
            },
            Some("set") => self.handle_set(&mut parts),
            _ => err("unknown command"),
        }
    }

    fn gpu(&self) -> Result<&Nvml, String> {
        self.nvml.as_deref_err()
    }

    fn handle_set(&mut self, parts: &mut std::str::SplitWhitespace) -> String {
        let Some(key) = parts.next() else {
            return err("set: missing key");
        };
        let arg = parts.next().unwrap_or("");
        let num_i32 = || arg.parse::<i32>().map_err(|_| format!("bad value: {arg}"));
        let num_u32 = || arg.parse::<u32>().map_err(|_| format!("bad value: {arg}"));
        let result: Result<(), String> = match key {
            "core_offset" => {
                num_i32().and_then(|v| self.gpu()?.set_clock_offset(CLOCK_GRAPHICS, v))
            }
            "mem_offset" => num_i32().and_then(|v| self.gpu()?.set_clock_offset(CLOCK_MEM, v)),
            "power_limit" => num_u32().and_then(|v| self.gpu()?.set_power_limit_w(v)),
            "fan" => {
                if arg == "auto" {
                    self.gpu().and_then(|g| g.set_fans_auto())
                } else {
                    num_u32().and_then(|v| self.gpu()?.set_fan_pct(v))
                }
            }
            "lock" => num_u32().and_then(|v| self.gpu()?.lock_core_clocks(0, v)),
            "unlock" => self.gpu().and_then(|g| g.unlock_core_clocks()),
            "pl1" => num_u32().and_then(|v| self.set_rapl(Limit::Pl1, v)),
            "pl2" => num_u32().and_then(|v| self.set_rapl(Limit::Pl2, v)),
            "governor" => self.cpu.set_governor(arg),
            "epp" => self.cpu.set_epp(arg),
            "turbo" => self.cpu.set_no_turbo(arg == "0"),
            "max_freq_pct" => num_u32().and_then(|v| self.cpu.set_max_freq_pct(v)),
            _ => Err(format!("unknown setting: {key}")),
        };
        done(result)
    }

    fn set_rapl(&self, limit: Limit, watts: u32) -> Result<(), String> {
        self.rapl
            .as_ref()
            .ok_or("RAPL not available".to_string())?
            .set_limit(limit, watts as f64)
    }

    fn info_line(&self) -> String {
        let mut f = Fields::new();
        f.push("root", (unsafe { libc::geteuid() } == 0) as u8);
        f.push("cpu_model", &self.cpu.model);
        match &self.nvml {
            Some(n) => match n.info() {
                Ok(i) => {
                    f.push("gpu", 1);
                    f.push("name", &i.name);
                    f.push("driver", &i.driver);
                    f.push("pmin", i.power_min_w);
                    f.push("pmax", i.power_max_w);
                    f.push("pdef", i.power_default_w);
                    f.push("core_off", i.core_offset.current);
                    f.push("core_min", i.core_offset.min);
                    f.push("core_max", i.core_offset.max);
                    f.push("mem_off", i.mem_offset.current);
                    f.push("mem_min", i.mem_offset.min);
                    f.push("mem_max", i.mem_offset.max);
                    f.push("fans", i.fan_count);
                }
                Err(e) => return err(e),
            },
            None => f.push("gpu", 0),
        }
        match &self.rapl {
            Some(r) => {
                let l = r.limits();
                f.push("rapl", 1);
                f.push("pl1", l.pl1_w.unwrap_or(0.0).round() as u64);
                f.push("pl2", l.pl2_w.unwrap_or(0.0).round() as u64);
                f.push("pl1_max", l.pl1_max_w.unwrap_or(0.0).round() as u64);
                f.push("pl2_max", l.pl2_max_w.unwrap_or(0.0).round() as u64);
            }
            None => f.push("rapl", 0),
        }
        f.into_ok()
    }

    fn snap_line(&mut self) -> String {
        let mut f = Fields::new();
        if let Some(n) = &self.nvml {
            match n.snapshot() {
                Ok(s) => {
                    f.push("temp", s.temp_c);
                    f.push("power", format!("{:.1}", s.power_w));
                    f.push("core", s.core_mhz);
                    f.push("mem", s.mem_mhz);
                    f.push("util", s.util_pct);
                    f.push("vram_used", s.vram_used_mb);
                    f.push("vram_total", s.vram_total_mb);
                    f.push("plimit", format!("{:.0}", s.power_limit_w));
                    let fans: Vec<String> = s.fans_pct.iter().map(|p| p.to_string()).collect();
                    f.push("fan", fans.join(","));
                }
                Err(e) => return err(e),
            }
        }
        let cs = self.cpu.snapshot();
        f.push("cpu_util", format!("{:.1}", cs.total_util_pct));
        if let Some(t) = cs.package_temp_c {
            f.push("cpu_temp", format!("{t:.1}"));
        }
        if let Some(w) = self.rapl.as_mut().and_then(|r| r.package_watts()) {
            f.push("pkg_w", format!("{w:.1}"));
        }
        let cores: Vec<String> = cs
            .cores
            .iter()
            .map(|c| format!("{}:{}:{:.0}:{}", c.id, c.mhz, c.util_pct, c.is_pcore as u8))
            .collect();
        f.push("cores", cores.join(","));
        f.into_ok()
    }

    fn cpu_tuning_line(&self) -> String {
        let t = self.cpu.tuning();
        let mut f = Fields::new();
        f.push("governor", &t.governor);
        f.push("governors", t.governors.join(","));
        f.push("epp", t.epp.unwrap_or_default());
        f.push("epps", t.epps.join(","));
        if let Some(nt) = t.no_turbo {
            f.push("turbo", (!nt) as u8);
        }
        f.push("max_freq_pct", t.max_freq_pct);
        f.into_ok()
    }

    /// Startup re-apply of a profile the user marked stable. Best-effort:
    /// collects errors instead of bailing so one bad knob doesn't block the
    /// rest.
    pub fn apply_profile(&mut self, p: &Profile) -> Vec<String> {
        let mut errors = Vec::new();
        let mut run = |what: &str, r: Result<(), String>| {
            if let Err(e) = r {
                errors.push(format!("{what}: {e}"));
            }
        };
        if let Some(n) = &self.nvml {
            run(
                "core_offset",
                n.set_clock_offset(CLOCK_GRAPHICS, p.core_offset),
            );
            run("mem_offset", n.set_clock_offset(CLOCK_MEM, p.mem_offset));
            if p.power_limit_w > 0 {
                run("power_limit", n.set_power_limit_w(p.power_limit_w));
            }
            if p.fan_manual {
                run("fan", n.set_fan_pct(p.fan_pct));
            }
            if p.lock_enabled && p.lock_mhz > 0 {
                run("lock", n.lock_core_clocks(0, p.lock_mhz));
            }
        }
        if p.pl1_w > 0 {
            run("pl1", self.set_rapl(Limit::Pl1, p.pl1_w));
        }
        if p.pl2_w > 0 {
            run("pl2", self.set_rapl(Limit::Pl2, p.pl2_w));
        }
        if !p.governor.is_empty() {
            run("governor", self.cpu.set_governor(&p.governor));
        }
        if !p.epp.is_empty() {
            run("epp", self.cpu.set_epp(&p.epp));
        }
        if p.turbo >= 0 {
            run("turbo", self.cpu.set_no_turbo(p.turbo == 0));
        }
        if p.max_freq_pct < 100 {
            run("max_freq_pct", self.cpu.set_max_freq_pct(p.max_freq_pct));
        }
        errors
    }
}

fn done(r: Result<(), String>) -> String {
    match r {
        Ok(()) => "ok".to_string(),
        Err(e) => err(e),
    }
}

trait OptNvml {
    fn as_deref_err(&self) -> Result<&Nvml, String>;
}

impl OptNvml for Option<Nvml> {
    fn as_deref_err(&self) -> Result<&Nvml, String> {
        self.as_ref().ok_or_else(|| "no NVIDIA GPU".to_string())
    }
}

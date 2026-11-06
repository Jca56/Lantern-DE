//! What the helper keeps: the hardware, its limits, the tune the machine
//! started in, the one applied now, and the trial that undoes itself.
//!
//! A tune is always written whole, in an order that keeps the machine
//! safe on the way: the fans first, the power limits before the clocks.
//! What is applied is then read back from the hardware wherever the
//! hardware can say it, so a knob that wouldn't take shows as it is.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use lntrn_afterglowd::hw::cpu::Cpu;
use lntrn_afterglowd::hw::nvml::{Clock, Nvml};
use lntrn_afterglowd::hw::rapl::{Limit, Rapl};
use lntrn_afterglowd::tune::{CpuTune, Fan, GpuTune, Limits, Tune};
use lntrn_afterglowd::wire::Status;
use lntrn_data::{Doc, json};

use super::fan::{FanLoop, Step};
use super::log::say;

/// The least a package power limit may be set to, watts: a desktop part
/// below this is barely running.
const PACKAGE_FLOOR: u32 = 35;

/// A tune on trial: what goes back if it isn't kept, and when.
struct Trial {
    fallback: Tune,
    until: Instant,
}

pub struct Daemon {
    nvml: Option<Nvml>,
    cpu: Cpu,
    rapl: Option<Rapl>,
    pub limits: Limits,
    applied: Tune,
    trial: Option<Trial>,
    reverted: u64,
    fan: FanLoop,
    package_watts: Option<f32>,
    /// Say what would be done, and do none of it.
    pub dry_run: bool,
}

/// Where the stock tune is kept between starts of the helper in one boot
/// (`/run` is emptied at every boot), so a restart after a change doesn't
/// take the change for stock.
fn stock_file(state: &Path) -> PathBuf {
    state.join("stock.json")
}

fn boot_id() -> String {
    std::fs::read_to_string("/proc/sys/kernel/random/boot_id").map(|s| s.trim().to_owned()).unwrap_or_default()
}

/// The stock tune saved earlier in this boot.
fn saved_stock(state: &Path) -> Option<Tune> {
    let doc = json::parse(&std::fs::read_to_string(stock_file(state)).ok()?).ok()?;
    if doc.get("boot").and_then(Doc::as_str) != Some(&boot_id()) {
        return None;
    }
    Tune::from_doc(doc.get("tune")?).ok()
}

fn save_stock(state: &Path, stock: &Tune) {
    let mut m = lntrn_data::Map::new();
    m.insert("boot", Doc::Str(boot_id()));
    m.insert("tune", stock.to_doc());
    let saved = std::fs::create_dir_all(state).and_then(|()| std::fs::write(stock_file(state), json::write(&Doc::Map(m))));
    if let Err(e) = saved {
        say(&format!("couldn't keep the stock settings in {}: {e}", state.display()));
    }
}

impl Daemon {
    /// Find the hardware and read where it stands. `state` is where the
    /// stock tune is kept (none when running as a user, for a test).
    pub fn start(state: Option<&Path>, dry_run: bool) -> Daemon {
        let nvml = open_nvml();
        let cpu = Cpu::discover();
        let rapl = Rapl::discover();
        let mut d = Daemon { nvml, cpu, rapl, limits: Limits { gpu: None, cpu: Default::default(), stock: placeholder() }, applied: placeholder(), trial: None, reverted: 0, fan: FanLoop::default(), package_watts: None, dry_run };
        let found = d.read_back(None);
        let stock = match state.and_then(saved_stock) {
            Some(saved) if saved.gpu.is_some() == found.gpu.is_some() => saved,
            _ => {
                // The graphics card's stock is its driver's own defaults,
                // whatever it is set to now; the processor's is how it is.
                let gpu = d.nvml.as_ref().map(|n| GpuTune { core_offset: 0, memory_offset: 0, power_limit: n.limits().power_default, lock: None, fan: Fan::Auto });
                let stock = Tune { gpu, cpu: found.cpu.clone() };
                if let Some(state) = state {
                    save_stock(state, &stock);
                }
                stock
            }
        };
        let mut cpu_limits = d.cpu.limits();
        if d.rapl.is_some() {
            // Never past what the firmware set: on recent Intel parts more
            // than that is how chips get hurt.
            let ceiling = stock.cpu.pl1.max(stock.cpu.pl2).max(PACKAGE_FLOOR);
            cpu_limits.power = Some((PACKAGE_FLOOR, ceiling));
        }
        d.limits = Limits { gpu: d.nvml.as_ref().map(Nvml::limits), cpu: cpu_limits, stock };
        d.applied = found;
        d
    }

    /// The tune the hardware is in now. Where the hardware can't say
    /// (a clock lock, the fans' mode), `asked` does: what was last set.
    fn read_back(&self, asked: Option<&Tune>) -> Tune {
        let gpu = self.nvml.as_ref().map(|n| {
            let (core_offset, memory_offset) = n.offsets();
            let asked = asked.and_then(|t| t.gpu.as_ref());
            GpuTune { core_offset, memory_offset, power_limit: n.power_limit().unwrap_or(0), lock: asked.and_then(|g| g.lock), fan: asked.map_or(Fan::Auto, |g| g.fan.clone()) }
        });
        let s = self.cpu.settings();
        // The performance governor forces its own preference; the one
        // asked for is kept for when the governor changes back.
        let epp = match asked {
            Some(t) if s.governor == "performance" => t.cpu.epp.clone(),
            _ => s.epp,
        };
        let limit = |l| self.rapl.as_ref().and_then(|r| r.limit(l)).unwrap_or(0);
        Tune { gpu, cpu: CpuTune { governor: s.governor, epp, turbo: s.turbo, max_percent: s.max_percent, pl1: limit(Limit::Sustained), pl2: limit(Limit::Boost) } }
    }

    /// Set every knob of `tune`, in an order that is safe on the way.
    /// What wouldn't take, each said plainly.
    fn write(&mut self, tune: &Tune) -> Vec<String> {
        let mut problems = Vec::new();
        let dry = self.dry_run;
        let mut run = |what: &str, r: &mut dyn FnMut() -> Result<(), String>| {
            if dry {
                say(&format!("(dry run) would set {what}"));
            } else if let Err(e) = r() {
                problems.push(format!("{what}: {e}"));
            }
        };
        if let (Some(n), Some(g)) = (&self.nvml, &tune.gpu) {
            // Fans first, so a hot card is never left waiting on them.
            match &g.fan {
                Fan::Auto => {
                    self.fan.stop();
                    run("fans to automatic", &mut || n.set_fans(None));
                }
                Fan::Fixed(p) => {
                    self.fan.stop();
                    run(&format!("fans to {p}%"), &mut || n.set_fans(Some(*p)));
                }
                Fan::Curve(points) => self.fan.start(points.clone()),
            }
            run(&format!("power limit to {} W", g.power_limit), &mut || n.set_power_limit(g.power_limit));
            let lock = g.lock.map_or_else(|| "the core clock free".to_owned(), |mhz| format!("the core clock held at {mhz} MHz"));
            run(&lock, &mut || n.set_lock(g.lock));
            run(&format!("core offset to {:+} MHz", g.core_offset), &mut || n.set_offset(Clock::Core, g.core_offset));
            run(&format!("memory offset to {:+} MHz", g.memory_offset), &mut || n.set_offset(Clock::Memory, g.memory_offset));
        }
        let c = &tune.cpu;
        let cpu = &self.cpu;
        if !c.governor.is_empty() {
            run(&format!("governor to {}", c.governor), &mut || cpu.set_governor(&c.governor));
        }
        if !c.epp.is_empty() && c.governor != "performance" {
            run(&format!("energy preference to {}", c.epp), &mut || cpu.set_epp(&c.epp));
        }
        if self.limits.cpu.turbo {
            run(&format!("turbo {}", if c.turbo { "on" } else { "off" }), &mut || cpu.set_turbo(c.turbo));
        }
        run(&format!("top speed to {}%", c.max_percent), &mut || cpu.set_max_percent(c.max_percent));
        if let Some(rapl) = &self.rapl {
            // Raising, the boost limit goes first; lowering, the sustained
            // one: the pair never crosses on the way.
            let raising = c.pl1 > rapl.limit(Limit::Sustained).unwrap_or(0);
            let order = if raising { [Limit::Boost, Limit::Sustained] } else { [Limit::Sustained, Limit::Boost] };
            for limit in order {
                let (name, watts) = if limit == Limit::Sustained { ("sustained power", c.pl1) } else { ("boost power", c.pl2) };
                run(&format!("{name} to {watts} W"), &mut || rapl.set(limit, watts));
            }
        }
        // A curve's first speed now rather than in a second.
        if self.fan.running() {
            self.follow_curve();
        }
        problems
    }

    fn put(&mut self, tune: &Tune) -> Vec<String> {
        let problems = self.write(tune);
        self.applied = if self.dry_run { tune.clone() } else { self.read_back(Some(tune)) };
        for p in &problems {
            say(p);
        }
        problems
    }

    /// Apply `tune`, as a trial that undoes itself unless kept, or for
    /// good. A trial over a trial goes back to what came before both.
    pub fn apply(&mut self, tune: Tune, trial: Option<Duration>) -> Result<Vec<String>, String> {
        self.limits.check(&tune)?;
        let fallback = self.trial.take().map_or_else(|| self.applied.clone(), |t| t.fallback);
        let problems = self.put(&tune);
        self.trial = trial.map(|after| Trial { fallback, until: Instant::now() + after });
        Ok(problems)
    }

    pub fn keep(&mut self) {
        if self.trial.take().is_some() {
            say("the trial was kept");
        }
    }

    pub fn revert(&mut self) -> Vec<String> {
        match self.trial.take() {
            Some(t) => self.put(&t.fallback),
            None => Vec::new(),
        }
    }

    pub fn reset(&mut self) -> Vec<String> {
        self.trial = None;
        let stock = self.limits.stock.clone();
        self.put(&stock)
    }

    /// Set the speed the curve gives for the card's heat now.
    fn follow_curve(&mut self) {
        let (Some(n), Some(l)) = (&self.nvml, &self.limits.gpu) else { return };
        let Some(range) = l.fan_range else { return };
        let dry = self.dry_run;
        let set = match self.fan.step(n.celsius(), range, l.slowdown) {
            Step::Hold => return,
            Step::Set(p) if dry => return say(&format!("(dry run) would set the fans to {p}%")),
            Step::Set(p) => n.set_fans(Some(p)),
            Step::Auto if dry => Ok(()),
            Step::Auto => n.set_fans(None),
        };
        if let Err(e) = set {
            self.fan.fail(format!("The fan curve stopped: {e}. The fans are back on the card's own curve."));
            if !dry {
                let _ = n.set_fans(None);
            }
        }
        // It gave up this step: say so, and say the fans are the card's.
        if !self.fan.running()
            && let Some(why) = &self.fan.trouble
        {
            say(why);
            if let Some(g) = &mut self.applied.gpu {
                g.fan = Fan::Auto;
            }
        }
    }

    /// Stop the curve and give the fans back to the card: what is left to
    /// do when a tick went wrong.
    pub fn fans_to_card(&mut self) {
        if !self.fan.running() {
            return;
        }
        self.fan.fail("Something went wrong following the fan curve, so the fans are back on the card's own curve.".to_owned());
        if let Some(g) = &mut self.applied.gpu {
            g.fan = Fan::Auto;
        }
        if let Some(n) = &self.nvml
            && !self.dry_run
        {
            let _ = n.set_fans(None);
        }
    }

    /// Once a second: run out a trial that wasn't kept, follow the fan
    /// curve, and measure what the package draws.
    pub fn tick(&mut self) {
        if self.trial.as_ref().is_some_and(|t| Instant::now() >= t.until) {
            say("a trial wasn't kept in time: going back");
            self.revert();
            self.reverted += 1;
        }
        if self.fan.running() {
            self.follow_curve();
        }
        self.package_watts = self.rapl.as_mut().and_then(Rapl::watts);
    }

    pub fn status(&self) -> Status {
        Status {
            applied: self.applied.clone(),
            trial: self.trial.as_ref().map(|t| t.until.saturating_duration_since(Instant::now()).as_secs_f64().ceil() as u64),
            reverted: self.reverted,
            package_watts: self.package_watts,
            fan_percent: self.fan.percent(),
            trouble: self.fan.trouble.clone(),
        }
    }

    /// Everything back to stock, fans first: the helper is going away and
    /// nothing will follow the curve or end a trial.
    pub fn shut_down(&mut self) {
        self.fan.stop();
        let problems = self.reset();
        say(if problems.is_empty() { "back to stock" } else { "back to stock, but not all of it took" });
    }
}

/// NVIDIA's library. At boot the driver can still be settling, so a card
/// the driver lists but won't start yet gets a few seconds.
fn open_nvml() -> Option<Nvml> {
    for attempt in 0..10 {
        match Nvml::open() {
            Ok(n) => {
                say(&format!("found {}", n.name()));
                return Some(n);
            }
            Err(e) if e == "no NVIDIA driver" || attempt == 9 => {
                say(&format!("no graphics card to tune: {e}"));
                return None;
            }
            Err(_) => std::thread::sleep(Duration::from_secs(1)),
        }
    }
    None
}

/// A tune that stands in until the real ones are read.
fn placeholder() -> Tune {
    Tune { gpu: None, cpu: CpuTune { governor: String::new(), epp: String::new(), turbo: true, max_percent: 100, pl1: 0, pl2: 0 } }
}

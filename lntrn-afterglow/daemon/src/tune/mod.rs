//! What Afterglow sets, and how far it may go.
//!
//! A [`Tune`] is every knob at once: the window edits one, a profile is
//! one, and the helper applies one whole (a trial's undo is the tune
//! before it). [`Limits`] are this machine's bounds for each knob, read
//! by the helper at start, and the tune the machine started in (stock).
//! [`Limits::check`] is the one judge of a tune: the helper refuses what
//! it refuses, and the window asks it first so it can say why. `doc`
//! turns them into documents (JSON on the wire, TOML for profiles) and back.

mod doc;

/// The most points a fan curve may have.
pub const MAX_POINTS: usize = 8;
/// The coolest and hottest a curve's points may sit, °C.
pub const CURVE_CELSIUS: (u32, u32) = (20, 100);

/// One point of a fan curve: at this heat, this speed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Point {
    pub celsius: u32,
    pub percent: u32,
}

impl Point {
    pub const fn new(celsius: u32, percent: u32) -> Point {
        Point { celsius, percent }
    }
}

/// How the card's fans are driven.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Fan {
    /// The card's own curve.
    Auto,
    /// One speed, percent, whatever the heat.
    Fixed(u32),
    /// Afterglow's curve, followed by the helper once a second.
    Curve(Vec<Point>),
}

/// The speed `points` give at `celsius`: along the line between the
/// points either side, the first point's below them all and the last's
/// above.
pub fn curve_at(points: &[Point], celsius: f64) -> f64 {
    let (Some(first), Some(last)) = (points.first(), points.last()) else { return 0.0 };
    if celsius <= f64::from(first.celsius) {
        return f64::from(first.percent);
    }
    for pair in points.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        if celsius <= f64::from(b.celsius) {
            let span = f64::from(b.celsius) - f64::from(a.celsius);
            let t = if span > 0.0 { (celsius - f64::from(a.celsius)) / span } else { 1.0 };
            return f64::from(a.percent) + t * (f64::from(b.percent) - f64::from(a.percent));
        }
    }
    f64::from(last.percent)
}

/// The graphics card's knobs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GpuTune {
    /// MHz added to every point of the core's clock curve.
    pub core_offset: i32,
    /// MHz added to the memory's clock.
    pub memory_offset: i32,
    /// The most it may draw, watts.
    pub power_limit: u32,
    /// The fastest the core may run, MHz: with a positive offset, the
    /// same clock at less voltage (how Linux undervolts). `None` lets it
    /// boost as it likes.
    pub lock: Option<u32>,
    pub fan: Fan,
}

/// The processor's knobs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CpuTune {
    pub governor: String,
    /// Energy-performance preference; empty where the driver has none.
    pub epp: String,
    pub turbo: bool,
    /// The fastest each core may run, percent of its own fastest.
    pub max_percent: u32,
    /// Sustained and boost package power limits, watts. `0` where the
    /// machine has none to set.
    pub pl1: u32,
    pub pl2: u32,
}

/// Every knob at once.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tune {
    /// `None` on a machine without an NVIDIA card.
    pub gpu: Option<GpuTune>,
    pub cpu: CpuTune,
}

/// How far the graphics card's knobs go.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GpuLimits {
    pub name: String,
    pub driver: String,
    pub core_offset: (i32, i32),
    pub memory_offset: (i32, i32),
    pub power: (u32, u32),
    pub power_default: u32,
    /// The lowest and highest core clocks it can be held to. `None` when
    /// the driver can't hold it.
    pub lock: Option<(u32, u32)>,
    pub fans: u32,
    /// The slowest and fastest its fans may be set, percent. `None` when
    /// they can't be set.
    pub fan_range: Option<(u32, u32)>,
    /// Where the card starts slowing itself for heat, °C.
    pub slowdown: u32,
}

/// How far the processor's knobs go.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CpuLimits {
    pub model: String,
    pub governors: Vec<String>,
    pub epps: Vec<String>,
    /// Whether turbo can be switched.
    pub turbo: bool,
    /// The fastest a performance core and an efficiency core run, MHz
    /// (`0` for a kind it doesn't have).
    pub pcore_mhz: u32,
    pub ecore_mhz: u32,
    /// The least and most the package power limits may be set to, watts.
    /// `None` without them.
    pub power: Option<(u32, u32)>,
}

/// This machine's bounds, and where it started.
#[derive(Clone, Debug, PartialEq)]
pub struct Limits {
    pub gpu: Option<GpuLimits>,
    pub cpu: CpuLimits,
    /// The tune the machine came up in: what Reset goes back to.
    pub stock: Tune,
}

fn within<T: PartialOrd + std::fmt::Display>(what: &str, v: T, (lo, hi): (T, T), unit: &str) -> Result<(), String> {
    if v < lo || v > hi { Err(format!("{what} {v}{unit} is outside {lo}{unit} to {hi}{unit}")) } else { Ok(()) }
}

impl Limits {
    /// Whether `tune` is one this machine can take: every number in its
    /// range, every word one the driver offers, a fan curve that only
    /// climbs. The first thing wrong, said plainly, when it isn't.
    pub fn check(&self, tune: &Tune) -> Result<(), String> {
        match (&tune.gpu, &self.gpu) {
            (Some(g), Some(l)) => check_gpu(g, l)?,
            (Some(_), None) => return Err("there is no NVIDIA card to tune".to_owned()),
            (None, Some(_)) => return Err("the graphics card's settings are missing".to_owned()),
            (None, None) => {}
        }
        let (c, l) = (&tune.cpu, &self.cpu);
        if !l.governors.is_empty() && !l.governors.contains(&c.governor) {
            return Err(format!("the governor \"{}\" isn't one this processor has", c.governor));
        }
        if !c.epp.is_empty() && !l.epps.contains(&c.epp) {
            return Err(format!("the energy preference \"{}\" isn't one this processor has", c.epp));
        }
        within("Top speed", c.max_percent, (10, 100), "%")?;
        match l.power {
            Some(range) => {
                within("Sustained power", c.pl1, range, " W")?;
                within("Boost power", c.pl2, range, " W")?;
                if c.pl2 < c.pl1 {
                    return Err(format!("boost power ({} W) is below sustained power ({} W)", c.pl2, c.pl1));
                }
            }
            None if c.pl1 != 0 || c.pl2 != 0 => return Err("this processor has no power limits to set".to_owned()),
            None => {}
        }
        Ok(())
    }
}

fn check_gpu(g: &GpuTune, l: &GpuLimits) -> Result<(), String> {
    within("Core offset", g.core_offset, l.core_offset, " MHz")?;
    within("Memory offset", g.memory_offset, l.memory_offset, " MHz")?;
    within("Power limit", g.power_limit, l.power, " W")?;
    if let Some(mhz) = g.lock {
        let Some(range) = l.lock else { return Err("this driver can't hold the core clock".to_owned()) };
        within("Core clock lock", mhz, range, " MHz")?;
    }
    let fans = || l.fan_range.ok_or_else(|| "this card's fans can't be set".to_owned());
    match &g.fan {
        Fan::Auto => {}
        Fan::Fixed(percent) => within("Fan speed", *percent, fans()?, "%")?,
        Fan::Curve(points) => {
            let range = fans()?;
            if points.len() < 2 || points.len() > MAX_POINTS {
                return Err(format!("a fan curve needs 2 to {MAX_POINTS} points, not {}", points.len()));
            }
            for p in points {
                within("A curve point's heat", p.celsius, CURVE_CELSIUS, " °C")?;
                within("A curve point's speed", p.percent, range, "%")?;
            }
            for pair in points.windows(2) {
                if pair[1].celsius <= pair[0].celsius {
                    return Err("a fan curve's points must go from cool to hot".to_owned());
                }
                if pair[1].percent < pair[0].percent {
                    return Err("a fan curve can't slow down as it gets hotter".to_owned());
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// A machine like the one Afterglow was built on.
    pub fn limits() -> Limits {
        let stock = Tune {
            gpu: Some(GpuTune { core_offset: 0, memory_offset: 0, power_limit: 400, lock: None, fan: Fan::Auto }),
            cpu: CpuTune { governor: "powersave".into(), epp: "balance_performance".into(), turbo: true, max_percent: 100, pl1: 253, pl2: 253 },
        };
        Limits {
            gpu: Some(GpuLimits { name: "NVIDIA GeForce RTX 3080 Ti".into(), driver: "615.78.08".into(), core_offset: (-1000, 1000), memory_offset: (-2000, 6000), power: (100, 450), power_default: 400, lock: Some((210, 2100)), fans: 2, fan_range: Some((30, 100)), slowdown: 95 }),
            cpu: CpuLimits {
                model: "Intel(R) Core(TM) i7-14700K".into(),
                governors: vec!["performance".into(), "powersave".into()],
                epps: ["default", "performance", "balance_performance", "balance_power", "power"].map(String::from).to_vec(),
                turbo: true,
                pcore_mhz: 5500,
                ecore_mhz: 4300,
                power: Some((35, 253)),
            },
            stock,
        }
    }

    fn curve(points: &[(u32, u32)]) -> Tune {
        let mut t = limits().stock;
        t.gpu.as_mut().unwrap().fan = Fan::Curve(points.iter().map(|&(c, p)| Point::new(c, p)).collect());
        t
    }

    #[test]
    fn the_stock_tune_passes_and_each_knob_is_held_to_its_range() {
        let l = limits();
        assert_eq!(l.check(&l.stock), Ok(()));
        let with = |f: &dyn Fn(&mut Tune)| {
            let mut t = l.stock.clone();
            f(&mut t);
            l.check(&t)
        };
        assert!(with(&|t| t.gpu.as_mut().unwrap().core_offset = 1200).unwrap_err().contains("Core offset 1200 MHz"));
        assert!(with(&|t| t.gpu.as_mut().unwrap().power_limit = 451).is_err());
        assert!(with(&|t| t.gpu.as_mut().unwrap().lock = Some(5000)).is_err());
        assert_eq!(with(&|t| t.gpu.as_mut().unwrap().lock = Some(1905)), Ok(()));
        assert!(with(&|t| t.gpu.as_mut().unwrap().fan = Fan::Fixed(10)).is_err());
        assert!(with(&|t| t.cpu.governor = "schedutil".into()).unwrap_err().contains("schedutil"));
        assert!(with(&|t| t.cpu.epp = "turbo pls".into()).is_err());
        assert!(with(&|t| t.cpu.max_percent = 5).is_err());
        assert!(with(&|t| t.cpu.pl1 = 300).is_err());
        assert!(with(&|t| t.cpu.pl2 = 100).unwrap_err().contains("below sustained"));
        assert!(with(&|t| t.gpu = None).is_err());
        // An empty preference is "this driver has none" and always fine.
        assert_eq!(with(&|t| t.cpu.epp.clear()), Ok(()));
    }

    #[test]
    fn a_curve_only_climbs_from_cool_to_hot() {
        let l = limits();
        assert_eq!(l.check(&curve(&[(40, 30), (60, 50), (80, 100)])), Ok(()));
        assert!(l.check(&curve(&[(40, 30)])).is_err(), "one point");
        assert!(l.check(&curve(&[(60, 30), (40, 50)])).is_err(), "backwards");
        assert!(l.check(&curve(&[(40, 30), (40, 50)])).is_err(), "two at one heat");
        assert!(l.check(&curve(&[(40, 60), (60, 50)])).unwrap_err().contains("slow down"));
        assert!(l.check(&curve(&[(40, 20), (60, 50)])).is_err(), "below the fans' least");
        assert!(l.check(&curve(&[(10, 30), (60, 50)])).is_err(), "too cool");
        let nine: Vec<(u32, u32)> = (0..9).map(|i| (30 + i * 5, 40)).collect();
        assert!(l.check(&curve(&nine)).is_err(), "too many");
    }

    #[test]
    fn a_curve_is_read_between_its_points_and_held_past_its_ends() {
        let p = [Point::new(40, 30), Point::new(60, 50), Point::new(80, 100)];
        assert_eq!(curve_at(&p, 20.0), 30.0);
        assert_eq!(curve_at(&p, 50.0), 40.0);
        assert_eq!(curve_at(&p, 70.0), 75.0);
        assert_eq!(curve_at(&p, 95.0), 100.0);
        assert_eq!(curve_at(&[], 50.0), 0.0);
    }
}

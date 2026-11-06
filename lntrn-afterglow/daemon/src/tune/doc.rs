//! Tunes and limits as documents and back: JSON on the wire, TOML in a
//! profile. Reading is strict, since the helper reads what any member of
//! the group sends it: a missing field, a word for a number or a number
//! out of its type's range is refused, saying which.

use lntrn_data::{Doc, Map};

use super::{CpuLimits, CpuTune, Fan, GpuLimits, GpuTune, Limits, MAX_POINTS, Point, Tune};

fn field<'a>(d: &'a Doc, key: &str) -> Result<&'a Doc, String> {
    d.get(key).ok_or_else(|| format!("missing \"{key}\""))
}

fn int<T: TryFrom<i64>>(d: &Doc, key: &str) -> Result<T, String> {
    let v = field(d, key)?.as_i64().ok_or_else(|| format!("\"{key}\" isn't a whole number"))?;
    T::try_from(v).map_err(|_| format!("\"{key}\" is out of range"))
}

fn text(d: &Doc, key: &str) -> Result<String, String> {
    field(d, key)?.as_str().map(str::to_owned).ok_or_else(|| format!("\"{key}\" isn't text"))
}

fn flag(d: &Doc, key: &str) -> Result<bool, String> {
    field(d, key)?.as_bool().ok_or_else(|| format!("\"{key}\" isn't true or false"))
}

fn words(d: &Doc, key: &str) -> Result<Vec<String>, String> {
    let list = field(d, key)?.as_list().ok_or_else(|| format!("\"{key}\" isn't a list"))?;
    list.iter().map(|w| w.as_str().map(str::to_owned).ok_or_else(|| format!("\"{key}\" holds something that isn't text"))).collect()
}

/// `[a, b]`: a range, or a curve's point. `what` names it in the error.
fn two<T: TryFrom<i64>>(v: &Doc, what: &str) -> Result<(T, T), String> {
    let bad = || format!("\"{what}\" isn't a pair of whole numbers");
    let [a, b] = v.as_list().ok_or_else(bad)? else { return Err(bad()) };
    let one = |v: &Doc| v.as_i64().and_then(|v| T::try_from(v).ok()).ok_or_else(bad);
    Ok((one(a)?, one(b)?))
}

fn pair<T: TryFrom<i64>>(d: &Doc, key: &str) -> Result<(T, T), String> {
    two(field(d, key)?, key)
}

/// An optional pair: absent (TOML has no null) or null is `None`.
fn maybe_pair<T: TryFrom<i64>>(d: &Doc, key: &str) -> Result<Option<(T, T)>, String> {
    match d.get(key) {
        None | Some(Doc::Null) => Ok(None),
        Some(_) => pair(d, key).map(Some),
    }
}

fn ints<T: Into<i64> + Copy>((a, b): (T, T)) -> Doc {
    Doc::List(vec![Doc::Int(a.into()), Doc::Int(b.into())])
}

fn map(entries: Vec<(&str, Doc)>) -> Doc {
    let mut m = Map::new();
    for (k, v) in entries {
        m.insert(k, v);
    }
    Doc::Map(m)
}

impl Fan {
    fn to_doc(&self) -> Doc {
        match self {
            Fan::Auto => map(vec![("mode", Doc::Str("auto".into()))]),
            Fan::Fixed(p) => map(vec![("mode", Doc::Str("fixed".into())), ("percent", Doc::Int((*p).into()))]),
            Fan::Curve(points) => map(vec![("mode", Doc::Str("curve".into())), ("points", Doc::List(points.iter().map(|p| ints((p.celsius, p.percent))).collect()))]),
        }
    }

    fn from_doc(d: &Doc) -> Result<Fan, String> {
        match text(d, "mode")?.as_str() {
            "auto" => Ok(Fan::Auto),
            "fixed" => Ok(Fan::Fixed(int(d, "percent")?)),
            "curve" => {
                let list = field(d, "points")?.as_list().ok_or("\"points\" isn't a list")?;
                // Read no more than a curve may hold, so a long list costs nothing.
                if list.len() > MAX_POINTS {
                    return Err(format!("a fan curve needs 2 to {MAX_POINTS} points, not {}", list.len()));
                }
                let point = |p: &Doc| two(p, "points").map(|(celsius, percent)| Point { celsius, percent });
                Ok(Fan::Curve(list.iter().map(point).collect::<Result<_, _>>()?))
            }
            other => Err(format!("\"{other}\" isn't a fan mode")),
        }
    }
}

impl Tune {
    pub fn to_doc(&self) -> Doc {
        let c = &self.cpu;
        let cpu = map(vec![
            ("governor", Doc::Str(c.governor.clone())),
            ("epp", Doc::Str(c.epp.clone())),
            ("turbo", Doc::Bool(c.turbo)),
            ("max_percent", Doc::Int(c.max_percent.into())),
            ("pl1", Doc::Int(c.pl1.into())),
            ("pl2", Doc::Int(c.pl2.into())),
        ]);
        let mut entries = vec![("cpu", cpu)];
        if let Some(g) = &self.gpu {
            let mut gpu = vec![("core_offset", Doc::Int(g.core_offset.into())), ("memory_offset", Doc::Int(g.memory_offset.into())), ("power_limit", Doc::Int(g.power_limit.into()))];
            if let Some(mhz) = g.lock {
                gpu.push(("lock", Doc::Int(mhz.into())));
            }
            gpu.push(("fan", g.fan.to_doc()));
            entries.insert(0, ("gpu", map(gpu)));
        }
        map(entries)
    }

    pub fn from_doc(d: &Doc) -> Result<Tune, String> {
        let gpu = match d.get("gpu") {
            None | Some(Doc::Null) => None,
            Some(g) => Some(GpuTune {
                core_offset: int(g, "core_offset")?,
                memory_offset: int(g, "memory_offset")?,
                power_limit: int(g, "power_limit")?,
                lock: match g.get("lock") {
                    None | Some(Doc::Null) => None,
                    Some(_) => Some(int(g, "lock")?),
                },
                fan: Fan::from_doc(field(g, "fan")?)?,
            }),
        };
        let c = field(d, "cpu")?;
        let cpu = CpuTune { governor: text(c, "governor")?, epp: text(c, "epp")?, turbo: flag(c, "turbo")?, max_percent: int(c, "max_percent")?, pl1: int(c, "pl1")?, pl2: int(c, "pl2")? };
        Ok(Tune { gpu, cpu })
    }
}

impl Limits {
    pub fn to_doc(&self) -> Doc {
        let c = &self.cpu;
        let mut cpu = vec![
            ("model", Doc::Str(c.model.clone())),
            ("governors", Doc::List(c.governors.iter().cloned().map(Doc::Str).collect())),
            ("epps", Doc::List(c.epps.iter().cloned().map(Doc::Str).collect())),
            ("turbo", Doc::Bool(c.turbo)),
            ("pcore_mhz", Doc::Int(c.pcore_mhz.into())),
            ("ecore_mhz", Doc::Int(c.ecore_mhz.into())),
        ];
        if let Some(range) = c.power {
            cpu.push(("power", ints(range)));
        }
        let mut entries = vec![("cpu", map(cpu)), ("stock", self.stock.to_doc())];
        if let Some(g) = &self.gpu {
            let mut gpu = vec![
                ("name", Doc::Str(g.name.clone())),
                ("driver", Doc::Str(g.driver.clone())),
                ("core_offset", ints(g.core_offset)),
                ("memory_offset", ints(g.memory_offset)),
                ("power", ints(g.power)),
                ("power_default", Doc::Int(g.power_default.into())),
                ("fans", Doc::Int(g.fans.into())),
                ("slowdown", Doc::Int(g.slowdown.into())),
            ];
            if let Some(range) = g.lock {
                gpu.push(("lock", ints(range)));
            }
            if let Some(range) = g.fan_range {
                gpu.push(("fan_range", ints(range)));
            }
            entries.insert(0, ("gpu", map(gpu)));
        }
        map(entries)
    }

    pub fn from_doc(d: &Doc) -> Result<Limits, String> {
        let gpu = match d.get("gpu") {
            None | Some(Doc::Null) => None,
            Some(g) => Some(GpuLimits {
                name: text(g, "name")?,
                driver: text(g, "driver")?,
                core_offset: pair(g, "core_offset")?,
                memory_offset: pair(g, "memory_offset")?,
                power: pair(g, "power")?,
                power_default: int(g, "power_default")?,
                lock: maybe_pair(g, "lock")?,
                fans: int(g, "fans")?,
                fan_range: maybe_pair(g, "fan_range")?,
                slowdown: int(g, "slowdown")?,
            }),
        };
        let c = field(d, "cpu")?;
        let cpu = CpuLimits {
            model: text(c, "model")?,
            governors: words(c, "governors")?,
            epps: words(c, "epps")?,
            turbo: flag(c, "turbo")?,
            pcore_mhz: int(c, "pcore_mhz")?,
            ecore_mhz: int(c, "ecore_mhz")?,
            power: maybe_pair(c, "power")?,
        };
        Ok(Limits { gpu, cpu, stock: Tune::from_doc(field(d, "stock")?)? })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tune::tests::limits;

    fn curve(points: &[(u32, u32)]) -> Tune {
        let mut t = limits().stock;
        t.gpu.as_mut().unwrap().fan = Fan::Curve(points.iter().map(|&(c, p)| Point::new(c, p)).collect());
        t
    }

    #[test]
    fn tunes_and_limits_survive_json_and_toml() {
        let l = limits();
        let mut t = curve(&[(40, 30), (70, 60), (85, 100)]);
        t.gpu.as_mut().unwrap().lock = Some(1905);
        let json = lntrn_data::json::parse(&lntrn_data::json::write(&t.to_doc())).unwrap();
        assert_eq!(Tune::from_doc(&json), Ok(t.clone()));
        let toml = lntrn_data::toml::parse(&lntrn_data::toml::write(&t.to_doc())).unwrap();
        assert_eq!(Tune::from_doc(&toml), Ok(t));
        let back = lntrn_data::json::parse(&lntrn_data::json::write(&l.to_doc())).unwrap();
        assert_eq!(Limits::from_doc(&back), Ok(l.clone()));
        // A processor-only machine has no graphics part either way.
        let bare = Limits { gpu: None, stock: Tune { gpu: None, ..l.stock.clone() }, ..l };
        let back = lntrn_data::json::parse(&lntrn_data::json::write(&bare.to_doc())).unwrap();
        assert_eq!(Limits::from_doc(&back), Ok(bare));
    }

    #[test]
    fn a_bad_document_says_what_is_wrong_with_it() {
        let parse = |s: &str| Tune::from_doc(&lntrn_data::json::parse(s).unwrap());
        assert!(parse("{}").unwrap_err().contains("cpu"));
        let cpu = r#""cpu":{"governor":"powersave","epp":"","turbo":true,"max_percent":100,"pl1":0,"pl2":0}"#;
        assert!(parse(&format!("{{{cpu}}}")).is_ok());
        assert!(parse(&format!(r#"{{{cpu},"gpu":{{"core_offset":99999999999,"memory_offset":0,"power_limit":1,"fan":{{"mode":"auto"}}}}}}"#)).unwrap_err().contains("out of range"));
        assert!(parse(&format!(r#"{{{cpu},"gpu":{{"core_offset":0,"memory_offset":0,"power_limit":1,"fan":{{"mode":"jet"}}}}}}"#)).unwrap_err().contains("jet"));
        let many = vec!["[40,50]"; 20].join(",");
        assert!(parse(&format!(r#"{{{cpu},"gpu":{{"core_offset":0,"memory_offset":0,"power_limit":1,"fan":{{"mode":"curve","points":[{many}]}}}}}}"#)).is_err());
    }
}

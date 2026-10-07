//! Numbers as people read them: sizes, rates, lengths of time.

const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];

/// `n` scaled to the largest unit it fills at least one of, and which.
fn scaled(n: f64) -> (f64, usize) {
    let mut unit = 0;
    let mut v = n.max(0.0);
    while v >= 1024.0 && unit + 1 < UNITS.len() {
        v /= 1024.0;
        unit += 1;
    }
    (v, unit)
}

/// A value in unit `unit`: whole below a mebibyte and from a hundred up,
/// one decimal between, where it still says something.
fn written(v: f64, unit: usize) -> String {
    if unit < 2 || v >= 99.95 { format!("{v:.0}") } else { format!("{v:.1}") }
}

/// A size in binary units: `812 MiB`, `19.3 GiB`.
pub fn bytes(n: u64) -> String {
    let (v, unit) = scaled(n as f64);
    format!("{} {}", written(v, unit), UNITS[unit])
}

/// How much of something is used, both in the unit of the whole:
/// `19.3 of 31.0 GiB`.
pub fn bytes_of(used: u64, total: u64) -> String {
    let (whole, unit) = scaled(total as f64);
    let part = used as f64 / 1024f64.powi(unit as i32);
    format!("{} of {} {}", written(part, unit), written(whole, unit), UNITS[unit])
}

/// Bytes a second: `1.2 MiB/s`.
pub fn rate(per_second: f64) -> String {
    let (v, unit) = scaled(per_second);
    format!("{} {}/s", written(v, unit), UNITS[unit])
}

/// The top of a graph of rates whose highest is `peak`: the next power
/// of two bytes a second, and never under a kibibyte, so an idle line
/// lies flat instead of filling the graph with its own noise.
pub fn rate_ceiling(peak: f64) -> f64 {
    let mut top = 1024.0;
    while top < peak && top < 1e15 {
        top *= 2.0;
    }
    top
}

/// A percentage: whole, or with one decimal under ten when `fine`
/// (a process's share, where 0.4% and 4% are different stories).
pub fn percent(v: f64, fine: bool) -> String {
    if fine && v < 9.95 { format!("{v:.1}%") } else { format!("{v:.0}%") }
}

/// A length of time by its two largest parts: `3 d 4 h`, `4 h 12 min`,
/// `12 min`, `45 s`.
pub fn duration(seconds: f64) -> String {
    let s = seconds.max(0.0) as u64;
    let (d, h, m) = (s / 86400, s % 86400 / 3600, s % 3600 / 60);
    if d > 0 {
        format!("{d} d {h} h")
    } else if h > 0 {
        format!("{h} h {m} min")
    } else if m > 0 {
        format!("{m} min")
    } else {
        format!("{s} s")
    }
}

/// A clock speed from megahertz: `4.21 GHz`, `800 MHz`.
pub fn clock(mhz: f64) -> String {
    if mhz >= 1000.0 { format!("{:.2} GHz", mhz / 1000.0) } else { format!("{mhz:.0} MHz") }
}

/// A whole number with its thousands set apart: `1,833`.
pub fn count(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// A temperature: `54 °C`.
pub fn celsius(c: f64) -> String {
    format!("{c:.0} °C")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_and_rates_pick_the_unit_that_reads_best() {
        assert_eq!(bytes(0), "0 B");
        assert_eq!(bytes(1023), "1023 B");
        assert_eq!(bytes(1536), "2 KiB");
        assert_eq!(bytes(1_288_490_189), "1.2 GiB");
        assert_eq!(bytes(851_443_712), "812 MiB");
        assert_eq!(bytes(104_815_657), "100 MiB", "99.96 is not written 100.0");
        assert_eq!(bytes_of(20_723_322_880, 33_329_098_752), "19.3 of 31.0 GiB");
        assert_eq!(bytes_of(0, 0), "0 of 0 B");
        assert_eq!(rate(1_258_291.2), "1.2 MiB/s");
        assert_eq!(rate(-5.0), "0 B/s");
    }

    #[test]
    fn a_rate_graph_tops_out_at_a_power_of_two() {
        assert_eq!(rate_ceiling(0.0), 1024.0);
        assert_eq!(rate_ceiling(1024.0), 1024.0);
        assert_eq!(rate_ceiling(1025.0), 2048.0);
        assert_eq!(rate_ceiling(3_000_000.0), 4_194_304.0);
        assert!(rate_ceiling(f64::INFINITY).is_finite());
        assert_eq!(rate_ceiling(f64::NAN), 1024.0);
    }

    #[test]
    fn the_rest_read_as_people_say_them() {
        assert_eq!(percent(37.4, false), "37%");
        assert_eq!(percent(0.42, true), "0.4%");
        assert_eq!(percent(12.6, true), "13%");
        assert_eq!(duration(45.0), "45 s");
        assert_eq!(duration(720.0), "12 min");
        assert_eq!(duration(15_120.0), "4 h 12 min");
        assert_eq!(duration(273_600.0), "3 d 4 h");
        assert_eq!(clock(4210.0), "4.21 GHz");
        assert_eq!(clock(800.0), "800 MHz");
        assert_eq!(count(7), "7");
        assert_eq!(count(1833), "1,833");
        assert_eq!(count(1_000_000), "1,000,000");
        assert_eq!(celsius(53.6), "54 °C");
    }
}

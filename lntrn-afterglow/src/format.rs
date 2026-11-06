//! Numbers as people read them: clocks, heat, power, offsets.

/// A clock: "870 MHz" below a gigahertz, "1.95 GHz" above.
pub fn clock(mhz: f64) -> String {
    if mhz >= 1000.0 { format!("{:.2} GHz", mhz / 1000.0) } else { format!("{mhz:.0} MHz") }
}

pub fn celsius(c: f64) -> String {
    format!("{c:.0} °C")
}

pub fn watts(w: f64) -> String {
    format!("{w:.0} W")
}

pub fn percent(p: f64) -> String {
    format!("{p:.0}%")
}

/// An offset, always signed: "+150 MHz", "−60 MHz", "0 MHz".
pub fn offset(mhz: i64) -> String {
    match mhz {
        0 => "0 MHz".to_owned(),
        m if m > 0 => format!("+{m} MHz"),
        m => format!("\u{2212}{} MHz", -m),
    }
}

/// Seconds as "15 s".
pub fn seconds(s: u64) -> String {
    format!("{s} s")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_read_the_way_they_are_said() {
        assert_eq!(clock(870.0), "870 MHz");
        assert_eq!(clock(1950.0), "1.95 GHz");
        assert_eq!(offset(150), "+150 MHz");
        assert_eq!(offset(-60), "\u{2212}60 MHz");
        assert_eq!(offset(0), "0 MHz");
        assert_eq!((celsius(66.6), watts(319.6), percent(41.2)), ("67 °C".to_owned(), "320 W".to_owned(), "41%".to_owned()));
    }
}

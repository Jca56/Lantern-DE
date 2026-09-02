//! Musical key normalisation. "F#m", "f sharp minor", "Gbm", "11A" (Camelot)
//! and "6m" (Open Key) all resolve to one canonical (musical, Camelot) pair,
//! so whatever a DJ tool wrote — or whatever you type — displays the same.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyInfo {
    pub musical: &'static str,
    pub camelot: &'static str,
}

/// Camelot wheel, index = number - 1: (minor "A" key, major "B" key).
const WHEEL: [(&str, &str); 12] = [
    ("Abm", "B"),
    ("Ebm", "F#"),
    ("Bbm", "Db"),
    ("Fm", "Ab"),
    ("Cm", "Eb"),
    ("Gm", "Bb"),
    ("Dm", "F"),
    ("Am", "C"),
    ("Em", "G"),
    ("Bm", "D"),
    ("F#m", "A"),
    ("Dbm", "E"),
];
const CAMELOT_A: [&str; 12] = [
    "1A", "2A", "3A", "4A", "5A", "6A", "7A", "8A", "9A", "10A", "11A", "12A",
];
const CAMELOT_B: [&str; 12] = [
    "1B", "2B", "3B", "4B", "5B", "6B", "7B", "8B", "9B", "10B", "11B", "12B",
];

pub fn from_camelot(number: usize, minor: bool) -> Option<KeyInfo> {
    if !(1..=12).contains(&number) {
        return None;
    }
    let i = number - 1;
    Some(if minor {
        KeyInfo {
            musical: WHEEL[i].0,
            camelot: CAMELOT_A[i],
        }
    } else {
        KeyInfo {
            musical: WHEEL[i].1,
            camelot: CAMELOT_B[i],
        }
    })
}

/// Pitch class (C=0 … B=11) → Camelot number. Both rings walk the circle of
/// fifths, so it's one modular formula with a per-mode offset.
fn camelot_number(pc: u8, minor: bool) -> usize {
    let off = if minor { 4 } else { 7 };
    ((pc as usize * 7 + off) % 12) + 1
}

/// Open Key (Traktor) numbers are Camelot rotated by five.
fn open_to_camelot(n: usize) -> usize {
    match (n + 7) % 12 {
        0 => 12,
        c => c,
    }
}

pub fn normalize(input: &str) -> Option<KeyInfo> {
    let compact: String = input.chars().filter(|c| !c.is_whitespace()).collect();
    if compact.is_empty() {
        return None;
    }

    // Camelot "11A" / Open Key "11m"
    let digits: String = compact.chars().take_while(|c| c.is_ascii_digit()).collect();
    if !digits.is_empty() {
        let n: usize = digits.parse().ok()?;
        let suffix = compact[digits.len()..].to_ascii_lowercase();
        return match suffix.as_str() {
            "a" => from_camelot(n, true),
            "b" => from_camelot(n, false),
            "m" => from_camelot(open_to_camelot(n), true),
            "d" => from_camelot(open_to_camelot(n), false),
            _ => None,
        };
    }

    let root = compact.chars().next()?;
    let mut pc: i32 = match root.to_ascii_uppercase() {
        'C' => 0,
        'D' => 2,
        'E' => 4,
        'F' => 5,
        'G' => 7,
        'A' => 9,
        'B' => 11,
        _ => return None,
    };
    let mut rest = &compact[root.len_utf8()..];
    loop {
        if let Some(r) = strip_any(rest, &["#", "♯", "sharp", "Sharp"]) {
            pc += 1;
            rest = r;
        } else if let Some(r) = strip_any(rest, &["b", "♭", "flat", "Flat"]) {
            pc -= 1;
            rest = r;
        } else {
            break;
        }
    }
    let minor = match rest {
        "" | "M" | "maj" | "Maj" | "MAJ" | "major" | "Major" | "MAJOR" => false,
        "m" | "min" | "Min" | "MIN" | "minor" | "Minor" | "MINOR" | "-" => true,
        _ => return None,
    };
    let pc = pc.rem_euclid(12) as u8;
    from_camelot(camelot_number(pc, minor), minor)
}

fn strip_any<'a>(s: &'a str, prefixes: &[&str]) -> Option<&'a str> {
    prefixes.iter().find_map(|p| s.strip_prefix(p))
}

/// "F#m · 11A" for anything we understand, the raw text otherwise.
#[allow(dead_code)]
pub fn display(input: &str) -> String {
    match normalize(input) {
        Some(k) => format!("{} · {}", k.musical, k.camelot),
        None => input.trim().to_string(),
    }
}

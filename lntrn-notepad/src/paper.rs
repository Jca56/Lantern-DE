//! The sheet of paper a document is set on when it leaves the screen: a
//! PDF's pages and a Word document's are this size. It is the paper of
//! where the machine says it is, so there is nothing to set.

/// A sheet of paper, in points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Paper {
    pub width: f32,
    pub height: f32,
}

/// The places that write on Letter. Everywhere else it is A4.
const LETTER_LANDS: [&str; 14] = ["US", "CA", "MX", "PH", "CL", "CO", "CR", "DO", "GT", "NI", "PA", "PR", "SV", "VE"];

impl Paper {
    pub const A4: Paper = Paper { width: 595.28, height: 841.89 };
    pub const LETTER: Paper = Paper { width: 612.0, height: 792.0 };

    /// The paper of the place a locale names (`en_US.UTF-8`).
    pub fn of_locale(locale: &str) -> Paper {
        let land = locale.split(['.', '@']).next().and_then(|name| name.split_once('_')).map_or("", |(_, land)| land);
        if LETTER_LANDS.contains(&land) { Paper::LETTER } else { Paper::A4 }
    }

    /// The paper of where this machine says it is.
    pub fn here() -> Paper {
        let set = |name: &str| std::env::var(name).ok().filter(|v| !v.is_empty());
        Paper::of_locale(&set("LC_ALL").or_else(|| set("LC_PAPER")).or_else(|| set("LANG")).unwrap_or_default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paper_is_the_places_own() {
        for (locale, paper) in [("en_US.UTF-8", Paper::LETTER), ("fr_CA", Paper::LETTER), ("es_MX.UTF-8@euro", Paper::LETTER), ("de_DE.UTF-8", Paper::A4), ("en_GB", Paper::A4), ("C", Paper::A4), ("POSIX", Paper::A4), ("", Paper::A4)] {
            assert_eq!(Paper::of_locale(locale), paper, "{locale}");
        }
    }
}

//! Arabic-script joining analysis: which positional form (isolated, initial,
//! medial, final) each cursive letter takes, from the ArabicShaping joining
//! types. The forms select the GSUB `isol`/`init`/`medi`/`fina` features —
//! that's what makes Arabic letters actually connect. Also used by Syriac,
//! N'Ko, Mongolian, etc. (same table).

use crate::unicode::{joining_type, JoiningType as JT};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub(crate) enum Form {
    #[default]
    None,
    Isol,
    Init,
    Medi,
    Fina,
}

/// Per-char positional forms as (byte offset, form), sorted, only for
/// joining letters (D/R/L types). Empty for text with no cursive script —
/// the cheap common case.
pub(crate) fn joining_forms(text: &str) -> Vec<(u32, Form)> {
    let chars: Vec<(usize, JT)> = text
        .char_indices()
        .map(|(i, c)| (i, joining_type(c)))
        .collect();
    if !chars
        .iter()
        .any(|&(_, t)| matches!(t, JT::D | JT::R | JT::L))
    {
        return Vec::new();
    }

    let mut out = Vec::new();
    for (k, &(byte, jt)) in chars.iter().enumerate() {
        if !matches!(jt, JT::D | JT::R | JT::L) {
            continue;
        }
        // Nearest non-transparent neighbors (marks are transparent).
        let prev = chars[..k]
            .iter()
            .rev()
            .map(|&(_, t)| t)
            .find(|&t| t != JT::T);
        let next = chars[k + 1..].iter().map(|&(_, t)| t).find(|&t| t != JT::T);
        // The previous letter extends a connection if dual/causing; the next
        // can receive one if dual/causing/right-joining.
        let linked_prev = matches!(prev, Some(JT::D | JT::C));
        let linked_next = matches!(next, Some(JT::D | JT::C | JT::R));
        let form = match jt {
            JT::D => match (linked_prev, linked_next) {
                (false, false) => Form::Isol,
                (true, false) => Form::Fina,
                (false, true) => Form::Init,
                (true, true) => Form::Medi,
            },
            JT::R => {
                if linked_prev {
                    Form::Fina
                } else {
                    Form::Isol
                }
            }
            _ => {
                // L-joining (rare): initial when followed by a receiver.
                if linked_next {
                    Form::Init
                } else {
                    Form::Isol
                }
            }
        };
        out.push((byte as u32, form));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{joining_forms, Form};

    #[test]
    fn latin_is_empty() {
        assert!(joining_forms("hello").is_empty());
    }

    #[test]
    fn arabic_word_forms() {
        // سلام: seen (D) lam (D) alef (R) meem (D).
        // Logical: س (init) ل (medi) ا (fina, R after D) م (isol? prev=R
        // which does NOT extend a connection → م isolated... prev alef is R:
        // R does not link forward, so meem gets no prev link; nothing after →
        // Isol).
        let forms: Vec<Form> = joining_forms("سلام").iter().map(|&(_, f)| f).collect();
        assert_eq!(forms, vec![Form::Init, Form::Medi, Form::Fina, Form::Isol]);
    }

    #[test]
    fn marks_are_transparent() {
        // ب + fatha mark + ب : the mark must not break the join.
        let forms: Vec<Form> = joining_forms("ب\u{064E}ب")
            .iter()
            .map(|&(_, f)| f)
            .collect();
        assert_eq!(forms, vec![Form::Init, Form::Fina]);
    }
}

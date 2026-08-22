//! UAX#24 script itemization: split text into runs of one script each.
//!
//! Common/Inherited/Unknown characters (punctuation, digits, combining
//! marks, spaces) adopt the script of the preceding run — or the following
//! one when a line *starts* with them. Runs drive per-script shaping in
//! Phase 8; the paired-bracket refinement (UAX#24 §5.1) can join it then.

use super::{script, tables::Script};

// Consumed by per-script shaping in Phase 8; unit-tested meanwhile.
#[allow(dead_code)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ScriptRun {
    pub start: usize,
    pub end: usize,
    pub script: Script,
}

fn is_neutral(s: Script) -> bool {
    matches!(s, Script::Common | Script::Inherited | Script::Unknown)
}

#[allow(dead_code)] // consumed by per-script shaping in Phase 8
pub(crate) fn script_runs(text: &str) -> Vec<ScriptRun> {
    let mut runs: Vec<ScriptRun> = Vec::new();
    for (i, c) in text.char_indices() {
        let end = i + c.len_utf8();
        let s = script(c);
        match runs.last_mut() {
            Some(run) if s == run.script || is_neutral(s) => run.end = end,
            // A leading neutral run adopts the first real script.
            Some(run) if is_neutral(run.script) => {
                run.script = s;
                run.end = end;
            }
            _ => runs.push(ScriptRun {
                start: i,
                end,
                script: s,
            }),
        }
    }
    runs
}

#[cfg(test)]
mod tests {
    use super::{script_runs, Script};

    #[test]
    fn latin_with_punctuation_is_one_run() {
        let runs = script_runs("hello, world 42!");
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].script, Script::Latin);
    }

    #[test]
    fn mixed_scripts_split() {
        let runs = script_runs("abc漢字xyz");
        assert_eq!(runs.len(), 3);
        assert_eq!(runs[0].script, Script::Latin);
        assert_eq!(runs[1].script, Script::Han);
        assert_eq!(runs[2].script, Script::Latin);
    }

    #[test]
    fn leading_neutrals_adopt_first_script() {
        let runs = script_runs("« bonjour »");
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].script, Script::Latin);
    }
}

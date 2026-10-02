//! The `Exec` line of a desktop entry, read the way the Desktop Entry
//! specification says (and the way GLib launches it, where the
//! specification leaves a case open).
//!
//! An Exec line is a command with arguments separated by blanks. An
//! argument may be quoted, and may hold field codes that stand for the
//! files to open:
//!
//!   %f  one file            %F  all files, each an argument of its own
//!   %u  one URL             %U  all URLs, likewise
//!   %i  `--icon <Icon>`     %c  the entry's name     %k  its .desktop file
//!   %%  a percent sign      %d %D %n %N %v %m  deprecated: dropped
//!
//! `parse` turns the line into a template once; `commands` fills in the
//! files and returns the argument lists to run: one for all files when the
//! line takes a list, one per file otherwise. Arguments are handed to the
//! program as they are. No shell is involved, so nothing in a file name is
//! ever interpreted.
//!
//! That has to hold for a launcher that brings its own shell, too
//! (`sh -c "viewer %f"`). There the file's name would become part of the
//! script's text, where no quoting Fox could add is safe: wrapped in single
//! quotes it is still expanded if the script puts double quotes around the
//! code (`sh -c 'viewer "%f"'`), and `$(...)` in a file name then runs. So a
//! file code is only ever a whole argument, or joined to plain text outside
//! quotes (`--file=%f`). `"%f"` on its own is the same as `%f`. A file code
//! in one argument with quoted text or with a blank (`"viewer "%f`,
//! `viewer\ %f`, `"viewer %f"`) makes the line one Fox does not run (the
//! specification leaves that case undefined).

use std::ffi::OsString;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
enum Piece {
    Text(String),
    Code(char),
}

/// An Exec line split into arguments, field codes not yet filled in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Template {
    args: Vec<Vec<Piece>>,
}

/// What `%i`, `%c` and `%k` stand for.
#[derive(Default)]
pub(crate) struct Entry<'a> {
    pub name: &'a str,
    pub icon: Option<&'a str>,
    pub desktop_file: Option<&'a Path>,
}

/// The escapes every string value of a desktop file may hold. They are
/// undone before the quoting rules apply: `\\\\` in a quoted argument is
/// one backslash.
fn unescape_value(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('s') => out.push(' '),
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('\\') => out.push('\\'),
            // Not one of the value escapes: both characters stay for the
            // quoting rules (`\"`, `\$`).
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

const FIELD_CODES: &str = "fFuUickdDnNvm";

pub(crate) fn parse(exec: &str) -> Result<Template, String> {
    #[derive(PartialEq)]
    enum Quote {
        None,
        Double,
        Single,
    }
    fn closes(quote: &Quote, c: char) -> bool {
        matches!((quote, c), (Quote::Double, '"') | (Quote::Single, '\''))
    }
    let line = unescape_value(exec.trim());
    let mut args: Vec<Vec<Piece>> = Vec::new();
    // The argument being read; `None` between arguments. (`""` is an
    // argument, an empty one.)
    let mut arg: Option<Vec<Piece>> = None;
    let mut text = String::new();
    let mut quote = Quote::None;
    // The quoted section being read: whether it holds text, how many codes,
    // and whether one of them stands for a file.
    let (mut q_text, mut q_codes, mut q_file) = (false, 0usize, false);
    // The argument being read holds text that came out of quotes, or a
    // blank: text of the kind a script is made of. A file code in the same
    // argument would put the file's name into that script.
    let mut arg_script = false;
    const MIXED: &str = "a file code inside a quoted command";

    /// The argument is done. An error if it joins a file code to script
    /// text.
    fn close_arg(
        arg: &mut Option<Vec<Piece>>,
        args: &mut Vec<Vec<Piece>>,
        script: &mut bool,
    ) -> Result<(), String> {
        let has_file_code = arg.iter().flatten().any(|piece| {
            matches!(piece, Piece::Code('f' | 'F' | 'u' | 'U' | 'k'))
        });
        if has_file_code && std::mem::take(script) {
            return Err(MIXED.into());
        }
        *script = false;
        args.extend(arg.take());
        Ok(())
    }

    fn flush(text: &mut String, arg: &mut Option<Vec<Piece>>) {
        let arg = arg.get_or_insert_with(Vec::new);
        if !text.is_empty() {
            arg.push(Piece::Text(std::mem::take(text)));
        }
    }

    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            ' ' | '\t' | '\n' if quote == Quote::None => {
                if arg.is_some() || !text.is_empty() {
                    flush(&mut text, &mut arg);
                    close_arg(&mut arg, &mut args, &mut arg_script)?;
                }
            }
            '"' if quote == Quote::None => {
                quote = Quote::Double;
                flush(&mut text, &mut arg);
                (q_text, q_codes, q_file) = (false, 0, false);
            }
            '"' | '\'' if quote != Quote::None && closes(&quote, c) => {
                if q_file && (q_text || q_codes > 1) {
                    return Err(MIXED.into());
                }
                arg_script |= q_text;
                quote = Quote::None;
            }
            // Not in the specification, but GLib reads single quotes the
            // way a shell does and entries rely on it (`sh -c '...'`).
            '\'' if quote == Quote::None => {
                quote = Quote::Single;
                flush(&mut text, &mut arg);
                (q_text, q_codes, q_file) = (false, 0, false);
            }
            '\\' if quote == Quote::Double => {
                q_text = true;
                match chars.peek() {
                    Some('"' | '`' | '$' | '\\') => text.extend(chars.next()),
                    _ => text.push('\\'),
                }
            }
            '\\' if quote == Quote::None => match chars.next() {
                Some(next) => {
                    // An escaped blank is a blank inside the argument.
                    arg_script |= next.is_whitespace();
                    text.push(next);
                }
                None => text.push('\\'),
            },
            '%' => match chars.next() {
                Some('%') => {
                    q_text = true;
                    text.push('%');
                }
                Some(code) if FIELD_CODES.contains(code) => {
                    flush(&mut text, &mut arg);
                    q_codes += 1;
                    // What the user's files (and the entry's own path) put
                    // in: text Fox does not control.
                    q_file |= matches!(code, 'f' | 'F' | 'u' | 'U' | 'k');
                    arg.get_or_insert_with(Vec::new).push(Piece::Code(code));
                }
                // "Command lines that contain a field code that is not
                // listed in this specification are invalid and must not be
                // processed."
                Some(code) => return Err(format!("unknown field code %{code}")),
                None => return Err("a % at the end of the line".into()),
            },
            c => {
                q_text = true;
                text.push(c);
            }
        }
    }
    if quote != Quote::None {
        return Err("a quote is never closed".into());
    }
    if arg.is_some() || !text.is_empty() {
        flush(&mut text, &mut arg);
        close_arg(&mut arg, &mut args, &mut arg_script)?;
    }
    if args.is_empty() {
        return Err("the command is empty".into());
    }
    Ok(Template { args })
}

impl Template {
    fn codes(&self) -> impl Iterator<Item = char> + '_ {
        self.args.iter().flatten().filter_map(|piece| match piece {
            Piece::Code(code) => Some(*code),
            Piece::Text(_) => None,
        })
    }

    /// The command lines that open `files`: each a program and its
    /// arguments. A line with `%F`/`%U` takes all files at once; any other
    /// is run once per file. With no files, the command once, without.
    pub(crate) fn commands(&self, files: &[PathBuf], entry: &Entry<'_>) -> Vec<Vec<OsString>> {
        let takes_list = self.codes().any(|c| matches!(c, 'F' | 'U'));
        let takes_file = takes_list || self.codes().any(|c| matches!(c, 'f' | 'u'));
        let batches: Vec<&[PathBuf]> = if takes_list || files.is_empty() {
            vec![files]
        } else {
            files.chunks(1).collect()
        };
        batches
            .into_iter()
            .map(|batch| {
                let mut argv = self.fill(batch, entry);
                // An entry that names no file code was still chosen to open
                // this file (Open With lists by MIME type): hand it over
                // the way a command line would.
                if !takes_file {
                    argv.extend(batch.iter().map(|f| f.as_os_str().to_os_string()));
                }
                argv
            })
            .filter(|argv| !argv.is_empty())
            .collect()
    }

    fn fill(&self, files: &[PathBuf], entry: &Entry<'_>) -> Vec<OsString> {
        let mut argv: Vec<OsString> = Vec::new();
        for arg in &self.args {
            let mut cur = OsString::new();
            // An argument made of nothing but codes that stand for nothing
            // (`%i` without an icon, `%f` without a file) is no argument.
            let mut solid = arg.is_empty();
            for piece in arg {
                match piece {
                    Piece::Text(text) => {
                        cur.push(text);
                        solid = true;
                    }
                    Piece::Code('f' | 'u') => {
                        if let Some(file) = files.first() {
                            cur.push(file.as_os_str());
                            solid = true;
                        }
                    }
                    Piece::Code('F' | 'U') => {
                        for (i, file) in files.iter().enumerate() {
                            if i > 0 {
                                // Each file an argument of its own.
                                argv.push(std::mem::take(&mut cur));
                            }
                            cur.push(file.as_os_str());
                            solid = true;
                        }
                    }
                    Piece::Code('i') => {
                        if let Some(icon) = entry.icon.filter(|icon| !icon.is_empty()) {
                            cur.push("--icon");
                            argv.push(std::mem::take(&mut cur));
                            cur.push(icon);
                            solid = true;
                        }
                    }
                    Piece::Code('c') => {
                        cur.push(entry.name);
                        solid = true;
                    }
                    Piece::Code('k') => {
                        if let Some(file) = entry.desktop_file {
                            cur.push(file.as_os_str());
                            solid = true;
                        }
                    }
                    // Deprecated codes stand for nothing.
                    Piece::Code(_) => {}
                }
            }
            if solid {
                argv.push(cur);
            }
        }
        argv
    }
}

#[cfg(test)]
mod tests;

//! Is it safe to let `git status` read this repository's configuration?
//!
//! Fox runs `git status` in whatever folder the user walks into, and a
//! repository's own `.git/config` can name programs for git to run: a
//! `filter.<x>.clean` for every changed file, `core.fsmonitor`, a promisor
//! remote whose `uploadpack` is fetched from on demand, anything an
//! `include` pulls in. An extracted archive or a USB stick is enough to
//! put such a file under the user's cursor.
//!
//! So the file is read here first, by rules of our own, and git is only
//! started when every key in it is on a short list of settings known to be
//! plain data. It is a list of what is allowed, not of what is dangerous:
//! a key git learns tomorrow is refused until someone adds it here.
//!
//! The reader follows git's own (config.c) character by character and
//! refuses whatever it does not understand, so that there is no spelling
//! the two of them read differently.

/// One `section[.subsection].key [= value]` of a config file.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Item {
    /// Lower case.
    pub section: String,
    /// As written (case matters to git) for `[section "sub"]`.
    pub subsection: Option<String>,
    /// Lower case.
    pub key: String,
    /// `None`: the key stood alone, which git reads as "true".
    pub value: Option<String>,
}

/// What the caller has to act on besides "every key is harmless".
#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct Facts {
    /// Every `core.worktree`: where git would take the work tree to be.
    /// The caller accepts it only when it is the folder it found itself.
    pub worktrees: Vec<String>,
    /// `extensions.worktreeConfig`: git also reads `config.worktree`.
    pub worktree_config: bool,
}

/// Read a config file's text. `Err` says, in words for the log, why git
/// must not be run with it.
pub(super) fn inspect(text: &str) -> Result<Facts, String> {
    let mut facts = Facts::default();
    for item in parse(text)? {
        match (
            item.section.as_str(),
            item.subsection.as_deref(),
            item.key.as_str(),
        ) {
            ("core", None, "worktree") => {
                // A bare `worktree` is "true", which is no path at all.
                facts.worktrees.push(item.value.clone().unwrap_or_default());
            }
            ("extensions", None, "worktreeconfig") => facts.worktree_config = true,
            _ => {}
        }
        if !allowed(&item) {
            return Err(format!("its config sets {}", describe(&item)));
        }
    }
    Ok(facts)
}

fn describe(item: &Item) -> String {
    match &item.subsection {
        Some(sub) => format!("{}.{}.{}", item.section, sub, item.key),
        None => format!("{}.{}", item.section, item.key),
    }
}

/// Settings of `[core]` that are numbers, switches and names: nothing git
/// would run, and no path it would read.
const CORE: &[&str] = &[
    "repositoryformatversion",
    "filemode",
    "bare",
    "logallrefupdates",
    "ignorecase",
    "precomposeunicode",
    "symlinks",
    "autocrlf",
    "eol",
    "safecrlf",
    "quotepath",
    "trustctime",
    "checkstat",
    "abbrev",
    "compression",
    "loosecompression",
    "bigfilethreshold",
    "preloadindex",
    "splitindex",
    "untrackedcache",
    "commitgraph",
    "multipackindex",
    "sparsecheckout",
    "sparsecheckoutcone",
    "longpaths",
    "fscache",
    "protecthfs",
    "protectntfs",
    "hidedotfiles",
    "filesreflocktimeout",
    "packedrefstimeout",
    "sharedrepository",
    "whitespace",
    // Checked against the folder by the caller (see `Facts::worktrees`).
    "worktree",
];

fn allowed(item: &Item) -> bool {
    let key = item.key.as_str();
    let named = item.subsection.is_some();
    match item.section.as_str() {
        "core" => !named && CORE.contains(&key),
        "remote" if !named => key == "pushdefault",
        // Not `promisor`, `partialclonefilter`, `uploadpack`, `receivepack`
        // or `vcs`: a status of a partial clone fetches what it is missing,
        // through whatever program the remote names.
        "remote" => {
            matches!(
                key,
                "url"
                    | "pushurl"
                    | "fetch"
                    | "push"
                    | "tagopt"
                    | "mirror"
                    | "prune"
                    | "prunetags"
                    | "skipdefaultupdate"
                    | "skipfetchall"
                    | "gh-resolved"
            )
        }
        "branch" if named => matches!(
            key,
            "remote"
                | "merge"
                | "rebase"
                | "pushremote"
                | "description"
                | "gh-merge-base"
                | "vscode-merge-base"
                | "github-pr-owner-number"
        ),
        "branch" => matches!(key, "autosetupmerge" | "autosetuprebase" | "sort"),
        "user" => !named && matches!(key, "name" | "email" | "signingkey"),
        "submodule" if named => match key {
            "url" | "active" | "branch" | "ignore" | "fetchrecursesubmodules" => true,
            // `!command` is a program to run on update.
            "update" => matches!(
                item.value.as_deref(),
                Some("checkout" | "rebase" | "merge" | "none")
            ),
            _ => false,
        },
        "submodule" => key == "active",
        // Not `partialclone`: see "remote" above.
        "extensions" => {
            !named
                && matches!(
                    key,
                    "objectformat"
                        | "refstorage"
                        | "worktreeconfig"
                        | "noop"
                        | "relativeworktrees"
                        | "preciousobjects"
                )
        }
        "pull" => !named && matches!(key, "rebase" | "ff"),
        "push" => !named && matches!(key, "default" | "autosetupremote" | "followtags"),
        "fetch" => !named && matches!(key, "prune" | "prunetags"),
        // Not `merge.tool`, and no `[merge "driver"]`.
        "merge" => !named && matches!(key, "ff" | "conflictstyle"),
        "rebase" => !named && matches!(key, "autostash" | "autosquash" | "updaterefs"),
        "rerere" => !named && matches!(key, "enabled" | "autoupdate"),
        "commit" => !named && matches!(key, "gpgsign" | "verbose"),
        "tag" => !named && matches!(key, "gpgsign" | "sort"),
        "init" => !named && key == "defaultbranch",
        "status" => {
            !named
                && matches!(
                    key,
                    "showuntrackedfiles"
                        | "relativepaths"
                        | "renames"
                        | "renamelimit"
                        | "aheadbehind"
                )
        }
        // Not `diff.external`, `diff.tool`, and no `[diff "driver"]`.
        "diff" => {
            !named
                && matches!(
                    key,
                    "renames"
                        | "renamelimit"
                        | "algorithm"
                        | "colormoved"
                        | "colormovedws"
                        | "mnemonicprefix"
                )
        }
        "index" => !named && matches!(key, "version" | "threads" | "sparse"),
        "feature" => !named && matches!(key, "manyfiles" | "experimental"),
        "gc" => {
            !named
                && matches!(
                    key,
                    "auto"
                        | "autodetach"
                        | "autopacklimit"
                        | "pruneexpire"
                        | "reflogexpire"
                        | "reflogexpireunreachable"
                )
        }
        "maintenance" => !named && matches!(key, "auto" | "strategy"),
        // Colours and hints: values git only ever prints with.
        "color" | "advice" => true,
        "gui" => !named && matches!(key, "wmstate" | "geometry" | "encoding"),
        // What `git lfs install --local` and a first push write. Not
        // `lfs.extension.*`, which names programs for git-lfs to run.
        "lfs" if named => matches!(key, "access" | "locksverify"),
        "lfs" => key == "repositoryformatversion",
        // filter, include, includeIf, alias, credential, url, http, pager,
        // sequence, mergetool, difftool, gpg, protocol, … and every section
        // not known here.
        _ => false,
    }
}

// ── The reader ──────────────────────────────────────────────────────────────

struct Source<'a> {
    chars: std::iter::Peekable<std::str::Chars<'a>>,
}

impl Source<'_> {
    /// The next character, with "\r\n" read as "\n" like git does. `None`
    /// at the end of the file.
    fn next(&mut self) -> Result<Option<char>, String> {
        let Some(c) = self.chars.next() else {
            return Ok(None);
        };
        match c {
            '\r' if self.chars.peek() == Some(&'\n') => {
                self.chars.next();
                Ok(Some('\n'))
            }
            '\n' | '\t' => Ok(Some(c)),
            // git has rules for a lone CR, a form feed, a NUL. They are not
            // worth knowing: such a file is simply not read.
            c if c.is_control() => Err("its config holds control characters".into()),
            c => Ok(Some(c)),
        }
    }
}

fn is_key_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '-'
}

fn bad() -> String {
    "its config cannot be read".to_string()
}

pub(super) fn parse(text: &str) -> Result<Vec<Item>, String> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut src = Source {
        chars: text.chars().peekable(),
    };
    let mut items = Vec::new();
    let mut section: Option<(String, Option<String>)> = None;

    while let Some(c) = src.next()? {
        match c {
            '\n' | ' ' | '\t' => {}
            '#' | ';' => {
                // A comment runs to the end of its line.
                while !matches!(src.next()?, None | Some('\n')) {}
            }
            '[' => section = Some(section_header(&mut src)?),
            c if c.is_ascii_alphabetic() => {
                let Some((name, sub)) = section.clone() else {
                    return Err(bad());
                };
                let (key, value) = key_and_value(c, &mut src)?;
                items.push(Item {
                    section: name,
                    subsection: sub,
                    key,
                    value,
                });
            }
            _ => return Err(bad()),
        }
    }
    Ok(items)
}

/// After the `[`: `name]`, `name.sub]` (old style) or `name "sub"]`.
fn section_header(src: &mut Source<'_>) -> Result<(String, Option<String>), String> {
    let mut name = String::new();
    let quoted = loop {
        match src.next()?.ok_or_else(bad)? {
            ']' => break None,
            ' ' | '\t' => break Some(quoted_subsection(src)?),
            c if is_key_char(c) || c == '.' => name.push(c.to_ascii_lowercase()),
            _ => return Err(bad()),
        }
    };
    // `[filter.x]` is `[filter "x"]` to git.
    let (name, dotted) = match name.split_once('.') {
        Some((head, tail)) => (head.to_string(), Some(tail.to_string())),
        None => (name, None),
    };
    if name.is_empty() {
        return Err(bad());
    }
    let subsection = match (dotted, quoted) {
        (None, sub) => sub,
        (Some(dotted), None) => Some(dotted),
        (Some(dotted), Some(quoted)) => Some(format!("{dotted}.{quoted}")),
    };
    Ok((name, subsection))
}

/// After the blank that follows a section name: `"sub"]`.
fn quoted_subsection(src: &mut Source<'_>) -> Result<String, String> {
    let open = loop {
        match src.next()?.ok_or_else(bad)? {
            ' ' | '\t' => {}
            c => break c,
        }
    };
    if open != '"' {
        return Err(bad());
    }
    let mut sub = String::new();
    loop {
        match src.next()?.ok_or_else(bad)? {
            '\n' => return Err(bad()),
            '"' => break,
            '\\' => match src.next()?.ok_or_else(bad)? {
                '\n' => return Err(bad()),
                c => sub.push(c),
            },
            c => sub.push(c),
        }
    }
    match src.next()? {
        Some(']') => Ok(sub),
        _ => Err(bad()),
    }
}

/// A key line, its first letter already read: `key`, `key = value`.
fn key_and_value(first: char, src: &mut Source<'_>) -> Result<(String, Option<String>), String> {
    let mut key = String::from(first.to_ascii_lowercase());
    let mut c = src.next()?;
    while let Some(k) = c.filter(|k| is_key_char(*k)) {
        key.push(k.to_ascii_lowercase());
        c = src.next()?;
    }
    while matches!(c, Some(' ' | '\t')) {
        c = src.next()?;
    }
    match c {
        None | Some('\n') => Ok((key, None)),
        Some('=') => Ok((key, Some(value(src)?))),
        Some(_) => Err(bad()),
    }
}

/// After the `=`: the value up to the end of the line, with git's quoting,
/// escapes, trailing comment and line continuation.
fn value(src: &mut Source<'_>) -> Result<String, String> {
    let mut out = String::new();
    let mut quoted = false;
    let mut comment = false;
    let mut blanks = 0usize;
    loop {
        let c = match src.next()? {
            None | Some('\n') => {
                return if quoted { Err(bad()) } else { Ok(out) };
            }
            Some(c) => c,
        };
        if comment {
            continue;
        }
        if !quoted && matches!(c, ' ' | '\t') {
            // Leading blanks are dropped, trailing ones never written.
            if !out.is_empty() {
                blanks += 1;
            }
            continue;
        }
        if !quoted && matches!(c, '#' | ';') {
            comment = true;
            continue;
        }
        out.extend(std::iter::repeat(' ').take(blanks));
        blanks = 0;
        match c {
            '\\' => match src.next()? {
                // The value goes on in the next line.
                Some('\n') => {}
                Some('t') => out.push('\t'),
                Some('n') => out.push('\n'),
                Some('b') => out.push('\u{8}'),
                Some(c @ ('\\' | '"')) => out.push(c),
                _ => return Err(bad()),
            },
            '"' => quoted = !quoted,
            c => out.push(c),
        }
    }
}

#[cfg(test)]
mod tests;

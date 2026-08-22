//! Desktop-file scanner + apps search provider.
//!
//! Scans the standard freedesktop application directories on startup,
//! parses each `.desktop` file into a `DesktopEntry`, and exposes a
//! `rank` method that scores all entries against a query using the
//! fuzzy matcher.
//!
//! Phase 2.2: data layer + ranking only. Phase 2.5 wires the result list
//! into the panel (currently the render layer just shows the names).
//!
//! Self-contained: we don't import from `lntrn-bar` or any shared crate.
//! The bar's `desktop.rs` is read-only reference; this file is its own
//! parallel implementation.

use std::path::{Path, PathBuf};

use super::fuzzy;

/// Parsed `.desktop` entry. Minimal subset — what the launcher needs.
/// Some fields are wired up in later phases; suppress dead_code now.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct DesktopEntry {
    /// File-stem app id, e.g. "firefox" for `/usr/share/applications/firefox.desktop`.
    pub app_id: String,
    /// `Name=` field, falling back to `app_id`.
    pub name: String,
    /// `Exec=` field with `%f`/`%U`/etc. field codes stripped.
    pub exec: String,
    /// `Icon=` field. Theme name or absolute path. None if missing.
    pub icon_name: Option<String>,
    /// `Keywords=` (semicolon-separated → Vec).
    pub keywords: Vec<String>,
    /// `Comment=` field.
    pub comment: Option<String>,
    /// Source `.desktop` file path.
    pub path: PathBuf,
    /// `NoDisplay=true` — the entry is registered but should not appear
    /// in launchers. We still keep it indexed but launcher views filter
    /// these out by default.
    pub no_display: bool,
}

/// Search-time view of a desktop entry. Holds only the fields needed by
/// the fuzzy matcher, pre-lowercased once at parse time so we don't
/// allocate during each keystroke.
struct Indexed {
    entry_idx: usize,
    name: String,
    app_id: String,
    keywords_joined: String,
}

/// Apps provider. Owns the cached desktop entries and a parallel index
/// used by the fuzzy matcher.
pub struct AppsProvider {
    entries: Vec<DesktopEntry>,
    index: Vec<Indexed>,
}

impl AppsProvider {
    /// Scan all standard application directories and build the index.
    /// This is called once on daemon startup; we cache the result.
    pub fn scan() -> Self {
        let dirs = standard_dirs();
        let mut entries: Vec<DesktopEntry> = Vec::with_capacity(256);

        // Track app_ids we've already seen so user overrides (in
        // ~/.local/share/applications, listed first) shadow system files.
        let mut seen_ids = std::collections::HashSet::new();

        for dir in &dirs {
            scan_dir(dir, &mut entries, &mut seen_ids);
        }

        let index = entries
            .iter()
            .enumerate()
            .map(|(i, e)| Indexed {
                entry_idx: i,
                name: e.name.clone(),
                app_id: e.app_id.clone(),
                keywords_joined: e.keywords.join(" "),
            })
            .collect();

        tracing::info!(count = entries.len(), "scanned .desktop files");

        Self { entries, index }
    }

    /// Total entries known. Mostly for diagnostics.
    pub fn count(&self) -> usize {
        self.entries.len()
    }

    /// Look up an entry by index (matches the `usize` returned by `rank`).
    pub fn get(&self, idx: usize) -> Option<&DesktopEntry> {
        self.entries.get(idx)
    }

    /// Rank all entries against `query`. Returns matches sorted by
    /// descending score. Filters out `NoDisplay=true` and user-hidden entries.
    /// `limit` caps the result count.
    pub fn rank(
        &self,
        query: &str,
        limit: usize,
        hidden: &crate::launcher::hidden::Hidden,
    ) -> Vec<RankedEntry> {
        if query.is_empty() {
            return Vec::new();
        }

        let mut hits: Vec<RankedEntry> = self
            .index
            .iter()
            .filter(|ix| {
                let e = &self.entries[ix.entry_idx];
                !e.no_display && !hidden.is_hidden(&e.app_id)
            })
            .filter_map(|ix| {
                // Score against name, app_id, and keywords; take the best.
                let s_name = fuzzy::score(query, &ix.name);
                let s_id = fuzzy::score(query, &ix.app_id);
                let s_kw = if ix.keywords_joined.is_empty() {
                    None
                } else {
                    fuzzy::score(query, &ix.keywords_joined)
                };

                let mut best = s_name;
                for s in [s_id, s_kw].into_iter().flatten() {
                    best = Some(best.map_or(s, |b| b.max(s)));
                }
                let score = best?;

                // Slight bias toward `name` matches over `keywords` — if
                // both matched, `name` already wins via max(). We just
                // ensure that an "id-only" match (e.g. app_id contains
                // the query but name doesn't) is dampened a touch.
                let score = if s_name.is_none() {
                    score * 0.85
                } else {
                    score
                };

                Some(RankedEntry {
                    entry_idx: ix.entry_idx,
                    score,
                })
            })
            .collect();

        hits.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        hits.truncate(limit);
        hits
    }
}

#[derive(Debug, Clone, Copy)]
pub struct RankedEntry {
    pub entry_idx: usize,
    pub score: f32,
}

// ── Directory discovery ─────────────────────────────────────────────────────

fn standard_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();

    // User dir first so it shadows system files for the same app_id.
    if let Some(home) = std::env::var_os("HOME") {
        let mut p = PathBuf::from(home);
        p.push(".local/share/applications");
        dirs.push(p);
    }

    // Flatpak user exports.
    if let Some(home) = std::env::var_os("HOME") {
        let mut p = PathBuf::from(home);
        p.push(".local/share/flatpak/exports/share/applications");
        dirs.push(p);
    }

    // System dirs from XDG_DATA_DIRS, falling back to the spec defaults.
    let data_dirs =
        std::env::var("XDG_DATA_DIRS").unwrap_or_else(|_| "/usr/local/share:/usr/share".into());
    for d in data_dirs.split(':') {
        if d.is_empty() {
            continue;
        }
        let mut p = PathBuf::from(d);
        p.push("applications");
        dirs.push(p);
    }

    // Flatpak system exports.
    dirs.push(PathBuf::from("/var/lib/flatpak/exports/share/applications"));

    dirs
}

fn scan_dir(
    dir: &Path,
    out: &mut Vec<DesktopEntry>,
    seen_ids: &mut std::collections::HashSet<String>,
) {
    let read_dir = match std::fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(_) => return,
    };
    for entry in read_dir.flatten() {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("desktop") {
            continue;
        }
        let app_id = match path.file_stem().and_then(|s| s.to_str()) {
            Some(s) => s.to_string(),
            None => continue,
        };
        if !seen_ids.insert(app_id.clone()) {
            // Already have a user-local override; skip.
            continue;
        }
        if let Some(parsed) = parse_desktop_file(&path, &app_id) {
            out.push(parsed);
        }
    }
}

// ── .desktop parser ─────────────────────────────────────────────────────────

fn parse_desktop_file(path: &Path, app_id: &str) -> Option<DesktopEntry> {
    let content = std::fs::read_to_string(path).ok()?;

    // Read only the [Desktop Entry] group. Per-locale subsections and
    // [Desktop Action ...] groups are ignored.
    let mut in_group = false;
    let mut name: Option<String> = None;
    let mut exec_raw: Option<String> = None;
    let mut icon_name: Option<String> = None;
    let mut keywords: Vec<String> = Vec::new();
    let mut comment: Option<String> = None;
    let mut no_display = false;
    let mut hidden = false;
    let mut entry_type: Option<String> = None;

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') {
            in_group = line == "[Desktop Entry]";
            continue;
        }
        if !in_group {
            continue;
        }
        if let Some((key, val)) = line.split_once('=') {
            let key = key.trim();
            let val = val.trim();
            match key {
                "Name" => {
                    if name.is_none() {
                        name = Some(val.to_string());
                    }
                }
                "Exec" => exec_raw = Some(val.to_string()),
                "Icon" => icon_name = Some(val.to_string()),
                "Keywords" => {
                    keywords.extend(val.split(';').filter(|s| !s.is_empty()).map(String::from));
                }
                "Comment" => {
                    if comment.is_none() {
                        comment = Some(val.to_string());
                    }
                }
                "NoDisplay" => no_display = val.eq_ignore_ascii_case("true"),
                "Hidden" => hidden = val.eq_ignore_ascii_case("true"),
                "Type" => entry_type = Some(val.to_string()),
                _ => {}
            }
        }
    }

    // Spec: only Type=Application is launchable; ignore Link / Directory.
    if entry_type.as_deref() != Some("Application") {
        return None;
    }
    if hidden {
        // Hidden=true means "treat as if it didn't exist." Skip entirely.
        return None;
    }

    let exec = exec_raw.map(|s| strip_field_codes(&s)).unwrap_or_default();
    let name = name.unwrap_or_else(|| app_id.to_string());

    Some(DesktopEntry {
        app_id: app_id.to_string(),
        name,
        exec,
        icon_name,
        keywords,
        comment,
        path: path.to_path_buf(),
        no_display,
    })
}

/// Strip freedesktop field codes (`%f %F %u %U %i %c %k %d %D %n %N %v %m`).
/// Also removes a trailing single-arg `%X` that some entries leave hanging.
fn strip_field_codes(exec: &str) -> String {
    let mut out = String::with_capacity(exec.len());
    let mut chars = exec.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '%' {
            match chars.peek() {
                Some(&'%') => {
                    out.push('%');
                    chars.next();
                }
                Some(&c)
                    if matches!(
                        c,
                        'f' | 'F' | 'u' | 'U' | 'i' | 'c' | 'k' | 'd' | 'D' | 'n' | 'N' | 'v' | 'm'
                    ) =>
                {
                    chars.next();
                }
                _ => out.push(ch),
            }
        } else {
            out.push(ch);
        }
    }
    // Collapse double spaces that field-code stripping may have left.
    while out.contains("  ") {
        out = out.replace("  ", " ");
    }
    out.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_simple() {
        assert_eq!(strip_field_codes("firefox %u"), "firefox");
        assert_eq!(
            strip_field_codes("env DISPLAY=:0 mpv %F"),
            "env DISPLAY=:0 mpv"
        );
        assert_eq!(strip_field_codes("foo %% bar"), "foo % bar");
    }

    #[test]
    fn strip_unknown_percent_kept() {
        assert_eq!(strip_field_codes("foo %z bar"), "foo %z bar");
    }
}

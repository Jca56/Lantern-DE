//! The apps installed here, as their `.desktop` files give them: what
//! each is called, its icon and the command that starts it, made ready
//! for a button of the ring.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// The terminal a `Terminal=true` app is run in.
const TERMINAL: &str = "lntrn-terminal -e";
/// How far down an `applications` folder is looked into (Wine keeps its
/// programs a few folders deep).
const DEPTH: usize = 4;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct App {
    pub name: String,
    /// The program and its arguments, as the desktop runs them.
    pub command: String,
    /// An icon theme's name for its icon, or a picture's path.
    pub icon: String,
}

/// Every app a launcher would list, by name.
pub fn installed() -> Vec<App> {
    let mut seen = HashSet::new();
    let mut apps = Vec::new();
    for dir in dirs() {
        scan(&dir, &dir, DEPTH, &mut seen, &mut apps);
    }
    apps.sort_by_key(|a| a.name.to_lowercase());
    apps
}

/// The `applications` folders, the user's own first: a file there
/// stands in for one of the same name further on.
fn dirs() -> Vec<PathBuf> {
    let home = lntrn_sys::dirs::home().unwrap_or_default();
    let set = |name: &str| std::env::var(name).ok().filter(|v| !v.is_empty());
    let data_home = set("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".local/share"));
    let data_dirs = set("XDG_DATA_DIRS").unwrap_or_else(|| "/usr/local/share:/usr/share".to_owned());
    let mut out = vec![data_home.join("applications")];
    for dir in data_dirs.split(':').filter(|d| !d.is_empty()) {
        let dir = Path::new(dir).join("applications");
        if !out.contains(&dir) {
            out.push(dir);
        }
    }
    out
}

fn scan(root: &Path, dir: &Path, depth: usize, seen: &mut HashSet<String>, apps: &mut Vec<App>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if depth > 0 {
                scan(root, &path, depth - 1, seen, apps);
            }
            continue;
        }
        if path.extension().and_then(|e| e.to_str()) != Some("desktop") {
            continue;
        }
        // Its id: where it is under `applications`, the slashes dashes.
        let id = path.strip_prefix(root).unwrap_or(&path).to_string_lossy().replace('/', "-");
        if !seen.insert(id) {
            continue;
        }
        if let Some(app) = std::fs::read_to_string(&path).ok().and_then(|text| parse(&text)) {
            apps.push(app);
        }
    }
}

/// The app a `.desktop` file describes: `None` for one a launcher
/// leaves out (hidden, not an application, nothing to run).
fn parse(text: &str) -> Option<App> {
    let (mut name, mut exec, mut icon) = (None, None, String::new());
    let (mut terminal, mut in_entry) = (false, false);
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_entry = line == "[Desktop Entry]";
            continue;
        }
        let Some((key, value)) = line.split_once('=').filter(|_| in_entry && !line.starts_with('#')) else { continue };
        let value = value.trim();
        match key.trim() {
            "Type" if value != "Application" => return None,
            "NoDisplay" | "Hidden" if value == "true" => return None,
            "Name" => name = Some(unescape(value)),
            "Exec" => exec = Some(unescape(value)),
            "Icon" => icon = unescape(value),
            "Terminal" => terminal = value == "true",
            _ => {}
        }
    }
    let command = command(&exec?, terminal);
    (!command.is_empty()).then(|| App { name: name.filter(|n| !n.is_empty()).unwrap_or_else(|| command.clone()), command, icon })
}

/// A `.desktop` string as it is meant: `\\` is one backslash, `\s` a
/// space, `\n`, `\t` and `\r` what they are anywhere. (An `Exec=` is
/// escaped twice, here and again inside its quotes: Wine's `C:\\\\x`
/// is `C:\\x` to the command line and `C:\x` to the program.)
fn unescape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars();
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
            Some(other) => out.push(other),
            None => {}
        }
    }
    out
}

/// An `Exec=` as a command the ring can run: its field codes (`%U`, the
/// files a launcher would hand it) taken out, `%%` a percent sign, and a
/// terminal put in front of an app that needs one. Quotes stay: the
/// desktop reads them the way a launcher does.
fn command(exec: &str, terminal: bool) -> String {
    let mut out = String::with_capacity(exec.len());
    let mut chars = exec.chars();
    while let Some(c) = chars.next() {
        match c {
            '%' => {
                if chars.next() == Some('%') {
                    out.push('%');
                }
            }
            c => out.push(c),
        }
    }
    let out = out.split_whitespace().collect::<Vec<_>>().join(" ");
    if terminal && !out.is_empty() { format!("{TERMINAL} {out}") } else { out }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_exec_loses_its_field_codes_and_keeps_its_quotes() {
        assert_eq!(command("firefox-bin --name=firefox-bin %u", false), "firefox-bin --name=firefox-bin");
        assert_eq!(command("/usr/bin/vscode --open-url %U", false), "/usr/bin/vscode --open-url");
        assert_eq!(command("\"/home/a/Ember Nights/run.sh\" %f --at 50%%", false), "\"/home/a/Ember Nights/run.sh\" --at 50%");
        assert_eq!(command("env \"WINEPREFIX=/home/a/.wine\" wine start %u", false), "env \"WINEPREFIX=/home/a/.wine\" wine start");
        assert_eq!(command("htop", true), "lntrn-terminal -e htop");
        assert_eq!(command("%U", true), "");
    }

    /// What this machine has, printed: `cargo test -- --ignored --nocapture what_is_installed`.
    #[test]
    #[ignore = "reads this machine's .desktop files"]
    fn what_is_installed_here_is_listed() {
        let apps = installed();
        for app in &apps {
            println!("{:<34} {:<28} {}", app.name, app.icon, app.command);
        }
        assert!(apps.windows(2).all(|w| w[0].name.to_lowercase() <= w[1].name.to_lowercase()));
        assert!(apps.iter().all(|a| !a.command.is_empty() && !a.command.contains('%')));
    }

    #[test]
    fn a_desktop_file_is_an_app_unless_a_launcher_would_hide_it() {
        let app = parse("[Desktop Entry]\nType=Application\nName=Web Browser\nName[de]=Internet\n# Exec=nope\nExec=firefox %u\nIcon=firefox\n\n[Desktop Action new-window]\nName=New Window\nExec=firefox --new-window\n");
        assert_eq!(app, Some(App { name: "Web Browser".into(), command: "firefox".into(), icon: "firefox".into() }));
        assert_eq!(parse("[Desktop Entry]\nName=htop\nExec=htop\nTerminal=true\n").map(|a| a.command), Some("lntrn-terminal -e htop".into()));
        assert_eq!(parse("[Desktop Entry]\nName=Handler\nExec=wine start %f\nNoDisplay=true\n"), None);
        assert_eq!(parse("[Desktop Entry]\nType=Link\nName=Site\nURL=https://example.org\n"), None);
        assert_eq!(parse("[Desktop Entry]\nName=Nothing to run\n"), None);
        // Wine's shortcuts: four backslashes in the file are two on the
        // command line, which its quotes make one for the program.
        let wine = parse("[Desktop Entry]\nName=Live\nExec=env \"WINEPREFIX=/home/a/.wine\" wine \"C:\\\\\\\\Live\\sSuite.lnk\"\n").unwrap();
        assert_eq!(wine.command, "env \"WINEPREFIX=/home/a/.wine\" wine \"C:\\\\Live Suite.lnk\"");
    }
}

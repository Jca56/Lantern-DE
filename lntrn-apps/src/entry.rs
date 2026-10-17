//! The apps installed here, as their `.desktop` files give them: what
//! each is called, what it runs, its icon and categories. Left out, as a
//! launcher leaves them: what isn't an application, what asks to be
//! hidden, what is for other desktops (`OnlyShowIn`, `NotShowIn` against
//! `$XDG_CURRENT_DESKTOP`) and what isn't installed (`TryExec`).

use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// The terminal a `Terminal=true` app is run in.
const TERMINAL: &str = "lntrn-terminal -e";
/// How far down an `applications` folder is looked into (Wine keeps its
/// programs a few folders deep).
const DEPTH: usize = 4;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct App {
    /// Its desktop-file id: the file's path under `applications`, the
    /// slashes dashes and `.desktop` off (`org.gnome.Nautilus`).
    pub id: String,
    pub name: String,
    /// What kind of app it is (`Web Browser`); may be empty.
    pub generic: String,
    pub comment: String,
    /// More words a search should find it by.
    pub keywords: Vec<String>,
    /// Its freedesktop categories, in the file's order.
    pub categories: Vec<String>,
    /// The program and its arguments, as the desktop runs them through
    /// the shell (see [`crate::launch::run`]).
    pub command: String,
    /// An icon theme's name for its icon, or a picture's path.
    pub icon: String,
}

/// Every app a launcher would list, by name.
pub fn installed() -> Vec<App> {
    let desktops = current_desktops();
    let mut seen = HashSet::new();
    let mut apps = Vec::new();
    for dir in dirs() {
        scan(&dir, &dir, DEPTH, &desktops, &mut seen, &mut apps);
    }
    apps.sort_by_key(|a| a.name.to_lowercase());
    apps
}

/// The names this desktop goes by (`Lantern`), from
/// `$XDG_CURRENT_DESKTOP`.
fn current_desktops() -> Vec<String> {
    std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default().split(':').filter(|d| !d.is_empty()).map(str::to_owned).collect()
}

/// The `applications` folders, the user's own first: a file there stands
/// in for one of the same id further on. Flatpak's are added when the
/// data dirs don't name them.
fn dirs() -> Vec<PathBuf> {
    let home = lntrn_sys::dirs::home().unwrap_or_default();
    let set = |name: &str| std::env::var(name).ok().filter(|v| !v.is_empty());
    let data_home = set("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".local/share"));
    let data_dirs = set("XDG_DATA_DIRS").unwrap_or_else(|| "/usr/local/share:/usr/share".to_owned());
    let mut out = vec![data_home.join("applications")];
    let flatpak = [home.join(".local/share/flatpak/exports/share"), PathBuf::from("/var/lib/flatpak/exports/share")];
    for dir in data_dirs.split(':').filter(|d| !d.is_empty()).map(PathBuf::from).chain(flatpak) {
        let dir = dir.join("applications");
        if !out.contains(&dir) {
            out.push(dir);
        }
    }
    out
}

fn scan(root: &Path, dir: &Path, depth: usize, desktops: &[String], seen: &mut HashSet<String>, apps: &mut Vec<App>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if depth > 0 {
                scan(root, &path, depth - 1, desktops, seen, apps);
            }
            continue;
        }
        if path.extension().and_then(|e| e.to_str()) != Some("desktop") {
            continue;
        }
        let id = path.strip_prefix(root).unwrap_or(&path).with_extension("").to_string_lossy().replace('/', "-");
        // The first file with an id wins, shown or not: a user's hidden
        // copy hides the system's.
        if !seen.insert(id.clone()) {
            continue;
        }
        if let Some(app) = std::fs::read_to_string(&path).ok().and_then(|text| parse(&text, desktops)) {
            apps.push(App { id, ..app });
        }
    }
}

/// The app a `.desktop` file describes (its `id` left empty): `None` for
/// one a launcher leaves out.
fn parse(text: &str, desktops: &[String]) -> Option<App> {
    let mut app = App::default();
    let (mut name, mut exec, mut try_exec) = (None, None, None);
    let (mut terminal, mut in_entry) = (false, false);
    let list = |v: &str| -> Vec<String> { v.split(';').map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned).collect() };
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
            "OnlyShowIn" if !list(value).iter().any(|d| desktops.contains(d)) => return None,
            "NotShowIn" if list(value).iter().any(|d| desktops.contains(d)) => return None,
            "Name" => name = Some(unescape(value)),
            "GenericName" => app.generic = unescape(value),
            "Comment" => app.comment = unescape(value),
            "Keywords" => app.keywords = list(&unescape(value)),
            "Categories" => app.categories = list(value),
            "Exec" => exec = Some(unescape(value)),
            "TryExec" => try_exec = Some(unescape(value)),
            "Icon" => app.icon = unescape(value),
            "Terminal" => terminal = value == "true",
            _ => {}
        }
    }
    if try_exec.is_some_and(|program| !on_path(&program)) {
        return None;
    }
    app.command = command(&exec?, terminal);
    if app.command.is_empty() {
        return None;
    }
    app.name = name.filter(|n| !n.is_empty()).unwrap_or_else(|| app.command.clone());
    Some(app)
}

/// Whether `program` is there to run: a path that exists, or a name in
/// one of `$PATH`'s folders.
fn on_path(program: &str) -> bool {
    if program.contains('/') {
        return Path::new(program).exists();
    }
    std::env::var_os("PATH").is_some_and(|path| std::env::split_paths(&path).any(|dir| dir.join(program).is_file()))
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

/// An `Exec=` as a command to run: its field codes (`%U`, the files a
/// launcher would hand it) taken out, `%%` a percent sign, and a terminal
/// put in front of an app that needs one. Quotes stay: the shell reads
/// them the way a launcher does.
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

    const LANTERN: &[&str] = &["Lantern"];

    fn here(text: &str) -> Option<App> {
        let desktops: Vec<String> = LANTERN.iter().map(|s| (*s).to_owned()).collect();
        parse(text, &desktops)
    }

    #[test]
    fn an_exec_loses_its_field_codes_and_keeps_its_quotes() {
        assert_eq!(command("firefox-bin --name=firefox-bin %u", false), "firefox-bin --name=firefox-bin");
        assert_eq!(command("\"/home/a/Ember Nights/run.sh\" %f --at 50%%", false), "\"/home/a/Ember Nights/run.sh\" --at 50%");
        assert_eq!(command("htop", true), "lntrn-terminal -e htop");
        assert_eq!(command("%U", true), "");
    }

    #[test]
    fn a_desktop_file_says_what_a_launcher_needs() {
        let app = here("[Desktop Entry]\nType=Application\nName=Web Browser\nName[de]=Internet\nGenericName=Browser\nComment=Browse the web\nKeywords=internet;www;\nCategories=Network;WebBrowser;\n# Exec=nope\nExec=firefox %u\nIcon=firefox\n\n[Desktop Action new-window]\nName=New Window\nExec=firefox --new-window\n").unwrap();
        assert_eq!((app.name.as_str(), app.generic.as_str(), app.comment.as_str(), app.command.as_str(), app.icon.as_str()), ("Web Browser", "Browser", "Browse the web", "firefox", "firefox"));
        assert_eq!((app.keywords, app.categories), (vec!["internet".to_owned(), "www".to_owned()], vec!["Network".to_owned(), "WebBrowser".to_owned()]));
    }

    #[test]
    fn what_a_launcher_hides_is_left_out() {
        assert_eq!(here("[Desktop Entry]\nName=Handler\nExec=wine start %f\nNoDisplay=true\n"), None);
        assert_eq!(here("[Desktop Entry]\nType=Link\nName=Site\nURL=https://example.org\n"), None);
        assert_eq!(here("[Desktop Entry]\nName=Nothing to run\n"), None);
        // Other desktops' own.
        assert_eq!(here("[Desktop Entry]\nName=KDE thing\nExec=k\nOnlyShowIn=KDE;\n"), None);
        assert!(here("[Desktop Entry]\nName=Ours too\nExec=k\nOnlyShowIn=KDE;Lantern;\n").is_some());
        assert_eq!(here("[Desktop Entry]\nName=Not here\nExec=k\nNotShowIn=Lantern;\n"), None);
        assert!(here("[Desktop Entry]\nName=Not GNOME\nExec=k\nNotShowIn=GNOME;\n").is_some());
        // Not installed.
        assert_eq!(here("[Desktop Entry]\nName=Gone\nExec=gone\nTryExec=/no/such/program\n"), None);
        assert!(here("[Desktop Entry]\nName=Shell\nExec=sh\nTryExec=sh\n").is_some());
    }

    #[test]
    fn wines_shortcuts_unescape_twice() {
        let wine = here("[Desktop Entry]\nName=Live\nExec=env \"WINEPREFIX=/home/a/.wine\" wine \"C:\\\\\\\\Live\\sSuite.lnk\"\n").unwrap();
        assert_eq!(wine.command, "env \"WINEPREFIX=/home/a/.wine\" wine \"C:\\\\Live Suite.lnk\"");
    }

    /// What this machine has, printed: `cargo test -p lntrn-apps -- --ignored --nocapture`.
    #[test]
    #[ignore = "reads this machine's .desktop files"]
    fn what_is_installed_here_is_listed() {
        let apps = installed();
        for app in &apps {
            println!("{:<34} {:<36} {:<24} {:?}", app.name, app.id, app.icon, app.categories);
        }
        assert!(apps.windows(2).all(|w| w[0].name.to_lowercase() <= w[1].name.to_lowercase()));
    }
}

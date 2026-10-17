//! XDG autostart: the `.desktop` entries in the autostart directories that
//! apply to this desktop, started once the shell is up; but none for a
//! program the session runs itself, which would run twice (a leftover
//! entry for the Command Center used to start it beside the bar).

use std::collections::{HashMap, HashSet};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::children::Children;
use crate::log::log;
use crate::paths;

/// An entry that is to be started.
#[derive(Debug, PartialEq)]
struct Entry {
    /// Its file name without `.desktop`.
    name: String,
    /// Its command line, for `sh -c`.
    exec: String,
}

/// Start every autostart entry that applies to this session, but those
/// for a program in `own` (the session's daemons, either panel).
pub fn run(children: &mut Children, own: &[&str]) {
    let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
    let desktops: Vec<&str> = desktop.split(':').filter(|d| !d.is_empty()).collect();
    let path = std::env::var("PATH").unwrap_or_default();

    for entry in entries(&dirs(), &desktops, &path) {
        if let Some(program) = program_of(&entry.exec).filter(|p| own.contains(&p.as_str())) {
            log(&format!("🏮 Autostart: skipping {}: the session runs {program} itself", entry.name));
            continue;
        }
        log(&format!("🏮 Autostart: {}", entry.exec));
        match Command::new("sh")
            .args(["-c", &entry.exec])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(child) => children.adopt_autostart(child.id() as i32, entry.name),
            Err(e) => log(&format!("🏮 WARNING: could not start {}: {e}", entry.name)),
        }
    }
}

/// The autostart directories, the one that wins first: the user's, then the
/// system's.
fn dirs() -> Vec<PathBuf> {
    let set = |name: &str| std::env::var_os(name).filter(|value| !value.is_empty());
    let config_home = set("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| paths::home().join(".config"));
    let config_dirs = set("XDG_CONFIG_DIRS").unwrap_or_else(|| "/etc/xdg".into());

    std::iter::once(config_home)
        .chain(std::env::split_paths(&config_dirs))
        .map(|dir| dir.join("autostart"))
        .collect()
}

/// The entries in `dirs` to start, for a session whose desktop names are
/// `desktops` and whose `$PATH` is `path`.
///
/// A file name counts once, where it comes first: an entry in the user's
/// directory replaces the system's of the same name, which is how one is
/// switched off (`Hidden=true`).
fn entries(dirs: &[PathBuf], desktops: &[&str], path: &str) -> Vec<Entry> {
    let mut seen = HashSet::new();
    let mut found = Vec::new();
    for dir in dirs {
        let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|entry| entry.path())
            .filter(|file| file.extension().is_some_and(|ext| ext == "desktop"))
            .collect();
        files.sort();

        for file in files {
            let Some(name) = file.file_stem().and_then(|stem| stem.to_str()) else {
                continue;
            };
            if !seen.insert(name.to_owned()) {
                continue;
            }
            let Ok(content) = std::fs::read_to_string(&file) else {
                continue;
            };
            if let Some(exec) = exec_of(&content, desktops, path) {
                found.push(Entry { name: name.to_owned(), exec });
            }
        }
    }
    found
}

/// The command an entry's text asks for, if it is to be started here at all.
fn exec_of(content: &str, desktops: &[&str], path: &str) -> Option<String> {
    let keys = desktop_entry(content);
    let get = |key: &str| keys.get(key).copied();
    let is = |key: &str, value: &str| get(key).is_some_and(|v| v.eq_ignore_ascii_case(value));
    let lists_this_desktop = |list: &str| {
        list.split(';')
            .map(str::trim)
            .any(|name| desktops.iter().any(|desktop| desktop.eq_ignore_ascii_case(name)))
    };

    if get("Type") != Some("Application") {
        return None;
    }
    if is("Hidden", "true") || is("X-GNOME-Autostart-enabled", "false") {
        return None;
    }
    if get("OnlyShowIn").is_some_and(|only| !lists_this_desktop(only)) {
        return None;
    }
    if get("NotShowIn").is_some_and(lists_this_desktop) {
        return None;
    }
    if get("TryExec").is_some_and(|program| !is_installed(program, path)) {
        return None;
    }
    let exec = without_field_codes(get("Exec")?);
    (!exec.is_empty()).then_some(exec)
}

/// The keys of a desktop file's `[Desktop Entry]` group.
fn desktop_entry(content: &str) -> HashMap<&str, &str> {
    let mut keys = HashMap::new();
    let mut in_group = false;
    for line in content.lines().map(str::trim) {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') {
            in_group = line == "[Desktop Entry]";
        } else if in_group {
            if let Some((key, value)) = line.split_once('=') {
                keys.insert(key.trim_end(), value.trim_start());
            }
        }
    }
    keys
}

/// The program an `Exec` runs, by its file name: past `env` and the
/// variables it sets (`env A=1 lntrn-command-center --hide` runs
/// `lntrn-command-center`).
fn program_of(exec: &str) -> Option<String> {
    let mut words = exec.split_whitespace().skip_while(|w| *w == "env" || (w.contains('=') && !w.starts_with('-')));
    let first = words.next()?.trim_matches(['"', '\'']);
    Path::new(first).file_name()?.to_str().map(str::to_owned)
}

/// `Exec` with its field codes taken out. Nothing is being opened at login,
/// so `%f`, `%U` and the rest stand for nothing; `%%` is a percent sign.
fn without_field_codes(exec: &str) -> String {
    let mut out = String::with_capacity(exec.len());
    let mut chars = exec.chars();
    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
        } else if chars.next() == Some('%') {
            out.push('%');
        }
    }
    out.trim().to_owned()
}

/// Whether `program` (a `TryExec`) is there to run: a path as it is, a bare
/// name by looking through `path`.
fn is_installed(program: &str, path: &str) -> bool {
    let runs = |file: &Path| {
        std::fs::metadata(file).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    };
    if program.contains('/') {
        return runs(Path::new(program));
    }
    std::env::split_paths(path).any(|dir| runs(&dir.join(program)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testdir::TestDir;

    const LANTERN: &[&str] = &["Lantern"];

    fn exec(content: &str) -> Option<String> {
        exec_of(content, LANTERN, "")
    }

    #[test]
    fn the_program_is_named_past_env_and_a_path() {
        assert_eq!(program_of("lntrn-command-center --hide").as_deref(), Some("lntrn-command-center"));
        assert_eq!(program_of("/home/a/.lantern/bin/lntrn-notifyd").as_deref(), Some("lntrn-notifyd"));
        assert_eq!(program_of("env GDK_BACKEND=wayland A=1 steam -silent").as_deref(), Some("steam"));
        assert_eq!(program_of("\"/opt/app/run\" --flag").as_deref(), Some("run"));
        assert_eq!(program_of("   "), None);
    }

    #[test]
    fn an_application_entry_gives_its_exec() {
        let entry = "[Desktop Entry]\nType=Application\nName=CC\nExec=lntrn-command-center --hide\n";
        assert_eq!(exec(entry).as_deref(), Some("lntrn-command-center --hide"));
        // Spaces around `=`, comments and blank lines are all allowed.
        let spaced = "# hi\n\n[Desktop Entry]\nType = Application\nExec = thing --flag\n";
        assert_eq!(exec(spaced).as_deref(), Some("thing --flag"));
        assert_eq!(exec("[Desktop Entry]\nType=Link\nExec=thing\n"), None);
        assert_eq!(exec("[Desktop Entry]\nType=Application\n"), None);
    }

    #[test]
    fn only_the_desktop_entry_group_counts() {
        let entry = "[Desktop Entry]\nType=Application\nExec=real\n\n\
                     [Desktop Action New]\nExec=action\nHidden=true\n";
        assert_eq!(exec(entry).as_deref(), Some("real"));
    }

    #[test]
    fn switched_off_entries_are_skipped() {
        let base = "[Desktop Entry]\nType=Application\nExec=thing\n";
        assert!(exec(base).is_some());
        assert_eq!(exec(&format!("{base}Hidden=true\n")), None);
        assert_eq!(exec(&format!("{base}X-GNOME-Autostart-enabled=false\n")), None);
        assert!(exec(&format!("{base}X-GNOME-Autostart-enabled=true\n")).is_some());
    }

    #[test]
    fn show_in_lists_are_checked_against_every_desktop_name() {
        let base = "[Desktop Entry]\nType=Application\nExec=thing\n";
        assert!(exec(&format!("{base}OnlyShowIn=Lantern;\n")).is_some());
        assert_eq!(exec(&format!("{base}OnlyShowIn=GNOME;Unity;\n")), None);
        assert_eq!(exec(&format!("{base}NotShowIn=KDE;Lantern;\n")), None);
        assert!(exec(&format!("{base}NotShowIn=KDE;\n")).is_some());
        // XDG_CURRENT_DESKTOP may hold several names.
        let only_gnome = format!("{base}OnlyShowIn=GNOME;\n");
        assert!(exec_of(&only_gnome, &["Lantern", "GNOME"], "").is_some());
    }

    #[test]
    fn field_codes_are_dropped() {
        assert_eq!(without_field_codes("firefox %U"), "firefox");
        assert_eq!(without_field_codes("app --file %f --quiet"), "app --file  --quiet");
        assert_eq!(without_field_codes("printf 100%%"), "printf 100%");
        assert_eq!(without_field_codes("odd %"), "odd");
    }

    #[test]
    fn try_exec_has_to_be_runnable() {
        let dir = TestDir::new("tryexec");
        let bin = dir.path().join("bin");
        std::fs::create_dir(&bin).unwrap();
        let tool = bin.join("tool");
        std::fs::write(&tool, "#!/bin/sh\n").unwrap();
        std::fs::write(bin.join("notes.txt"), "").unwrap();
        std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o755)).unwrap();
        let path = format!("/nonexistent:{}", bin.display());

        assert!(is_installed("tool", &path));
        assert!(is_installed(tool.to_str().unwrap(), ""));
        assert!(!is_installed("tool", "/nonexistent"));
        assert!(!is_installed("notes.txt", &path), "not executable");
        assert!(!is_installed("bin", &dir.path().display().to_string()), "a directory");

        let entry = "[Desktop Entry]\nType=Application\nExec=tool\nTryExec=tool\n";
        assert!(exec_of(entry, LANTERN, &path).is_some());
        assert_eq!(exec_of(entry, LANTERN, "/nonexistent"), None);
    }

    #[test]
    fn the_users_entry_replaces_the_systems_of_the_same_name() {
        let dir = TestDir::new("override");
        let user = dir.path().join("user/autostart");
        let system = dir.path().join("system/autostart");
        std::fs::create_dir_all(&user).unwrap();
        std::fs::create_dir_all(&system).unwrap();
        let app = |exec: &str| format!("[Desktop Entry]\nType=Application\nExec={exec}\n");

        std::fs::write(system.join("zeta.desktop"), app("zeta")).unwrap();
        std::fs::write(system.join("tracker.desktop"), app("tracker")).unwrap();
        std::fs::write(system.join("panel.desktop"), app("system-panel")).unwrap();
        std::fs::write(system.join("readme.txt"), app("not-an-entry")).unwrap();
        // Switched off by the user, and replaced by the user.
        std::fs::write(user.join("tracker.desktop"), "[Desktop Entry]\nHidden=true\n").unwrap();
        std::fs::write(user.join("panel.desktop"), app("my-panel")).unwrap();

        let found = entries(&[user, system, dir.path().join("missing")], LANTERN, "");
        let found: Vec<(&str, &str)> =
            found.iter().map(|e| (e.name.as_str(), e.exec.as_str())).collect();
        assert_eq!(found, [("panel", "my-panel"), ("zeta", "zeta")]);
    }
}

//! What Run would run, worked out from the file being edited and the
//! project around it: a Python file goes to the interpreter (the
//! project's virtual environment's when it has one), anything in a Rust
//! crate that has a program goes to `cargo run` there.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

/// A program to start and where.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plan {
    /// What is being run, for the button: the file, or the crate.
    pub label: String,
    /// The program and its arguments, run as they are.
    pub argv: Vec<OsString>,
    /// The folder it runs in.
    pub cwd: PathBuf,
    /// The command as one would type it, for the Run editor's header.
    pub shown: String,
}

impl Plan {
    fn new(label: String, argv: Vec<OsString>, cwd: PathBuf) -> Self {
        // The program by its name alone: a virtual environment's path is long.
        let shown = argv.iter().enumerate().map(|(i, a)| if i == 0 { name_of(Path::new(a)) } else { a.to_string_lossy().into_owned() }).collect::<Vec<_>>().join(" ");
        Self { label, argv, cwd, shown }
    }

    /// A command the user wrote for the project: the shell reads it, in
    /// the project's folder.
    pub fn custom(command: &str, root: &Path) -> Self {
        let shell = std::env::var_os("SHELL").filter(|s| !s.is_empty()).unwrap_or_else(|| "/bin/sh".into());
        Self { label: name_of(root), argv: vec![shell, "-c".into(), command.into()], cwd: root.to_path_buf(), shown: command.to_owned() }
    }

    /// Whether the program is there to start: a path that is a file, or
    /// a name some folder of the `PATH` has.
    pub fn program_found(&self) -> bool {
        let Some(program) = self.argv.first() else { return false };
        let path = Path::new(program);
        if path.components().count() > 1 {
            return path.is_file();
        }
        on_path(program)
    }

    /// The program's name, for saying it is missing.
    pub fn program(&self) -> String {
        self.argv.first().map(|p| name_of(Path::new(p))).unwrap_or_default()
    }
}

fn name_of(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| p.display().to_string())
}

fn on_path(program: &OsStr) -> bool {
    std::env::var_os("PATH").is_some_and(|paths| std::env::split_paths(&paths).any(|dir| dir.join(program).is_file()))
}

/// What to run for `file` (the one being edited) in the project at
/// `root`: the file itself when it is a program, else the crate it is
/// in, else what the project's folder holds.
pub fn detect(file: Option<&Path>, root: Option<&Path>) -> Option<Plan> {
    if let Some(f) = file {
        if is_python(f) {
            return Some(python(f, root));
        }
        if let Some(plan) = f.parent().and_then(|dir| dir.ancestors().find_map(|d| cargo(d, Some(f)))) {
            return Some(plan);
        }
    }
    let root = root?;
    cargo(root, None).or_else(|| {
        let main = root.join("main.py");
        main.is_file().then(|| python(&main, Some(root)))
    })
}

fn is_python(file: &Path) -> bool {
    file.extension().is_some_and(|e| e == "py")
}

/// `file` run where it sits, the way `python main.py` is typed there, so
/// what it opens beside itself is found.
fn python(file: &Path, root: Option<&Path>) -> Plan {
    let dir = file.parent().unwrap_or(Path::new("/")).to_path_buf();
    let name: OsString = file.file_name().unwrap_or(file.as_os_str()).into();
    Plan::new(name.to_string_lossy().into_owned(), vec![interpreter(&dir, root), name], dir)
}

/// The Python of the virtual environment nearest `dir`, looking no
/// higher than the project; else the system's.
fn interpreter(dir: &Path, root: Option<&Path>) -> OsString {
    let inside = |d: &Path| root.is_some_and(|r| d.starts_with(r));
    for d in dir.ancestors().enumerate().take_while(|(i, d)| *i == 0 || inside(d)).map(|(_, d)| d) {
        for venv in [".venv", "venv"] {
            let python = d.join(venv).join("bin/python");
            if python.is_file() {
                return python.into();
            }
        }
    }
    if on_path(OsStr::new("python3")) { "python3".into() } else { "python".into() }
}

/// `cargo run` for the crate whose `Cargo.toml` is in `dir`, when it has
/// a program to run: `file`'s own when it is one of the crate's examples
/// or extra binaries, else the crate's main one.
fn cargo(dir: &Path, file: Option<&Path>) -> Option<Plan> {
    let manifest = std::fs::read_to_string(dir.join("Cargo.toml")).ok()?;
    let name = package_name(&manifest)?;
    let mut argv: Vec<OsString> = vec!["cargo".into(), "run".into()];
    let own = file.filter(|f| f.extension().is_some_and(|e| e == "rs")).and_then(|f| Some((f.parent()?, f.file_stem()?)));
    match own {
        Some((parent, stem)) if parent == dir.join("examples") => argv.extend(["--example".into(), stem.to_owned()]),
        Some((parent, stem)) if parent == dir.join("src/bin") => argv.extend(["--bin".into(), stem.to_owned()]),
        _ => {
            let has_bin = dir.join("src/main.rs").is_file() || dir.join("src/bin").is_dir() || manifest.lines().any(|l| l.trim() == "[[bin]]");
            if !has_bin {
                return None;
            }
        }
    }
    Some(Plan::new(name, argv, dir.to_path_buf()))
}

/// The `name` of a manifest's `[package]`; `None` for a workspace that
/// is no package itself.
fn package_name(manifest: &str) -> Option<String> {
    let mut in_package = false;
    for line in manifest.lines() {
        let l = line.trim();
        if l.starts_with('[') {
            in_package = l == "[package]";
        } else if in_package
            && let Some((k, v)) = l.split_once('=')
            && k.trim() == "name"
        {
            return Some(v.trim().trim_matches(['"', '\'']).to_owned());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("lntrn-code-run-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    fn args(plan: &Plan) -> Vec<String> {
        plan.argv.iter().map(|a| a.to_string_lossy().into_owned()).collect()
    }

    #[test]
    fn python_files_and_their_virtual_environment() {
        let dir = scratch("py");
        write(&dir.join("main.py"), "print('hi')\n");
        write(&dir.join("game/level one.py"), "");
        write(&dir.join("notes.md"), "");
        // The open file runs where it sits.
        let plan = detect(Some(&dir.join("game/level one.py")), Some(&dir)).unwrap();
        assert_eq!((plan.label.as_str(), plan.cwd.as_path()), ("level one.py", dir.join("game").as_path()));
        assert_eq!(args(&plan)[1], "level one.py");
        assert!(matches!(args(&plan)[0].as_str(), "python3" | "python"), "the system's Python: {:?}", plan.argv);
        // With no program open, the project's main.py.
        for file in [None, Some(dir.join("notes.md"))] {
            let plan = detect(file.as_deref(), Some(&dir)).unwrap();
            assert_eq!((plan.label.as_str(), plan.cwd.as_path()), ("main.py", dir.as_path()));
        }
        // A virtual environment in the project is the Python that has its packages.
        write(&dir.join(".venv/bin/python"), "");
        let plan = detect(Some(&dir.join("game/level one.py")), Some(&dir)).unwrap();
        assert_eq!(Path::new(&plan.argv[0]), dir.join(".venv/bin/python"));
        assert_eq!(plan.shown, "python level one.py");
        assert!(plan.program_found());
        // One above the project is somebody else's.
        let plan = detect(Some(&dir.join("game/level one.py")), Some(&dir.join("game"))).unwrap();
        assert!(Path::new(&plan.argv[0]).components().count() == 1, "{:?}", plan.argv);
        assert_eq!(detect(Some(&dir.join("notes.md")), Some(&dir.join("game"))), None, "nothing to run there");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rust_crates() {
        let dir = scratch("rs");
        write(&dir.join("Cargo.toml"), "[workspace]\nmembers = [\"app\", \"lib\"]\n");
        write(&dir.join("app/Cargo.toml"), "[package]\nname = \"demo-app\"\nversion = \"0.1.0\"\n\n[dependencies]\nname = \"1\"\n");
        write(&dir.join("app/src/main.rs"), "fn main() {}\n");
        write(&dir.join("app/src/ui/panel.rs"), "");
        write(&dir.join("app/src/bin/tool.rs"), "fn main() {}\n");
        write(&dir.join("app/examples/hello.rs"), "fn main() {}\n");
        write(&dir.join("app/README.md"), "");
        write(&dir.join("lib/Cargo.toml"), "[package]\nname = 'demo-lib'\n");
        write(&dir.join("lib/src/lib.rs"), "");
        // Any file of a crate runs the crate, from the crate's folder.
        for file in ["app/src/ui/panel.rs", "app/README.md", "app/Cargo.toml"] {
            let plan = detect(Some(&dir.join(file)), Some(&dir)).unwrap();
            assert_eq!((plan.label.as_str(), plan.shown.as_str(), plan.cwd.as_path()), ("demo-app", "cargo run", dir.join("app").as_path()), "{file}");
        }
        // An example or an extra binary runs itself.
        assert_eq!(detect(Some(&dir.join("app/examples/hello.rs")), Some(&dir)).unwrap().shown, "cargo run --example hello");
        assert_eq!(detect(Some(&dir.join("app/src/bin/tool.rs")), Some(&dir)).unwrap().shown, "cargo run --bin tool");
        // A library has nothing to run, and neither has a bare workspace.
        assert_eq!(detect(Some(&dir.join("lib/src/lib.rs")), Some(&dir)), None);
        assert_eq!(detect(None, Some(&dir)), None);
        // The project's own crate, with nothing open.
        assert_eq!(detect(None, Some(&dir.join("app"))).unwrap().label, "demo-app");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn custom_commands_go_through_the_shell() {
        let plan = Plan::custom("make deploy-lntrn-code && lntrn-code", Path::new("/home/x/Lantern-DE"));
        assert_eq!((plan.label.as_str(), plan.shown.as_str()), ("Lantern-DE", "make deploy-lntrn-code && lntrn-code"));
        assert_eq!(args(&plan)[1..], ["-c", "make deploy-lntrn-code && lntrn-code"]);
        assert!(plan.program_found(), "the shell is there");
        let gone = Plan::new("x".into(), vec!["lntrn-no-such-program".into()], PathBuf::from("/"));
        assert!(!gone.program_found());
        assert_eq!(gone.program(), "lntrn-no-such-program");
    }
}

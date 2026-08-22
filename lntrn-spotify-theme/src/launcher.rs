//! Writes a user-level `spotify.desktop` that launches Spotify with
//! `--enable-transparent-visuals`, so the CEF window carries an alpha channel
//! and the compositor's blur shows through. Lives in `~/.local/share`, which
//! XDG-shadows the packaged desktop file and survives Spotify updates.

use std::path::{Path, PathBuf};

/// CEF flags. `--enable-transparent-visuals` is the one that matters; if a
/// driver refuses alpha with GPU compositing, append `--disable-gpu` here.
const FLAGS: &str = "--enable-transparent-visuals --no-zygote";
const REAL_BIN: &str = "/opt/spotify/spotify-client/spotify";

const DESKTOP_NAMES: &[&str] = &[
    "spotify.desktop",
    "com.spotify.Client.desktop",
    "spotify-client.desktop",
];

fn data_home() -> PathBuf {
    std::env::var("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let home = std::env::var("HOME").unwrap_or_else(|_| "/root".into());
            PathBuf::from(home).join(".local/share")
        })
}

/// Locate the packaged spotify desktop file to mirror its metadata.
fn system_desktop() -> Option<PathBuf> {
    for d in ["/usr/share/applications", "/usr/local/share/applications"] {
        for name in DESKTOP_NAMES {
            let p = Path::new(d).join(name);
            if p.exists() {
                return Some(p);
            }
        }
    }
    None
}

/// Install the override desktop file. Returns the written path.
pub fn install() -> Result<PathBuf, String> {
    let apps = data_home().join("applications");
    std::fs::create_dir_all(&apps).map_err(|e| format!("mkdir {}: {e}", apps.display()))?;

    let (file_name, contents) = match system_desktop() {
        Some(sys) => {
            let name = sys.file_name().unwrap().to_string_lossy().into_owned();
            let body = std::fs::read_to_string(&sys)
                .map_err(|e| format!("read {}: {e}", sys.display()))?;
            (name, rewrite_exec(&body))
        }
        None => ("spotify.desktop".to_string(), fresh_desktop()),
    };

    let target = apps.join(&file_name);
    std::fs::write(&target, contents).map_err(|e| format!("write {}: {e}", target.display()))?;
    Ok(target)
}

/// Remove any override desktop file we installed.
pub fn uninstall() -> Result<(), String> {
    let apps = data_home().join("applications");
    for name in DESKTOP_NAMES {
        let p = apps.join(name);
        if p.exists() {
            std::fs::remove_file(&p).map_err(|e| format!("remove {}: {e}", p.display()))?;
        }
    }
    Ok(())
}

/// True if our override desktop file is currently installed.
pub fn installed() -> bool {
    let apps = data_home().join("applications");
    DESKTOP_NAMES.iter().any(|n| apps.join(n).exists())
}

/// Rewrite each `Exec=` line to run the real binary with the transparency flag,
/// preserving the original field code (`%U`, `%f`, …).
fn rewrite_exec(body: &str) -> String {
    let mut out = String::with_capacity(body.len() + 80);
    out.push_str("# Overridden by lntrn-spotify-theme for transparent visuals.\n");
    for line in body.lines() {
        if let Some(rest) = line.strip_prefix("Exec=") {
            let field = rest
                .rsplit(' ')
                .find(|t| t.starts_with('%'))
                .unwrap_or("%U");
            out.push_str(&format!("Exec={REAL_BIN} {FLAGS} {field}\n"));
        } else {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

fn fresh_desktop() -> String {
    format!(
        "[Desktop Entry]\n\
         # Created by lntrn-spotify-theme for transparent visuals.\n\
         Type=Application\n\
         Name=Spotify\n\
         GenericName=Music Player\n\
         Icon=spotify-client\n\
         Exec={REAL_BIN} {FLAGS} %U\n\
         Terminal=false\n\
         MimeType=x-scheme-handler/spotify;\n\
         Categories=Audio;Music;Player;AudioVideo;\n"
    )
}

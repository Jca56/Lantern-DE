use std::path::PathBuf;
use super::app::{App, ClipboardOp, dirs_home};

fn trash_dir() -> PathBuf {
    dirs_home().join(".local/share/Trash")
}

/// Simple ISO-ish timestamp for trash info (no chrono crate).
fn chrono_now() -> String {
    let dur = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = dur.as_secs();
    let days = secs / 86400;
    let time_of_day = secs % 86400;
    let hours = time_of_day / 3600;
    let mins = (time_of_day % 3600) / 60;
    let s = time_of_day % 60;
    let mut y = 1970u64;
    let mut remaining = days;
    loop {
        let leap = y % 4 == 0 && (y % 100 != 0 || y % 400 == 0);
        let year_days = if leap { 366 } else { 365 };
        if remaining < year_days { break; }
        remaining -= year_days;
        y += 1;
    }
    let leap = y % 4 == 0 && (y % 100 != 0 || y % 400 == 0);
    let month_days = [31, if leap { 29 } else { 28 }, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let mut m = 0usize;
    for &md in &month_days {
        if remaining < md { break; }
        remaining -= md;
        m += 1;
    }
    format!("{y}-{:02}-{:02}T{hours:02}:{mins:02}:{s:02}", m + 1, remaining + 1)
}

pub fn copy_dir_recursive(src: &std::path::Path, dst: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let target = dst.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir_recursive(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

/// Returns true if the path looks like an extractable archive.
pub fn is_archive(path: &std::path::Path) -> bool {
    let name = path.to_string_lossy().to_lowercase();
    name.ends_with(".tar.gz") || name.ends_with(".tgz")
        || name.ends_with(".tar.bz2") || name.ends_with(".tbz2")
        || name.ends_with(".tar.xz") || name.ends_with(".txz")
        || name.ends_with(".tar")
        || name.ends_with(".zip")
        || name.ends_with(".7z")
}

/// File operation methods for App.
impl App {
    pub fn copy_selected(&mut self) {
        let paths = self.selected_paths();
        if !paths.is_empty() {
            self.clipboard = Some(ClipboardOp::Copy(paths));
        }
    }

    pub fn cut_selected(&mut self) {
        let paths = self.selected_paths();
        if !paths.is_empty() {
            self.clipboard = Some(ClipboardOp::Cut(paths));
        }
    }

    pub fn paste(&mut self) {
        let Some(op) = self.clipboard.take() else { return };
        let dest = &self.current_dir;
        if self.root_mode {
            match op {
                ClipboardOp::Copy(paths) => {
                    for src in &paths {
                        let name = src.file_name().unwrap_or_default();
                        let target = dest.join(name);
                        let src = src.clone();
                        std::thread::spawn(move || {
                            let _ = std::process::Command::new("pkexec")
                                .args(["cp", "-r", "--"])
                                .arg(&src).arg(&target)
                                .status();
                        });
                    }
                    self.clipboard = Some(ClipboardOp::Copy(paths));
                }
                ClipboardOp::Cut(paths) => {
                    for src in &paths {
                        let name = src.file_name().unwrap_or_default();
                        let target = dest.join(name);
                        let src = src.clone();
                        std::thread::spawn(move || {
                            let _ = std::process::Command::new("pkexec")
                                .args(["mv", "--"])
                                .arg(&src).arg(&target)
                                .status();
                        });
                    }
                }
            }
        } else {
            match op {
                ClipboardOp::Copy(paths) => {
                    let mut created = Vec::new();
                    for src in &paths {
                        let name = src.file_name().unwrap_or_default();
                        let target = dest.join(name);
                        let ok = if src.is_dir() {
                            copy_dir_recursive(src, &target).is_ok()
                        } else {
                            std::fs::copy(src, &target).is_ok()
                        };
                        if ok { created.push(target); }
                    }
                    if !created.is_empty() {
                        self.undo_stack.push(crate::undo::UndoAction::Copy {
                            sources: paths.clone(), created,
                        });
                    }
                    self.clipboard = Some(ClipboardOp::Copy(paths));
                }
                ClipboardOp::Cut(paths) => {
                    let mut moves = Vec::new();
                    for src in &paths {
                        let name = src.file_name().unwrap_or_default();
                        let target = dest.join(name);
                        if std::fs::rename(src, &target).is_ok() {
                            moves.push((src.clone(), target));
                        }
                    }
                    if !moves.is_empty() {
                        self.undo_stack.push(crate::undo::UndoAction::Move(moves));
                    }
                }
            }
        }
        self.reload();
    }

    pub fn trash_selected(&mut self) {
        if self.root_mode {
            self.delete_selected();
            return;
        }
        let trash_dir = trash_dir();
        let trash_info_dir = trash_dir.join("info");
        let trash_files_dir = trash_dir.join("files");
        let _ = std::fs::create_dir_all(&trash_info_dir);
        let _ = std::fs::create_dir_all(&trash_files_dir);

        let mut undo_entries = Vec::new();
        for entry in &self.entries {
            if !entry.selected { continue; }
            let name = entry.path.file_name().unwrap_or_default().to_string_lossy().to_string();

            let mut dest_name = name.clone();
            let mut counter = 1u32;
            while trash_files_dir.join(&dest_name).exists() {
                let stem = std::path::Path::new(&name).file_stem()
                    .map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
                let ext = std::path::Path::new(&name).extension()
                    .map(|s| format!(".{}", s.to_string_lossy())).unwrap_or_default();
                dest_name = format!("{stem}.{counter}{ext}");
                counter += 1;
            }

            let now = chrono_now();
            let info_content = format!(
                "[Trash Info]\nPath={}\nDeletionDate={}\n",
                entry.path.display(), now
            );
            let info_path = trash_info_dir.join(format!("{dest_name}.trashinfo"));
            let file_path = trash_files_dir.join(&dest_name);
            let _ = std::fs::write(&info_path, info_content);
            if std::fs::rename(&entry.path, &file_path).is_ok() {
                undo_entries.push((entry.path.clone(), file_path, info_path));
            }
        }
        if !undo_entries.is_empty() {
            self.undo_stack.push(crate::undo::UndoAction::Trash(undo_entries));
        }
        self.reload();
    }

    pub fn delete_selected(&mut self) {
        if self.root_mode {
            let paths: Vec<PathBuf> = self.entries.iter()
                .filter(|e| e.selected)
                .map(|e| e.path.clone())
                .collect();
            std::thread::spawn(move || {
                for path in &paths {
                    let _ = std::process::Command::new("pkexec")
                        .args(["rm", "-rf", "--"])
                        .arg(path)
                        .status();
                }
            });
        } else {
            for entry in &self.entries {
                if !entry.selected { continue; }
                if entry.is_dir {
                    let _ = std::fs::remove_dir_all(&entry.path);
                } else {
                    let _ = std::fs::remove_file(&entry.path);
                }
            }
        }
        self.reload();
    }

    pub fn open_selected(&mut self) {
        let selected: Vec<_> = self.entries.iter()
            .enumerate()
            .filter(|(_, e)| e.selected)
            .map(|(i, _)| i)
            .collect();
        if selected.len() == 1 {
            let entry = &self.entries[selected[0]];
            if entry.is_dir {
                let path = entry.path.clone();
                self.navigate_to(path);
                return;
            }
        }
        for &i in &selected {
            let path = self.entries[i].path.clone();
            std::thread::spawn(move || {
                let _ = std::process::Command::new("xdg-open").arg(&path).spawn();
            });
        }
    }

    #[allow(dead_code)]
    pub fn open_with(&self, app_name: &str) {
        for entry in &self.entries {
            if !entry.selected { continue; }
            let path = entry.path.clone();
            let app = app_name.to_string();
            std::thread::spawn(move || {
                let _ = std::process::Command::new(&app).arg(&path).spawn();
            });
        }
    }

    #[allow(dead_code)]
    pub fn copy_path_to_clipboard(&self) {
        let paths: Vec<String> = self.entries.iter()
            .filter(|e| e.selected)
            .map(|e| e.path.display().to_string())
            .collect();
        if paths.is_empty() { return; }
        let text = paths.join("\n");
        if let Some(clip) = &self.wayland_clipboard {
            clip.set_text(&text);
        }
    }

    #[allow(dead_code)]
    pub fn copy_name_to_clipboard(&self) {
        let names: Vec<String> = self.entries.iter()
            .filter(|e| e.selected)
            .map(|e| e.name.clone())
            .collect();
        if names.is_empty() { return; }
        let text = names.join("\n");
        if let Some(clip) = &self.wayland_clipboard {
            clip.set_text(&text);
        }
    }

    pub fn duplicate_selected(&mut self) {
        let selected: Vec<_> = self.entries.iter()
            .filter(|e| e.selected)
            .map(|e| (e.path.clone(), e.name.clone(), e.is_dir))
            .collect();
        let root_mode = self.root_mode;
        for (path, name, is_dir) in selected {
            let parent = path.parent().unwrap_or(&self.current_dir).to_path_buf();
            let stem = std::path::Path::new(&name).file_stem()
                .map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
            let ext = std::path::Path::new(&name).extension()
                .map(|s| format!(".{}", s.to_string_lossy())).unwrap_or_default();

            let mut dest_name = format!("{stem} (copy){ext}");
            let mut counter = 2u32;
            while parent.join(&dest_name).exists() {
                dest_name = format!("{stem} (copy {counter}){ext}");
                counter += 1;
            }
            let dest = parent.join(&dest_name);
            if root_mode {
                let src = path.clone();
                let d = dest.clone();
                std::thread::spawn(move || {
                    let _ = std::process::Command::new("pkexec")
                        .args(["cp", "-r", "--"])
                        .arg(&src).arg(&d)
                        .status();
                });
            } else if is_dir {
                let src = path.clone();
                let d = dest.clone();
                std::thread::spawn(move || { let _ = copy_dir_recursive(&src, &d); });
            } else {
                let _ = std::fs::copy(&path, &dest);
            }
            self.undo_stack.push(crate::undo::UndoAction::Copy {
                sources: vec![path], created: vec![dest],
            });
        }
        self.reload();
    }

    pub fn compress_selected(&mut self) {
        let selected: Vec<PathBuf> = self.entries.iter()
            .filter(|e| e.selected)
            .map(|e| e.path.clone())
            .collect();
        if selected.is_empty() { return; }

        let base_name = selected[0].file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "archive".into());
        let mut archive_name = format!("{base_name}.tar.gz");
        let mut counter = 2u32;
        while self.current_dir.join(&archive_name).exists() {
            archive_name = format!("{base_name} ({counter}).tar.gz");
            counter += 1;
        }

        let dir = self.current_dir.clone();
        let root_mode = self.root_mode;
        std::thread::spawn(move || {
            let file_args: Vec<String> = selected.iter()
                .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
                .collect();
            if root_mode {
                let mut args = vec!["tar".to_string(), "czf".to_string(), archive_name];
                args.extend(file_args);
                let _ = std::process::Command::new("pkexec")
                    .args(&args)
                    .current_dir(&dir)
                    .status();
            } else {
                let _ = std::process::Command::new("tar")
                    .arg("czf").arg(&archive_name)
                    .args(&file_args)
                    .current_dir(&dir)
                    .status();
            }
        });
    }

    pub fn extract_selected(&mut self) {
        let selected: Vec<PathBuf> = self.entries.iter()
            .filter(|e| e.selected)
            .map(|e| e.path.clone())
            .collect();
        let dir = self.current_dir.clone();
        let root_mode = self.root_mode;
        std::thread::spawn(move || {
            for path in &selected {
                let ext = path.extension()
                    .map(|e| e.to_string_lossy().to_lowercase())
                    .unwrap_or_default();
                let name = path.to_string_lossy();

                // Derive subfolder name from archive filename (strip extensions)
                let stem = {
                    let file_name = path.file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_default();
                    // Strip compound extensions like .tar.gz, .tar.bz2, etc.
                    let s = file_name.as_str();
                    if s.ends_with(".tar.gz") || s.ends_with(".tar.bz2") || s.ends_with(".tar.xz") {
                        s.rsplitn(3, '.').last().unwrap_or(s).to_string()
                    } else {
                        std::path::Path::new(&file_name).file_stem()
                            .map(|s| s.to_string_lossy().to_string())
                            .unwrap_or(file_name)
                    }
                };
                let extract_dir = dir.join(&stem);
                let _ = std::fs::create_dir_all(&extract_dir);

                // Build (program, args) for each archive type
                let (prog, args): (&str, Vec<std::ffi::OsString>) = if name.ends_with(".tar.gz") || name.ends_with(".tgz") {
                    ("tar", vec!["xzf".into(), path.as_os_str().into(), "-C".into(), extract_dir.as_os_str().into()])
                } else if name.ends_with(".tar.bz2") || name.ends_with(".tbz2") {
                    ("tar", vec!["xjf".into(), path.as_os_str().into(), "-C".into(), extract_dir.as_os_str().into()])
                } else if name.ends_with(".tar.xz") || name.ends_with(".txz") {
                    ("tar", vec!["xJf".into(), path.as_os_str().into(), "-C".into(), extract_dir.as_os_str().into()])
                } else if name.ends_with(".tar") {
                    ("tar", vec!["xf".into(), path.as_os_str().into(), "-C".into(), extract_dir.as_os_str().into()])
                } else if ext == "zip" {
                    ("unzip", vec!["-o".into(), path.as_os_str().into(), "-d".into(), extract_dir.as_os_str().into()])
                } else if ext == "7z" {
                    let out_flag: std::ffi::OsString = format!("-o{}", extract_dir.display()).into();
                    ("7z", vec!["x".into(), path.as_os_str().into(), out_flag])
                } else {
                    continue;
                };

                if root_mode {
                    let mut cmd = std::process::Command::new("pkexec");
                    cmd.arg(prog).args(&args);
                    let _ = cmd.status();
                } else {
                    let _ = std::process::Command::new(prog)
                        .args(&args)
                        .status();
                }
            }
        });
    }

    pub fn open_as_root(&mut self) {
        // Navigate into the selected folder with root mode enabled
        let selected: Vec<PathBuf> = self.entries.iter()
            .filter(|e| e.selected && e.is_dir)
            .map(|e| e.path.clone())
            .collect();
        if let Some(path) = selected.into_iter().next() {
            self.navigate_to(path);
            self.root_mode = true; // set after navigate_to (which resets it)
        } else {
            // No folder selected — just toggle root mode for current dir
            self.root_mode = !self.root_mode;
        }
    }

    pub fn open_in_terminal(&self) {
        let dir = self.current_dir.clone();
        std::thread::spawn(move || {
            let _ = std::process::Command::new("lntrn-terminal")
                .current_dir(&dir)
                .spawn();
        });
    }
}

pub(crate) fn _wl_copy(_text: String) {
    // Deprecated — native Wayland clipboard (clipboard.rs) is used instead.
}

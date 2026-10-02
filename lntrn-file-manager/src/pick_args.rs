//! The picker's command line: `lntrn-file-manager --pick ...` turns the
//! window into a file dialog for another program.
//!
//!   --pick             choose an existing file
//!   --pick-save        choose a name to save under (asks before a name
//!                      that is taken is returned)
//!   --pick-directory   choose a folder
//!   --pick-any         files and folders
//!   --pick-multiple    several may be chosen; without it the result is
//!                      never more than one path
//!   --pick-print0      write the result NUL-separated instead of one path
//!                      per line (pick_output.rs)
//!   --title T, --start-dir DIR, --save-name NAME,
//!   --filters "Images:*.png,*.jpg|Documents:*.pdf,*.txt"
//!
//! Exit status 0 with the chosen paths on stdout, 1 when cancelled.

use std::ffi::OsString;
use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct PickConfig {
    pub mode: PickType,
    pub multiple: bool,
    pub title: Option<String>,
    pub start_dir: Option<PathBuf>,
    pub filters: Vec<FileFilter>,
    pub active_filter: usize,
    pub save_name: Option<String>,
    /// `--pick-print0`: the result is written NUL-separated, as raw bytes,
    /// instead of one path per line.
    pub print0: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PickType {
    Open,
    Save,
    Directory,
    /// Files and/or folders mixed.
    Mixed,
}

#[derive(Clone, Debug)]
pub struct FileFilter {
    pub name: String,
    pub patterns: Vec<String>,
}

pub enum PickResult {
    Selected(Vec<PathBuf>),
    Cancelled,
}

impl PickConfig {
    pub(crate) fn default_title(&self) -> &str {
        match self.mode {
            PickType::Open => "Open File",
            PickType::Save => "Save File",
            PickType::Directory => "Select Folder",
            PickType::Mixed => "Select Files & Folders",
        }
    }
}

/// Parse `--filters "Images:*.png,*.jpg|Documents:*.pdf,*.txt"`
fn parse_filter_arg(s: &str) -> Vec<FileFilter> {
    s.split('|')
        .filter(|g| !g.is_empty())
        .filter_map(|group| {
            let (name, pats) = group.split_once(':')?;
            let patterns: Vec<String> = pats.split(',').map(|p| p.trim().to_string()).collect();
            Some(FileFilter {
                name: name.trim().to_string(),
                patterns,
            })
        })
        .collect()
}

/// The picker this process was started as, if it was.
pub(crate) fn from_command_line() -> Option<PickConfig> {
    let raw: Vec<OsString> = std::env::args_os().skip(1).collect();
    parse(&raw)
}

/// `raw`: the arguments after the program name. Flags are matched on a
/// lossy decoding; the start directory is taken from the raw value, so a
/// path that is not valid UTF-8 arrives intact.
fn parse(raw: &[OsString]) -> Option<PickConfig> {
    let args: Vec<String> = raw
        .iter()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    if args.is_empty() {
        return None;
    }

    let mut mode = None;
    let mut multiple = false;
    let mut title = None;
    let mut start_dir = None;
    let mut filters = Vec::new();
    let mut save_name = None;
    let mut print0 = false;
    let mut i = 0;

    while i < args.len() {
        match args[i].as_str() {
            "--pick" => mode = Some(PickType::Open),
            "--pick-save" => mode = Some(PickType::Save),
            "--pick-directory" => mode = Some(PickType::Directory),
            "--pick-any" => mode = Some(PickType::Mixed),
            "--pick-multiple" => multiple = true,
            "--pick-print0" => print0 = true,
            "--title" => {
                i += 1;
                title = args.get(i).cloned();
            }
            "--start-dir" => {
                i += 1;
                start_dir = raw.get(i).map(PathBuf::from);
            }
            "--filters" => {
                i += 1;
                if let Some(s) = args.get(i) {
                    filters = parse_filter_arg(s);
                }
            }
            "--save-name" => {
                i += 1;
                save_name = args.get(i).cloned();
            }
            _ => {}
        }
        i += 1;
    }

    mode.map(|m| PickConfig {
        mode: m,
        multiple,
        title,
        start_dir,
        filters,
        active_filter: 0,
        save_name,
        print0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<OsString> {
        list.iter().map(OsString::from).collect()
    }

    #[test]
    fn a_plain_picker_is_for_one_item_written_as_a_line() {
        let pick = parse(&args(&["--pick", "--title", "Open File"])).unwrap();
        assert_eq!(pick.mode, PickType::Open);
        assert!(!pick.multiple && !pick.print0);
        assert_eq!(pick.title.as_deref(), Some("Open File"));
        // No picker flag: an ordinary window.
        assert!(parse(&args(&["/home/a/Documents"])).is_none());
        assert!(parse(&[]).is_none());
    }

    #[test]
    fn the_flags_of_a_picker() {
        let pick = parse(&args(&[
            "--pick-save",
            "--pick-print0",
            "--pick-multiple",
            "--save-name",
            "Untitled.lnote",
            "--filters",
            "Images:*.png, *.jpg|Notes:*.lnote",
        ]))
        .unwrap();
        assert_eq!(pick.mode, PickType::Save);
        assert!(pick.multiple && pick.print0);
        assert_eq!(pick.save_name.as_deref(), Some("Untitled.lnote"));
        assert_eq!(pick.filters.len(), 2);
        assert_eq!(pick.filters[0].patterns, ["*.png", "*.jpg"]);
        assert_eq!(pick.filters[1].name, "Notes");
    }

    #[test]
    fn a_start_folder_that_is_not_utf8_arrives_intact() {
        use std::os::unix::ffi::OsStringExt;
        let dir = OsString::from_vec(b"/mnt/old/caf\xe9".to_vec());
        let pick = parse(&[OsString::from("--pick"), OsString::from("--start-dir"), dir.clone()])
            .unwrap();
        assert_eq!(pick.start_dir, Some(PathBuf::from(dir)));
    }
}

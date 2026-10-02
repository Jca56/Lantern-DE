use super::*;

const NVIM: &str = "[Desktop Entry]\n\
    Name[de]=Neovim (de)\n\
    Name=Neovim\n\
    GenericName=Text Editor\n\
    Exec=nvim %F\n\
    Terminal=true\n\
    Icon=nvim\n\
    MimeType=text/plain;text/x-rust;\n\
    \n\
    [Desktop Action new-window]\n\
    Name=New Window\n\
    Exec=nvim --other\n\
    Terminal=false\n";

#[test]
fn the_keys_of_the_desktop_entry_group_are_read() {
    let path = Path::new("/usr/share/applications/nvim.desktop");
    let app = parse_desktop_unfiltered(path, NVIM).expect("an application");
    assert_eq!(app.name, "Neovim");
    // The Exec line as written: it is parsed when launched, not here.
    assert_eq!(app.exec, "nvim %F");
    assert!(app.terminal, "Terminal=true is honoured");
    assert_eq!(app.icon.as_deref(), Some("nvim"));
    assert_eq!(app.work_dir, None);
    assert_eq!(app.desktop_id, "nvim");
    assert_eq!(app.file, path);
}

#[test]
fn spaces_around_the_equals_sign_do_not_matter() {
    let keys = EntryKeys::read(
        "[Desktop Entry]\nName = Thing\nExec = \"/opt/My App/app\" %U\n\
         NoDisplay = true\nHidden=false\nTerminal = false\nPath = /opt/My App\n",
    );
    assert_eq!(keys.name.as_deref(), Some("Thing"));
    assert_eq!(keys.exec.as_deref(), Some("\"/opt/My App/app\" %U"));
    assert!(keys.no_display, "was missed when written with spaces");
    assert!(!keys.hidden);
    assert!(!keys.terminal);
    assert_eq!(keys.work_dir.as_deref(), Some("/opt/My App"));
}

#[test]
fn an_entry_without_a_name_or_a_command_is_no_application() {
    let path = Path::new("/x/y.desktop");
    assert!(parse_desktop_unfiltered(path, "[Desktop Entry]\nName=Only a name\n").is_none());
    assert!(parse_desktop_unfiltered(path, "[Desktop Entry]\nExec=prog\n").is_none());
    // Keys outside the group do not count.
    assert!(parse_desktop_unfiltered(path, "[Other]\nName=N\nExec=prog\n").is_none());
}

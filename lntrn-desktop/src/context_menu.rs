use crate::state::{ContextMenuState, MenuAction, MenuEntry};

fn entry(label: &str, action: MenuAction) -> MenuEntry {
    MenuEntry {
        label: label.to_string(),
        action,
        separator: false,
        disabled: false,
    }
}

fn separator() -> MenuEntry {
    MenuEntry {
        label: String::new(),
        action: MenuAction::None,
        separator: true,
        disabled: false,
    }
}

/// Menu when right-clicking on a file or folder item.
pub fn item_menu(target_idx: usize, anchor_x: f32, anchor_y: f32) -> ContextMenuState {
    ContextMenuState {
        anchor_x,
        anchor_y,
        target_idx: Some(target_idx),
        hover: None,
        items: vec![
            entry("Open", MenuAction::Open),
            separator(),
            entry("Copy Name", MenuAction::CopyName),
            entry("Copy Path", MenuAction::CopyPath),
            separator(),
            entry("Rename", MenuAction::Rename),
            entry("Move to Trash", MenuAction::Trash),
        ],
    }
}

/// Menu when right-clicking on an empty area of the desktop.
///
/// Superseded by the radial menu (see `radial_menu`), kept as a list-style
/// fallback for empty-space right-clicks.
#[allow(dead_code)]
pub fn empty_menu(anchor_x: f32, anchor_y: f32) -> ContextMenuState {
    ContextMenuState {
        anchor_x,
        anchor_y,
        target_idx: None,
        hover: None,
        items: vec![
            entry("New Folder", MenuAction::NewFolder),
            separator(),
            entry("Select All", MenuAction::SelectAll),
            entry("Refresh", MenuAction::Refresh),
            separator(),
            entry("Open Terminal Here", MenuAction::OpenTerminal),
        ],
    }
}

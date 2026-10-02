//! Clicks in the Properties dialog, taken on the press.
//!
//! The folder icon, the icon picker (tabs, cells, Reset, Back, Choose
//! Custom Image) and the checksum row used to act while drawing, whenever
//! their zone was "down this frame". A press and release that arrive
//! together (a touchpad tap, any click delivered while the loop was busy)
//! are both handled before the next draw, so the draw never saw the button
//! down and the click was lost; and a button held for several frames acted
//! on every one of them. The main loop has the press as an event and the
//! zone it landed on: each of these now happens once, there.

use std::sync::atomic::Ordering;

use crate::app::App;
use crate::properties::{FileProperties, IconPickerTab, PropertiesEvent, ZONE_PROPS_CHECKSUM_ROW};
use crate::{
    ZONE_PROPS_ICON, ZONE_PROPS_ICON_BASE, ZONE_PROPS_PICKER_BACK, ZONE_PROPS_PICKER_CUSTOM,
    ZONE_PROPS_PICKER_RESET, ZONE_PROPS_PICKER_TAB_BASE,
};

impl FileProperties {
    /// The left button went down on `zone`. What only concerns the dialog
    /// is done here; what the app has to do comes back.
    pub(crate) fn on_press(&mut self, zone: u32) -> Option<PropertiesEvent> {
        if zone == ZONE_PROPS_ICON {
            // The header icon opens and closes the picker (folders only).
            if self.is_dir {
                self.picker_open = !self.picker_open;
                self.picker_scroll = 0.0;
            }
            return None;
        }
        if zone == ZONE_PROPS_CHECKSUM_ROW {
            // The row only has a zone once the hash is there.
            let hash = self.checksum_job.as_ref().and_then(|job| job.get())?;
            return Some(PropertiesEvent::CopyText(hash));
        }
        if !(self.picker_open && self.is_dir) {
            return None;
        }

        // ── Icon picker ─────────────────────────────────────────────────
        let tabs = IconPickerTab::all();
        if let Some(tab) = zone
            .checked_sub(ZONE_PROPS_PICKER_TAB_BASE)
            .and_then(|i| tabs.get(i as usize))
        {
            self.picker_tab = *tab;
            // Another tab, another grid: from its top.
            self.picker_scroll = 0.0;
            return None;
        }
        match zone {
            ZONE_PROPS_PICKER_BACK => {
                self.picker_open = false;
                None
            }
            ZONE_PROPS_PICKER_RESET => {
                self.picker_open = false;
                Some(PropertiesEvent::IconReset)
            }
            ZONE_PROPS_PICKER_CUSTOM => {
                if self.picker_tab == IconPickerTab::Custom {
                    self.choose_custom_icon();
                    self.picker_open = false;
                }
                None
            }
            _ => {
                // A cell of the grid: the icons of the tab as it was last
                // drawn, which is what the pointer was on.
                let index = zone.checked_sub(ZONE_PROPS_ICON_BASE)? as usize;
                let icon = match &self.picker_icons {
                    Some((tab, icons)) if *tab == self.picker_tab => icons.get(index)?.clone(),
                    _ => return None,
                };
                self.picker_open = false;
                Some(PropertiesEvent::IconChosen(icon))
            }
        }
    }

    /// Let the user pick any image as this folder's icon: a file picker of
    /// our own, waited for on a thread, which sets the icon when it comes
    /// back with a choice.
    fn choose_custom_icon(&self) {
        let folder = self.path.clone();
        let refresh = self.refresh.clone();
        std::thread::spawn(move || {
            if crate::pick_output::choose_folder_icon(&folder) {
                // Listings carry the attribute; have them read again so
                // the new icon shows.
                refresh.store(true, Ordering::SeqCst);
                crate::bg::wake();
            }
        });
    }
}

impl App {
    /// A left press on a zone of the Properties dialog that is neither its
    /// close button, a section header nor part of the audio tag editor.
    pub(crate) fn properties_pressed(&mut self, zone: u32) {
        let Some(props) = self.properties.as_mut() else {
            return;
        };
        let folder = props.path.clone();
        match props.on_press(zone) {
            // The attribute is written, and the icon cache told, between
            // frames (see `pending_icon_apply` in the main loop).
            Some(PropertiesEvent::IconChosen(icon)) => self
                .pending_icon_apply
                .push((folder, Some(icon.to_string_lossy().to_string()))),
            Some(PropertiesEvent::IconReset) => self.pending_icon_apply.push((folder, None)),
            Some(PropertiesEvent::CopyText(text)) => {
                if let Some(clip) = &self.wayland_clipboard {
                    clip.set_text(&text);
                }
            }
            None => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;

    fn props(is_dir: bool) -> FileProperties {
        let hint = crate::props_load::Hint {
            is_dir,
            ..Default::default()
        };
        // A path that does not exist: nothing is read for these tests.
        let path = std::env::temp_dir().join(format!("fox-props-click-{}", std::process::id()));
        FileProperties::open(&path, hint, Arc::new(AtomicBool::new(false)))
    }

    fn with_icons(mut p: FileProperties, tab: IconPickerTab, names: &[&str]) -> FileProperties {
        p.picker_tab = tab;
        p.picker_icons = Some((tab, names.iter().map(PathBuf::from).collect()));
        p
    }

    #[test]
    fn one_press_on_the_folder_icon_toggles_the_picker_once() {
        let mut p = props(true);
        assert_eq!(p.on_press(ZONE_PROPS_ICON), None);
        assert!(p.picker_open);
        assert_eq!(p.on_press(ZONE_PROPS_ICON), None);
        assert!(!p.picker_open);
        // A file's icon is not a button.
        let mut file = props(false);
        file.on_press(ZONE_PROPS_ICON);
        assert!(!file.picker_open);
    }

    #[test]
    fn picker_tabs_cells_reset_and_back() {
        let mut p = with_icons(
            props(true),
            IconPickerTab::Standard,
            &["/i/a.svg", "/i/b.svg"],
        );
        p.picker_open = true;

        assert_eq!(p.on_press(ZONE_PROPS_PICKER_TAB_BASE + 1), None);
        assert_eq!(p.picker_tab, IconPickerTab::Colors);
        assert!(p.picker_open);
        // The list on screen is still the previous tab's until the next
        // draw: a cell pressed now is not an icon of this tab.
        assert_eq!(p.on_press(ZONE_PROPS_ICON_BASE), None);
        assert!(p.picker_open);

        let mut p = with_icons(p, IconPickerTab::Colors, &["/c/red.svg", "/c/blue.svg"]);
        assert_eq!(
            p.on_press(ZONE_PROPS_ICON_BASE + 1),
            Some(PropertiesEvent::IconChosen(PathBuf::from("/c/blue.svg")))
        );
        assert!(!p.picker_open, "choosing closes the picker");

        p.picker_open = true;
        assert_eq!(p.on_press(ZONE_PROPS_ICON_BASE + 2), None, "no such cell");
        assert_eq!(p.on_press(ZONE_PROPS_PICKER_BACK), None);
        assert!(!p.picker_open);

        p.picker_open = true;
        assert_eq!(
            p.on_press(ZONE_PROPS_PICKER_RESET),
            Some(PropertiesEvent::IconReset)
        );
        assert!(!p.picker_open);
    }

    #[test]
    fn picker_zones_do_nothing_while_the_picker_is_closed() {
        let mut p = with_icons(props(true), IconPickerTab::Standard, &["/i/a.svg"]);
        assert_eq!(p.on_press(ZONE_PROPS_ICON_BASE), None);
        assert_eq!(p.on_press(ZONE_PROPS_PICKER_RESET), None);
        assert_eq!(p.on_press(ZONE_PROPS_PICKER_TAB_BASE + 2), None);
        assert_eq!(p.picker_tab, IconPickerTab::Standard);
        // And the checksum row copies nothing before there is a checksum.
        assert_eq!(p.on_press(ZONE_PROPS_CHECKSUM_ROW), None);
    }

    #[test]
    fn the_picker_buttons_have_ids_of_their_own() {
        // Back used to be 85, the split-view toggle, and Choose Custom Image
        // shared Back's id.
        let ids = [
            ZONE_PROPS_ICON,
            ZONE_PROPS_PICKER_RESET,
            ZONE_PROPS_PICKER_BACK,
            ZONE_PROPS_PICKER_CUSTOM,
            ZONE_PROPS_CHECKSUM_ROW,
            crate::ZONE_SPLIT_TOGGLE,
        ];
        for (i, a) in ids.iter().enumerate() {
            assert!(!ids[i + 1..].contains(a), "zone id {a} is used twice");
        }
        let tabs = IconPickerTab::all().len() as u32;
        assert!(ZONE_PROPS_PICKER_TAB_BASE + tabs <= ZONE_PROPS_PICKER_RESET);
    }
}

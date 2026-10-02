//! Cloud sync's dialogs as entries of the op-dialog queue: what they hold,
//! and what their buttons do. (When the question opens by itself is
//! decided in cloud_ui.rs; the wording is in view.rs.)

use std::cell::Cell;
use std::time::Instant;

use lntrn_render::Rect;

use crate::app::App;
use crate::cloud::guard::HeldDeletions;
use crate::cloud::sync::{SyncHandle, SyncProblem, SyncStatus};
use crate::op_dialogs::OpDialog;

use super::view::{self, DialogView};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    /// The question about the held set `id`.
    Held {
        id: String,
    },
    Details,
}

/// Where a dialog's list was drawn last, when it has more rows than fit.
#[derive(Clone, Copy, Debug)]
pub(super) struct ListArea {
    /// The box the rows are in.
    pub viewport: Rect,
    /// Height of all rows, in the units the scroll offset counts in.
    pub content_h: f32,
    /// Height of one row.
    pub row_h: f32,
}

/// A cloud dialog in the op-dialog queue.
#[derive(Clone, Debug)]
pub struct CloudDialog {
    pub(super) kind: Kind,
    pub view: DialogView,
    /// It takes no clicks and no keys before this.
    pub(super) armed_at: Instant,
    /// How far the list is scrolled, px. Clamped where it is drawn.
    pub(super) scroll: Cell<f32>,
    /// Set by the draw: `None` while every row fits.
    pub(super) list_area: Cell<Option<ListArea>>,
    /// The list's scrollbar thumb is held: the grabbed point's distance
    /// from the thumb's top.
    pub(super) grab: Option<f32>,
}

impl CloudDialog {
    pub(super) fn new(kind: Kind, view: DialogView, armed_at: Instant) -> Self {
        Self {
            kind,
            view,
            armed_at,
            scroll: Cell::new(0.0),
            list_area: Cell::new(None),
            grab: None,
        }
    }

    /// The question about `held`. `full`: its complete list of paths, if
    /// the window could read it (see `view::held_view`).
    pub(super) fn held(
        held: &HeldDeletions,
        full: Option<Vec<String>>,
        armed_at: Instant,
    ) -> Self {
        Self::new(
            Kind::Held {
                id: held.id.clone(),
            },
            view::held_view(held, full),
            armed_at,
        )
    }

    /// Show another held set in this dialog. It starts over: at the top of
    /// the new list, and deaf for a moment, because a click on its way to
    /// the old list must not answer for the new one.
    pub(super) fn show_held(&mut self, held: &HeldDeletions, full: Option<Vec<String>>, note: &str) {
        self.kind = Kind::Held {
            id: held.id.clone(),
        };
        self.view = view::held_view(held, full);
        // (Under the note about an incomplete list, if there is one.)
        self.view.note = Some(match self.view.note.take() {
            Some(other) => format!("{note} {other}"),
            None => note.to_string(),
        });
        self.armed_at = Instant::now() + super::INPUT_GUARD;
        self.scroll.set(0.0);
        self.grab = None;
    }

    /// The list's scrollbar as it is on screen.
    fn list_bar(&self, s: f32) -> Option<(lntrn_ui::gpu::Scrollbar, ListArea)> {
        let area = self.list_area.get()?;
        let bar = crate::scrollbar::bar(&area.viewport, area.content_h, self.scroll.get(), s);
        Some((bar, area))
    }
}

/// True for the buttons only cloud dialogs have. (Their "Decide later" and
/// "Close" are the queue's own safe button.)
pub fn is_dialog_zone(zone: u32) -> bool {
    matches!(
        zone,
        crate::ZONE_CLOUD_DLG_RESTORE
            | crate::ZONE_CLOUD_DLG_DELETE
            | crate::ZONE_CLOUD_DLG_RETRY
            | crate::ZONE_CLOUD_DLG_SIGN_OUT
            | crate::ZONE_CLOUD_DLG_CREATE
    )
}

impl App {
    /// `folder_missing`: the caller has just found ~/Cloud gone. The engine
    /// says the same once it has looked (it does every half minute); until
    /// then its report would have this dialog claim that all is well.
    pub(super) fn open_cloud_details(&mut self, folder_missing: bool) {
        // Fresh, not the copy of the last poll.
        let Some(mut report) = self.cloud_sync.as_ref().map(SyncHandle::report) else {
            return;
        };
        if folder_missing {
            report.status = SyncStatus::Error;
            report.problem = Some(SyncProblem::FolderMissing);
        }
        self.op_dialogs.push_back(OpDialog::Cloud(CloudDialog::new(
            Kind::Details,
            view::details_view(&report, &self.cloud_ui.last_email),
            Instant::now(),
        )));
    }

    /// The question about `held`, with the complete list when it can be had.
    pub(super) fn held_dialog(&self, held: &HeldDeletions, armed_at: Instant) -> CloudDialog {
        let full = self
            .cloud_sync
            .as_ref()
            .and_then(|handle| handle.held_paths(&held.id));
        CloudDialog::held(held, full, armed_at)
    }

    /// A dialog was answered and another came to the front. If that is the
    /// held-deletions question, it is deaf for a moment: the press that
    /// answered the one before must not answer it too.
    pub(crate) fn cloud_dialog_fronted(&mut self) {
        if let Some(OpDialog::Cloud(dialog)) = self.op_dialogs.front_mut() {
            if matches!(dialog.kind, Kind::Held { .. }) {
                dialog.armed_at = dialog.armed_at.max(Instant::now() + super::INPUT_GUARD);
            }
        }
    }

    // ── The list of the front dialog ────────────────────────────────────

    fn front_cloud_dialog_mut(&mut self) -> Option<&mut CloudDialog> {
        match self.op_dialogs.front_mut() {
            Some(OpDialog::Cloud(dialog)) => Some(dialog),
            _ => None,
        }
    }

    /// The wheel turned by `delta` px while a dialog is open. (The draw
    /// clamps the offset to what there is to scroll.)
    pub(crate) fn cloud_list_wheel(&mut self, delta: f32) {
        if let Some(dialog) = self.front_cloud_dialog_mut() {
            if dialog.list_area.get().is_some() {
                dialog.scroll.set((dialog.scroll.get() + delta).max(0.0));
            }
        }
    }

    /// Up / Down / Page Up / Page Down / Home / End on a dialog with a list
    /// that scrolls. True when the key was one of those.
    pub(crate) fn cloud_list_key(&mut self, key: u32) -> bool {
        use crate::keyboard::code;
        let Some(dialog) = self.front_cloud_dialog_mut() else {
            return false;
        };
        let Some(area) = dialog.list_area.get() else {
            return false;
        };
        let page = (area.viewport.h - area.row_h).max(area.row_h);
        let at = dialog.scroll.get();
        let to = match key {
            code::UP => at - area.row_h,
            code::DOWN => at + area.row_h,
            code::PAGE_UP => at - page,
            code::PAGE_DOWN => at + page,
            code::HOME => 0.0,
            code::END => area.content_h,
            _ => return false,
        };
        dialog.scroll.set(to.max(0.0));
        true
    }

    /// The left button went down on the list's scrollbar, at `cy`.
    pub(crate) fn cloud_list_press(&mut self, cy: f32, s: f32) {
        let Some(dialog) = self.front_cloud_dialog_mut() else {
            return;
        };
        let Some((bar, area)) = dialog.list_bar(s) else {
            return;
        };
        if cy >= bar.thumb.y && cy <= bar.thumb.y + bar.thumb.h {
            dialog.grab = Some(cy - bar.thumb.y);
        } else {
            // On the track: the thumb's middle goes there, and is held.
            dialog.grab = Some(bar.thumb.h * 0.5);
            dialog
                .scroll
                .set(bar.offset_for_thumb_y(cy, area.content_h, area.viewport.h));
        }
    }

    /// The pointer is at `cy` with the button still down.
    pub(crate) fn cloud_list_drag(&mut self, cy: f32, s: f32) {
        let Some(dialog) = self.front_cloud_dialog_mut() else {
            return;
        };
        let (Some(grab), Some((bar, area))) = (dialog.grab, dialog.list_bar(s)) else {
            return;
        };
        dialog.scroll.set(bar.offset_for_thumb_y(
            cy - grab + bar.thumb.h * 0.5,
            area.content_h,
            area.viewport.h,
        ));
    }

    /// The left button went up.
    pub(crate) fn cloud_list_release(&mut self) {
        if let Some(dialog) = self.front_cloud_dialog_mut() {
            dialog.grab = None;
        }
    }

    /// The list's scrollbar is being dragged: frames are due.
    pub(crate) fn cloud_list_dragging(&self) -> bool {
        self.front_cloud_dialog().is_some_and(|d| d.grab.is_some())
    }

    pub(super) fn close_cloud_dialogs(&mut self) {
        self.op_dialogs
            .retain(|dialog| !matches!(dialog, OpDialog::Cloud(_)));
    }

    fn front_cloud_dialog(&self) -> Option<&CloudDialog> {
        match self.op_dialogs.front() {
            Some(OpDialog::Cloud(dialog)) => Some(dialog),
            _ => None,
        }
    }

    /// The front cloud dialog, once it takes input. A question that opened
    /// by itself is deaf for a moment (`armed_at`): the click or key was on
    /// its way to whatever was on screen before, and must neither answer
    /// the question nor wave it away unread.
    fn armed_cloud_dialog(&self) -> Option<&CloudDialog> {
        self.front_cloud_dialog()
            .filter(|dialog| Instant::now() >= dialog.armed_at)
    }

    /// "Decide later", "Close", Esc, a click outside: the dialog goes and
    /// nothing changes. (A hold stays held; the pill reopens the question.)
    pub(crate) fn cloud_dialog_dismiss(&mut self) {
        if self.armed_cloud_dialog().is_some() {
            self.op_dialogs.pop_front();
        }
    }

    /// Enter on a cloud dialog: its default button. For the question that
    /// is "Restore them", which deletes nothing.
    pub(crate) fn cloud_dialog_enter(&mut self) {
        let Some(dialog) = self.armed_cloud_dialog() else {
            return;
        };
        match dialog.view.default_zone() {
            Some(crate::ZONE_OP_DIALOG_SAFE) | None => self.op_dialog_choose(false),
            Some(zone) => self.cloud_dialog_button(zone),
        }
    }

    /// A button of the front cloud dialog was pressed. ("Decide later" and
    /// "Close" are the queue's safe button and never get here: they close
    /// the dialog and change nothing.)
    pub(crate) fn cloud_dialog_button(&mut self, zone: u32) {
        let Some(dialog) = self.armed_cloud_dialog() else {
            return;
        };
        if !dialog.view.has_button(zone) {
            return;
        }
        let kind = dialog.kind.clone();
        match (zone, kind) {
            (crate::ZONE_CLOUD_DLG_RESTORE, Kind::Held { id }) => self.answer_held(&id, false),
            (crate::ZONE_CLOUD_DLG_DELETE, Kind::Held { id }) => self.answer_held(&id, true),
            (crate::ZONE_CLOUD_DLG_RETRY, Kind::Details) => {
                self.op_dialogs.pop_front();
                let asked = self
                    .cloud_sync
                    .as_ref()
                    .is_some_and(SyncHandle::retry_failed);
                self.set_status_note(if asked {
                    "Cloud sync is trying the failed files again"
                } else {
                    "Could not ask cloud sync to try again (see fox-cloud.log)"
                });
            }
            (crate::ZONE_CLOUD_DLG_SIGN_OUT, Kind::Details) => self.cloud_sign_out(),
            (crate::ZONE_CLOUD_DLG_CREATE, Kind::Details) => {
                self.op_dialogs.pop_front();
                match crate::cloud::ensure_cloud_dir() {
                    Ok(root) => {
                        // The engine looks at a missing folder only every
                        // half minute; this makes it look now. It will find
                        // the folder empty and ask before deleting anything.
                        if let Some(handle) = &self.cloud_sync {
                            handle.retry_failed();
                        }
                        self.navigate_to(root);
                    }
                    Err(e) => self
                        .show_notice("The Cloud folder could not be created", vec![e.to_string()]),
                }
            }
            _ => {}
        }
        self.cloud_dialog_fronted();
    }

    /// Restore (`confirm` false) or Delete everywhere (`confirm` true) for
    /// the held set `id`, the one the dialog shows.
    fn answer_held(&mut self, id: &str, confirm: bool) {
        let Some(handle) = self.cloud_sync.as_ref() else {
            self.op_dialogs.pop_front();
            return;
        };
        if handle.answer_held(id, confirm) {
            let count = self
                .front_cloud_dialog()
                .map_or(0, |dialog| dialog.view.list_total);
            self.op_dialogs.pop_front();
            self.cloud_ui.ask.answered(id, Instant::now());
            self.set_status_note(match (confirm, count) {
                (true, 1) => "Cloud sync: deleting 1 file everywhere".to_string(),
                (true, n) => format!("Cloud sync: deleting {n} files everywhere"),
                (false, 1) => "Cloud sync: restoring 1 file".to_string(),
                (false, n) => format!("Cloud sync: restoring {n} files"),
            });
            return;
        }
        // Not recorded. Either a pass has computed another set since (the
        // answer must be for what is on screen) or the answer could not be
        // written. Show what is held now and say so.
        match handle.held_deletions() {
            None => {
                self.op_dialogs.pop_front();
                self.set_status_note("Cloud sync is no longer waiting for an answer");
            }
            Some(held) => {
                let note = if held.id == id {
                    "The answer could not be saved (see fox-cloud.log). Nothing was changed."
                } else {
                    "The list changed just now. Nothing was done: check it and choose again."
                };
                let full = handle.held_paths(&held.id);
                if let Some(dialog) = self.front_cloud_dialog_mut() {
                    dialog.show_held(&held, full, note);
                }
            }
        }
    }
}

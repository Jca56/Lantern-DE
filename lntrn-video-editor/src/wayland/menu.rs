//! Menu-bar action dispatch + the small clip/project helpers it
//! delegates to. Both the menu-bar and the keyboard shortcut path
//! reuse these helpers.

use lntrn_ui::gpu::MenuItem;

use crate::actions;
use crate::playback::Playback;
use crate::project::Project;

use super::state::{PendingPick, PickKind};

pub(super) fn build_menus() -> Vec<(&'static str, Vec<MenuItem>)> {
    vec![
        (
            "File",
            vec![
                MenuItem::action(actions::ACT_NEW_PROJECT, "New Project"),
                MenuItem::action_with(actions::ACT_OPEN, "Open Project…", "Ctrl+O"),
                MenuItem::action_with(actions::ACT_SAVE, "Save Project", "Ctrl+S"),
                MenuItem::separator(),
                MenuItem::action(actions::ACT_IMPORT_MEDIA, "Import Media…"),
                MenuItem::separator(),
                MenuItem::action_with(actions::ACT_EXPORT_MP4_MED, "Export MP4 (medium)", "Ctrl+E"),
                MenuItem::action(actions::ACT_EXPORT_MP4_HIGH, "Export MP4 (high)"),
                MenuItem::action(actions::ACT_EXPORT_MP4_LOW, "Export MP4 (low)"),
                MenuItem::action(actions::ACT_EXPORT_GIF_SMALL, "Export GIF (480px)"),
                MenuItem::action(actions::ACT_EXPORT_GIF_LARGE, "Export GIF (720px)"),
                MenuItem::separator(),
                MenuItem::action_with(actions::ACT_QUIT, "Quit", "Ctrl+Q"),
            ],
        ),
        (
            "Edit",
            vec![
                MenuItem::action_with(10, "Undo", "Ctrl+Z"),
                MenuItem::action_with(11, "Redo", "Ctrl+Shift+Z"),
                MenuItem::separator(),
                MenuItem::action_with(12, "Cut", "Ctrl+X"),
                MenuItem::action_with(13, "Copy", "Ctrl+C"),
                MenuItem::action_with(14, "Paste", "Ctrl+V"),
                MenuItem::action_with(actions::ACT_DELETE, "Delete", "Del"),
                MenuItem::separator(),
                MenuItem::action(16, "Select All"),
            ],
        ),
        (
            "View",
            vec![
                MenuItem::toggle(20, "Media Browser", true),
                MenuItem::toggle(21, "Properties", true),
                MenuItem::separator(),
                MenuItem::action(22, "Zoom In"),
                MenuItem::action(23, "Zoom Out"),
                MenuItem::action(24, "Fit Timeline"),
            ],
        ),
        (
            "Clip",
            vec![
                MenuItem::action(actions::ACT_INSERT_SELECTED, "Insert Selected at Playhead"),
                MenuItem::separator(),
                MenuItem::action_with(actions::ACT_SPLIT, "Split at Playhead", "S"),
                MenuItem::action(actions::ACT_TRIM_START, "Trim Start"),
                MenuItem::action(actions::ACT_TRIM_END, "Trim End"),
                MenuItem::separator(),
                MenuItem::action_with(actions::ACT_SPEED_DOWN, "Slow Down", "["),
                MenuItem::action_with(actions::ACT_SPEED_UP, "Speed Up", "]"),
                MenuItem::action_with(actions::ACT_UNLINK_AUDIO, "Unlink Audio", "\\"),
                MenuItem::action_with(actions::ACT_TOGGLE_MUTE_TRACK, "Mute Track", "M"),
            ],
        ),
        (
            "Track",
            vec![
                MenuItem::action_with(actions::ACT_ADD_VIDEO_TRACK, "Add Video Track", "T"),
                MenuItem::action(actions::ACT_ADD_AUDIO_TRACK, "Add Audio Track"),
            ],
        ),
    ]
}

pub(super) fn dispatch_menu_action(
    id: u32,
    running: &mut bool,
    pending_pick: &mut Option<PendingPick>,
    project: &mut Project,
    playback: &mut Playback,
) {
    match id {
        actions::ACT_NEW_PROJECT => {
            *project = Project::new();
            playback.pause();
        }
        actions::ACT_OPEN => trigger_open_project(pending_pick),
        actions::ACT_SAVE => trigger_save(project, pending_pick),
        actions::ACT_IMPORT_MEDIA => {
            *pending_pick = Some(PendingPick {
                kind: PickKind::ImportMedia,
                rx: actions::spawn_video_picker("Import Media"),
            });
        }
        actions::ACT_INSERT_SELECTED => {
            if project
                .insert_selected_at_playhead(playback.timeline_position)
                .is_none()
            {
                eprintln!("[video-editor] no selected media to insert");
            }
        }
        actions::ACT_SPLIT => split_at_playhead(project, playback),
        actions::ACT_DELETE => delete_selected_clip(project),
        actions::ACT_SPEED_UP => nudge_selected_speed(project, 1.25),
        actions::ACT_SPEED_DOWN => nudge_selected_speed(project, 1.0 / 1.25),
        actions::ACT_UNLINK_AUDIO => unlink_selected(project),
        actions::ACT_TOGGLE_MUTE_TRACK => mute_selected_clip_track(project),
        actions::ACT_ADD_VIDEO_TRACK => {
            add_track(project, crate::project::TrackKind::Video);
        }
        actions::ACT_ADD_AUDIO_TRACK => {
            add_track(project, crate::project::TrackKind::Audio);
        }
        actions::ACT_EXPORT_MP4_LOW => kick_export(
            crate::export::ExportFormat::Mp4,
            crate::export::ExportQuality::Low,
            project,
        ),
        actions::ACT_EXPORT_MP4_MED => kick_export(
            crate::export::ExportFormat::Mp4,
            crate::export::ExportQuality::Medium,
            project,
        ),
        actions::ACT_EXPORT_MP4_HIGH => kick_export(
            crate::export::ExportFormat::Mp4,
            crate::export::ExportQuality::High,
            project,
        ),
        actions::ACT_EXPORT_GIF_SMALL => {
            let mut req = crate::export::ExportRequest::defaults_for(
                crate::export::ExportFormat::Gif,
            );
            req.width = 480;
            if let Err(e) = crate::export::start(req, project) {
                eprintln!("[video-editor] export: {e}");
            }
        }
        actions::ACT_EXPORT_GIF_LARGE => {
            let mut req = crate::export::ExportRequest::defaults_for(
                crate::export::ExportFormat::Gif,
            );
            req.width = 720;
            if let Err(e) = crate::export::start(req, project) {
                eprintln!("[video-editor] export: {e}");
            }
        }
        actions::ACT_QUIT => {
            *running = false;
        }
        other => {
            eprintln!("[video-editor] menu action {other} not yet wired");
        }
    }
}

pub(super) fn trigger_open_project(pending_pick: &mut Option<PendingPick>) {
    *pending_pick = Some(PendingPick {
        kind: PickKind::OpenProject,
        rx: actions::spawn_open_project_picker("Open Project"),
    });
}

/// Direct-save if the project already has a path, otherwise spawn a save picker
/// (which lands in `~/Videos/Lantern Projects/` by default).
pub(super) fn trigger_save(project: &mut Project, pending_pick: &mut Option<PendingPick>) {
    if let Some(path) = project.save_path.clone() {
        if let Err(e) = crate::projectio::save(project, &path) {
            eprintln!("[video-editor] save project: {e}");
        }
        return;
    }
    *pending_pick = Some(PendingPick {
        kind: PickKind::SaveProject,
        rx: actions::spawn_save_project_picker("Save Project", "Untitled.lproj"),
    });
}

fn kick_export(
    format: crate::export::ExportFormat,
    quality: crate::export::ExportQuality,
    project: &Project,
) {
    let mut req = crate::export::ExportRequest::defaults_for(format);
    req.quality = quality;
    if let Err(e) = crate::export::start(req, project) {
        eprintln!("[video-editor] export: {e}");
    }
}

pub(super) fn nudge_selected_speed(project: &mut Project, factor: f32) {
    let Some(id) = project.selected_clip else {
        return;
    };
    let linked = project.clip_by_id(id).and_then(|c| c.linked_id);
    project.nudge_speed(id, factor);
    if let Some(lid) = linked {
        project.nudge_speed(lid, factor);
    }
}

pub(super) fn unlink_selected(project: &mut Project) {
    if let Some(id) = project.selected_clip {
        if !project.unlink(id) {
            eprintln!("[video-editor] selected clip has no linked partner");
        }
    }
}

pub(super) fn mute_selected_clip_track(project: &mut Project) {
    if let Some(clip) = project.selected_clip_ref() {
        let tid = clip.track_id;
        project.toggle_mute(tid);
    }
}

pub(super) fn add_track(project: &mut Project, kind: crate::project::TrackKind) {
    project.add_track(kind);
}

pub(super) fn apply_inspector_drag(
    project: &mut Project,
    hit: &crate::inspector::FieldHit,
    cx: f32,
    s: f32,
) {
    let value = crate::inspector::slider_value_for_cursor(hit, cx, s);
    let Some(clip_id) = project.selected_clip else {
        return;
    };
    use crate::inspector::InspectorField;
    match hit.field {
        InspectorField::Speed => project.set_speed(clip_id, value),
        InspectorField::Scale => project.set_scale(clip_id, value),
        InspectorField::OffsetX => {
            if let Some(clip) = project.timeline_clips.iter_mut().find(|c| c.id == clip_id) {
                clip.transform.offset_x = value.clamp(-0.5, 0.5);
            }
        }
        InspectorField::OffsetY => {
            if let Some(clip) = project.timeline_clips.iter_mut().find(|c| c.id == clip_id) {
                clip.transform.offset_y = value.clamp(-0.5, 0.5);
            }
        }
        InspectorField::Opacity => {
            if let Some(clip) = project.timeline_clips.iter_mut().find(|c| c.id == clip_id) {
                clip.transform.opacity = value.clamp(0.0, 1.0);
            }
        }
        InspectorField::Volume => project.set_volume(clip_id, value),
    }
}

pub(super) fn split_at_playhead(project: &mut Project, playback: &Playback) {
    if project.split_at(playback.timeline_position).is_none() {
        eprintln!("[video-editor] playhead not inside a clip — nothing to split");
    }
}

pub(super) fn delete_selected_clip(project: &mut Project) {
    if project.delete_selected_clip().is_none() {
        eprintln!("[video-editor] no selected timeline clip to delete");
    }
}

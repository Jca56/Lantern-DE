use std::ffi::c_void;
use std::ptr::NonNull;

use anyhow::{anyhow, Result};
use lntrn_render::{Color, GpuContext, Painter, TextRenderer};
use lntrn_ui::gpu::{
    FoxPalette, InteractionContext, MenuBar, MenuItem, PopupSurface, WaylandPopupBackend,
};
use raw_window_handle::{
    DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, RawDisplayHandle,
    RawWindowHandle, WaylandDisplayHandle, WaylandWindowHandle, WindowHandle,
};
use wayland_client::{
    protocol::{wl_compositor, wl_pointer, wl_seat, wl_surface},
    Connection, EventQueue, Proxy,
};
use wayland_protocols::wp::cursor_shape::v1::client::{
    wp_cursor_shape_device_v1, wp_cursor_shape_manager_v1,
};
use wayland_protocols::wp::viewporter::client::wp_viewporter;
use wayland_protocols::xdg::decoration::zv1::client::{
    zxdg_decoration_manager_v1, zxdg_toplevel_decoration_v1,
};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};

use crate::actions;
use crate::chrome::TITLE_BAR_H;
use crate::playback::Playback;
use crate::preview::PreviewMonitor;
use crate::project::Project;

pub const BTN_LEFT: u32 = 0x110;
pub const BTN_RIGHT: u32 = 0x111;
const KEY_ESC: u32 = 1;
const KEY_E: u32 = 18;
const KEY_T: u32 = 20;
const KEY_LEFTBRACE: u32 = 26;
const KEY_RIGHTBRACE: u32 = 27;
const KEY_ENTER: u32 = 28;
const KEY_S: u32 = 31;
const KEY_BACKSLASH: u32 = 43;
const KEY_M: u32 = 50;
const KEY_SPACE: u32 = 57;
const KEY_DELETE: u32 = 111;

#[derive(Clone, Copy)]
enum PickKind {
    Open,
    Import,
}

struct PendingPick {
    kind: PickKind,
    rx: crossbeam_channel::Receiver<std::path::PathBuf>,
}

// ── WaylandHandle for wgpu ─────────────────────────────────────────────────

struct WaylandHandle {
    display: NonNull<c_void>,
    surface: NonNull<c_void>,
}
impl HasDisplayHandle for WaylandHandle {
    fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
        let raw = RawDisplayHandle::Wayland(WaylandDisplayHandle::new(self.display));
        Ok(unsafe { DisplayHandle::borrow_raw(raw) })
    }
}
impl HasWindowHandle for WaylandHandle {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        let raw = RawWindowHandle::Wayland(WaylandWindowHandle::new(self.surface));
        Ok(unsafe { WindowHandle::borrow_raw(raw) })
    }
}

// ── Wayland state ──────────────────────────────────────────────────────────

pub(crate) struct State {
    pub(crate) running: bool,
    pub(crate) configured: bool,
    pub(crate) frame_done: bool,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) scale: i32,
    pub(crate) output_phys_width: u32,
    pub(crate) maximized: bool,
    pub(crate) compositor: Option<wl_compositor::WlCompositor>,
    pub(crate) wm_base: Option<xdg_wm_base::XdgWmBase>,
    pub(crate) viewporter: Option<wp_viewporter::WpViewporter>,
    pub(crate) surface: Option<wl_surface::WlSurface>,
    pub(crate) xdg_surface: Option<xdg_surface::XdgSurface>,
    pub(crate) toplevel: Option<xdg_toplevel::XdgToplevel>,
    pub(crate) seat: Option<wl_seat::WlSeat>,
    pub(crate) cursor_x: f64,
    pub(crate) cursor_y: f64,
    pub(crate) pointer_in_surface: bool,
    pub(crate) left_pressed: bool,
    pub(crate) left_released: bool,
    pub(crate) right_pressed: bool,
    pub(crate) scroll_delta: f32,
    pub(crate) pointer_serial: u32,
    pub(crate) enter_serial: u32,
    pub(crate) cursor_shape_mgr: Option<wp_cursor_shape_manager_v1::WpCursorShapeManagerV1>,
    pub(crate) cursor_shape_device: Option<wp_cursor_shape_device_v1::WpCursorShapeDeviceV1>,
    pub(crate) current_cursor_shape: Option<wp_cursor_shape_device_v1::Shape>,
    pub(crate) pointer: Option<wl_pointer::WlPointer>,
    pub(crate) key_pressed: Option<u32>,
    pub(crate) ctrl_held: bool,
    pub(crate) shift_held: bool,
    pub(crate) decoration_mgr: Option<zxdg_decoration_manager_v1::ZxdgDecorationManagerV1>,
    pub(crate) popup_backend: Option<WaylandPopupBackend<State>>,
    pub(crate) popup_closed: bool,
    pub(crate) pointer_surface: Option<wl_surface::WlSurface>,
}

impl State {
    fn new() -> Self {
        Self {
            running: true,
            configured: false,
            frame_done: true,
            width: 0,
            height: 0,
            scale: 1,
            output_phys_width: 0,
            maximized: false,
            compositor: None,
            wm_base: None,
            viewporter: None,
            surface: None,
            xdg_surface: None,
            toplevel: None,
            seat: None,
            cursor_x: 0.0,
            cursor_y: 0.0,
            pointer_in_surface: false,
            left_pressed: false,
            left_released: false,
            right_pressed: false,
            scroll_delta: 0.0,
            pointer_serial: 0,
            enter_serial: 0,
            cursor_shape_mgr: None,
            cursor_shape_device: None,
            current_cursor_shape: None,
            pointer: None,
            key_pressed: None,
            ctrl_held: false,
            shift_held: false,
            decoration_mgr: None,
            popup_backend: None,
            popup_closed: false,
            pointer_surface: None,
        }
    }

    fn fractional_scale(&self) -> f64 {
        if self.output_phys_width > 0 && self.width > 0 {
            self.output_phys_width as f64 / self.width as f64
        } else {
            self.scale.max(1) as f64
        }
    }

    fn phys_width(&self) -> u32 {
        (self.width as f64 * self.fractional_scale()).round() as u32
    }
    fn phys_height(&self) -> u32 {
        (self.height as f64 * self.fractional_scale()).round() as u32
    }
}

// ── Edge resize helper ─────────────────────────────────────────────────────

fn edge_resize(
    cx: f32,
    cy: f32,
    w: f32,
    h: f32,
    border: f32,
    controls_x: f32,
) -> Option<xdg_toplevel::ResizeEdge> {
    let left = cx < border;
    let right = cx > w - border;
    let top = cy < border;
    let bottom = cy > h - border;
    if top && cx > controls_x {
        return None;
    }
    match (left, right, top, bottom) {
        (true, _, true, _) => Some(xdg_toplevel::ResizeEdge::TopLeft),
        (_, true, true, _) => Some(xdg_toplevel::ResizeEdge::TopRight),
        (true, _, _, true) => Some(xdg_toplevel::ResizeEdge::BottomLeft),
        (_, true, _, true) => Some(xdg_toplevel::ResizeEdge::BottomRight),
        (true, _, _, _) => Some(xdg_toplevel::ResizeEdge::Left),
        (_, true, _, _) => Some(xdg_toplevel::ResizeEdge::Right),
        (_, _, true, _) => Some(xdg_toplevel::ResizeEdge::Top),
        (_, _, _, true) => Some(xdg_toplevel::ResizeEdge::Bottom),
        _ => None,
    }
}

fn resize_edge_to_cursor(edge: xdg_toplevel::ResizeEdge) -> wp_cursor_shape_device_v1::Shape {
    use wp_cursor_shape_device_v1::Shape;
    match edge {
        xdg_toplevel::ResizeEdge::Top => Shape::NResize,
        xdg_toplevel::ResizeEdge::Bottom => Shape::SResize,
        xdg_toplevel::ResizeEdge::Left => Shape::WResize,
        xdg_toplevel::ResizeEdge::Right => Shape::EResize,
        xdg_toplevel::ResizeEdge::TopLeft => Shape::NwResize,
        xdg_toplevel::ResizeEdge::TopRight => Shape::NeResize,
        xdg_toplevel::ResizeEdge::BottomLeft => Shape::SwResize,
        xdg_toplevel::ResizeEdge::BottomRight => Shape::SeResize,
        _ => Shape::Default,
    }
}

// ── Entry point ────────────────────────────────────────────────────────────

pub fn run() -> Result<()> {
    let conn = Connection::connect_to_env()?;
    let display = conn.display();
    let mut event_queue: EventQueue<State> = conn.new_event_queue();
    let qh = event_queue.handle();
    let mut state = State::new();

    display.get_registry(&qh, ());
    event_queue.roundtrip(&mut state)?;

    let compositor = state
        .compositor
        .clone()
        .ok_or_else(|| anyhow!("wl_compositor not available"))?;
    let wm_base = state
        .wm_base
        .clone()
        .ok_or_else(|| anyhow!("xdg_wm_base not available"))?;

    if state.width == 0 {
        state.width = 1280;
    }
    if state.height == 0 {
        state.height = 800;
    }

    let surface = compositor.create_surface(&qh, ());
    let xdg_surface = wm_base.get_xdg_surface(&surface, &qh, ());
    let toplevel = xdg_surface.get_toplevel(&qh, ());
    toplevel.set_title("Lantern Edit".into());
    toplevel.set_app_id("lntrn-video-editor".into());
    toplevel.set_min_size(960, 600);

    if let Some(mgr) = &state.decoration_mgr {
        let deco = mgr.get_toplevel_decoration(&toplevel, &qh, ());
        deco.set_mode(zxdg_toplevel_decoration_v1::Mode::ClientSide);
    }

    surface.commit();
    state.surface = Some(surface.clone());
    state.xdg_surface = Some(xdg_surface);
    state.toplevel = Some(toplevel.clone());

    while !state.configured {
        event_queue.blocking_dispatch(&mut state)?;
    }
    state.configured = false;

    surface.set_buffer_scale(1);
    let viewport = state.viewporter.as_ref().map(|vp| {
        let vp = vp.get_viewport(&surface, &qh, ());
        vp.set_destination(state.width as i32, state.height as i32);
        vp
    });

    // GPU init
    let display_ptr = conn.backend().display_ptr() as *mut c_void;
    let surface_ptr = Proxy::id(&surface).as_ptr() as *mut c_void;
    let wl_handle = WaylandHandle {
        display: NonNull::new(display_ptr).ok_or_else(|| anyhow!("null wl_display"))?,
        surface: NonNull::new(surface_ptr).ok_or_else(|| anyhow!("null wl_surface"))?,
    };

    let phys_w = state.phys_width().max(1);
    let phys_h = state.phys_height().max(1);
    let mut gpu = GpuContext::from_window(&wl_handle, phys_w, phys_h)
        .map_err(|e| anyhow!("GPU init failed: {e}"))?;
    let mut painter = Painter::new(&gpu);
    let mut text = TextRenderer::new(&gpu);
    let mut ix = InteractionContext::new();
    // Custom palette so menu labels and dropdowns match the lantern brown theme.
    let fox = FoxPalette {
        text: crate::chrome::text(),
        text_secondary: crate::chrome::text_dim(),
        accent: crate::chrome::accent(),
        bg: crate::chrome::BG,
        surface: crate::chrome::PANEL,
        surface_2: crate::chrome::PANEL_DARK,
        ..FoxPalette::night_sky()
    };
    let mut menu_bar = MenuBar::new(&fox);

    // Playback + preview
    let mut playback = Playback::new();
    let mut preview = PreviewMonitor::new(&gpu);
    let mut project = Project::new();

    // In-flight file picker (set when File>Open / Import Media spawned a chooser).
    let mut pending_pick: Option<PendingPick> = None;

    // True while the user is dragging the playhead in the timeline.
    let mut scrubbing = false;
    // Was playback running when the scrub started? If so, resume on release.
    let mut scrub_was_playing = false;
    // Last seek position sent during scrub — avoids hammering the decoder when
    // the cursor hasn't moved.
    let mut last_scrub_secs: f64 = -1.0;

    // Active trim drag, if any. `(clip_id, edge)` — set on mouse-down over an
    // edge handle, cleared on release.
    let mut trimming: Option<(crate::project::ClipId, crate::render::TrimEdge)> = None;
    // Active inspector slider drag.
    let mut inspector_drag: Option<crate::inspector::FieldHit> = None;

    // Open video from CLI arg if provided
    if let Some(path) = std::env::args().nth(1) {
        let path = std::path::PathBuf::from(path);
        if let Err(e) = playback.open_file(&path) {
            eprintln!("[video-editor] failed to open {}: {e}", path.display());
        } else {
            project.import_from_playback(&path, &playback);
            if let Some(new_id) = project.insert_selected_at_end() {
                let start = project
                    .timeline_clips
                    .iter()
                    .find(|c| c.id == new_id)
                    .map(|c| c.start)
                    .unwrap_or(0.0);
                playback.activate_clip_at(start, &project);
            }
        }
    }

    // Popup backend
    {
        let xdg_surf = state.xdg_surface.as_ref().unwrap().clone();
        let vp = state.viewporter.as_ref();
        let scale = state.fractional_scale() as f32;
        state.popup_backend = Some(WaylandPopupBackend::new(
            &conn,
            &compositor,
            &wm_base,
            &xdg_surf,
            vp,
            &gpu,
            scale,
            &qh,
        ));
    }

    let menus: Vec<(&str, Vec<MenuItem>)> = vec![
        (
            "File",
            vec![
                MenuItem::action(1, "New Project"),
                MenuItem::action_with(2, "Open", "Ctrl+O"),
                MenuItem::action_with(3, "Save", "Ctrl+S"),
                MenuItem::separator(),
                MenuItem::action(4, "Import Media"),
                MenuItem::separator(),
                MenuItem::action_with(actions::ACT_EXPORT_MP4_MED, "Export MP4 (medium)", "Ctrl+E"),
                MenuItem::action(actions::ACT_EXPORT_MP4_HIGH, "Export MP4 (high)"),
                MenuItem::action(actions::ACT_EXPORT_MP4_LOW, "Export MP4 (low)"),
                MenuItem::action(actions::ACT_EXPORT_GIF_SMALL, "Export GIF (480px)"),
                MenuItem::action(actions::ACT_EXPORT_GIF_LARGE, "Export GIF (720px)"),
                MenuItem::separator(),
                MenuItem::action_with(6, "Quit", "Ctrl+Q"),
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
    ];

    // ── Main loop ──────────────────────────────────────────────────────────
    while state.running {
        if let Err(e) = event_queue.blocking_dispatch(&mut state) {
            eprintln!("[video-editor] dispatch error: {e}");
            break;
        }
        if !state.frame_done {
            continue;
        }
        state.frame_done = false;

        let s = state.fractional_scale() as f32;

        if state.configured {
            state.configured = false;
            gpu.resize(state.phys_width().max(1), state.phys_height().max(1));
            surface.set_buffer_scale(1);
            if let Some(vp) = &viewport {
                vp.set_destination(state.width as i32, state.height as i32);
            }
        }

        let wf = gpu.width() as f32;
        let hf = gpu.height() as f32;

        let pointer_on_popup = state.pointer_surface.as_ref().and_then(|ps| {
            state
                .popup_backend
                .as_ref()?
                .find_popup_id_by_wl_surface(ps)
        });

        let cx = (state.cursor_x as f32) * s;
        let cy = (state.cursor_y as f32) * s;
        if pointer_on_popup.is_some() {
            ix.on_cursor_left();
        } else if state.pointer_in_surface {
            ix.on_cursor_moved(cx, cy);
        } else {
            ix.on_cursor_left();
        }
        if let Some(backend) = &mut state.popup_backend {
            let active = if state.pointer_in_surface {
                pointer_on_popup
            } else {
                None
            };
            backend.route_cursor(active, cx, cy);
        }

        // Keyboard
        if let Some(key) = state.key_pressed.take() {
            let ctrl = state.ctrl_held;
            match key {
                KEY_ESC => state.running = false,
                KEY_ENTER => {
                    if project
                        .insert_selected_at_playhead(playback.timeline_position)
                        .is_none()
                    {
                        eprintln!("[video-editor] no selected media to insert");
                    }
                }
                KEY_SPACE => playback.timeline_toggle(&project),
                KEY_S => split_at_playhead(&mut project, &playback),
                KEY_DELETE => delete_selected_clip(&mut project),
                KEY_LEFTBRACE => nudge_selected_speed(&mut project, 1.0 / 1.25),
                KEY_RIGHTBRACE => nudge_selected_speed(&mut project, 1.25),
                KEY_BACKSLASH => unlink_selected(&mut project),
                KEY_M => mute_selected_clip_track(&mut project),
                KEY_T => add_track(&mut project, crate::project::TrackKind::Video),
                KEY_E if ctrl => {
                    let req = crate::export::ExportRequest::defaults_for(
                        crate::export::ExportFormat::Mp4,
                    );
                    if let Err(e) = crate::export::start(req, &project) {
                        eprintln!("[video-editor] export: {e}");
                    }
                }
                _ => {}
            }
        }

        // Drain decoder, update timeline position, handle clip transitions.
        playback.tick(&project);
        if playback.frame_changed {
            if let Some(frame) = &playback.current_frame {
                preview.upload_frame(&gpu, frame);
            }
        }

        // Update menu bar hit zones (must run before on_click consults label_rects).
        let title_h = TITLE_BAR_H * s;
        let menubar_rect = lntrn_render::Rect::new(0.0, 0.0, wf * 0.5, title_h);
        menu_bar.update(&mut ix, &menus, menubar_rect, s);

        // Compute layout up front so click handling can hit-test the timeline.
        let layout = crate::layout::Layout::compute(wf, hf, title_h, s);
        let scrub_rect = layout.timeline_scrub_rect(s);

        // Continuous scrub: while held, keep seeking to wherever the cursor is.
        // Only re-issue a seek if the target moved by at least one frame —
        // otherwise we'd thrash the decoder when the user holds without moving.
        if scrubbing && state.pointer_in_surface {
            let prog = ((cx - scrub_rect.x) / scrub_rect.w).clamp(0.0, 1.0);
            let timeline_dur = crate::render::timeline_visible_duration(&project, &playback);
            let target = prog as f64 * timeline_dur;
            let frame_step = 1.0 / playback.fps.max(1.0) as f64;
            if (target - last_scrub_secs).abs() >= frame_step * 0.5 {
                playback.timeline_seek(target, &project);
                last_scrub_secs = target;
            }
        }

        // Continuous inspector slider drag.
        if let Some(hit) = &inspector_drag {
            apply_inspector_drag(&mut project, hit, cx, s);
        }

        // Continuous trim: while a clip edge is being dragged, map cursor x to
        // a timeline-seconds value and update the clip in the project model.
        if let Some((clip_id, edge)) = trimming {
            if let Some(secs) = crate::render::cursor_to_timeline_secs(
                &project,
                &playback,
                &layout.timeline,
                cx,
                s,
            ) {
                match edge {
                    crate::render::TrimEdge::Left => project.trim_left(clip_id, secs),
                    crate::render::TrimEdge::Right => {
                        let source_dur = project
                            .timeline_clips
                            .iter()
                            .find(|c| c.id == clip_id)
                            .and_then(|c| project.media_by_id(c.media_id))
                            .map(|m| m.duration)
                            .unwrap_or(0.0);
                        project.trim_right(clip_id, secs, source_dur);
                    }
                }
            }
        }

        // Left press
        if state.left_pressed {
            state.left_pressed = false;
            if let Some(pid) = pointer_on_popup {
                if let Some(backend) = &mut state.popup_backend {
                    if let Some(ctx) = backend.popup_render(pid) {
                        ctx.interaction.on_left_pressed();
                    }
                }
            } else {
                let border = 10.0 * s;
                let controls_x = wf - 120.0 * s;
                if let Some(edge) = edge_resize(cx, cy, wf, hf, border, controls_x) {
                    if let Some(seat) = &state.seat {
                        toplevel.resize(seat, state.pointer_serial, edge);
                    }
                } else if cy < title_h {
                    let hit_r = 20.0 * s;
                    let btn_y = title_h * 0.5;
                    let close_cx = wf - 28.0 * s;
                    let max_cx = wf - 66.0 * s;
                    let min_cx = wf - 104.0 * s;
                    let dist = |bx: f32| ((cx - bx).powi(2) + (cy - btn_y).powi(2)).sqrt();
                    if dist(close_cx) < hit_r {
                        state.running = false;
                    } else if dist(max_cx) < hit_r {
                        if state.maximized {
                            toplevel.unset_maximized();
                        } else {
                            toplevel.set_maximized();
                        }
                    } else if dist(min_cx) < hit_r {
                        toplevel.set_minimized();
                    } else if !menu_bar.on_click(&mut ix, &menus, s) {
                        if let Some(seat) = &state.seat {
                            toplevel._move(seat, state.pointer_serial);
                        }
                    }
                } else if !menu_bar.on_click(&mut ix, &menus, s) {
                    if let Some(media_id) =
                        crate::render::media_item_at(&project, &layout.media_browser, cx, cy, s)
                    {
                        let path = project.select_media(media_id).map(|item| item.path.clone());
                        if let Some(path) = path {
                            if let Err(e) = playback.open_file(&path) {
                                eprintln!("[video-editor] failed to open {}: {e}", path.display());
                            }
                        }
                    } else if let Some(track_id) = crate::render::track_mute_at(
                        &project,
                        &layout.timeline,
                        cx,
                        cy,
                        s,
                    ) {
                        project.toggle_mute(track_id);
                    } else if let Some(hit) =
                        crate::inspector::field_at(&project, &layout.properties, cx, cy, s)
                    {
                        apply_inspector_drag(&mut project, &hit, cx, s);
                        inspector_drag = Some(hit);
                    } else if let Some((clip_id, edge)) = crate::render::timeline_clip_edge_at(
                        &project,
                        &playback,
                        &layout.timeline,
                        cx,
                        cy,
                        s,
                    ) {
                        project.select_clip(clip_id);
                        trimming = Some((clip_id, edge));
                    } else if let Some(clip_id) = crate::render::timeline_clip_at(
                        &project,
                        &playback,
                        &layout.timeline,
                        cx,
                        cy,
                        s,
                    ) {
                        project.select_clip(clip_id);
                    } else if scrub_rect.contains(cx, cy)
                        && !project.timeline_clips.is_empty()
                    {
                        project.clear_clip_selection();
                        scrub_was_playing = playback.is_playing();
                        if scrub_was_playing {
                            playback.pause();
                        }
                        scrubbing = true;
                        let prog = ((cx - scrub_rect.x) / scrub_rect.w).clamp(0.0, 1.0);
                        let timeline_dur =
                            crate::render::timeline_visible_duration(&project, &playback);
                        let target = prog as f64 * timeline_dur;
                        playback.timeline_seek(target, &project);
                        last_scrub_secs = target;
                    } else {
                        ix.on_left_pressed();
                    }
                }
            }
        }

        // Left release
        if state.left_released {
            state.left_released = false;
            if scrubbing {
                scrubbing = false;
                last_scrub_secs = -1.0;
                if scrub_was_playing {
                    playback.play();
                }
                scrub_was_playing = false;
            }
            if trimming.is_some() {
                trimming = None;
            }
            if inspector_drag.is_some() {
                inspector_drag = None;
            }
            if let Some(pid) = pointer_on_popup {
                if let Some(backend) = &mut state.popup_backend {
                    if let Some(ctx) = backend.popup_render(pid) {
                        ctx.interaction.on_left_released();
                    }
                }
            } else {
                ix.on_left_released();
            }
        }

        // Right press (close menus)
        if state.right_pressed {
            state.right_pressed = false;
            menu_bar.close();
        }

        if state.popup_closed {
            state.popup_closed = false;
        }

        state.scroll_delta = 0.0;

        // Cursor shape
        if state.pointer_in_surface {
            let border = 10.0 * s;
            let controls_x = wf - 120.0 * s;
            let desired = if trimming.is_some() {
                wp_cursor_shape_device_v1::Shape::EwResize
            } else if let Some(edge) = edge_resize(cx, cy, wf, hf, border, controls_x) {
                resize_edge_to_cursor(edge)
            } else if crate::render::timeline_clip_edge_at(
                &project,
                &playback,
                &layout.timeline,
                cx,
                cy,
                s,
            )
            .is_some()
            {
                wp_cursor_shape_device_v1::Shape::EwResize
            } else {
                wp_cursor_shape_device_v1::Shape::Default
            };
            if state.current_cursor_shape != Some(desired) {
                if let Some(dev) = &state.cursor_shape_device {
                    dev.set_shape(state.enter_serial, desired);
                }
                state.current_cursor_shape = Some(desired);
            }
        }

        // ── Render ─────────────────────────────────────────────────────────
        ix.begin_frame();
        painter.clear();

        let sw = gpu.width();
        let sh = gpu.height();
        // Background + chrome
        crate::chrome::draw_background(&mut painter, wf, hf);
        crate::chrome::draw_title_bar(&mut painter, &mut text, s, wf, sw, sh);
        let menu_labels: Vec<&str> = menus.iter().map(|(l, _)| *l).collect();
        menu_bar.draw_with_labels(&mut painter, &mut text, &fox, &menu_labels, sw, sh, s);
        crate::chrome::draw_controls(&mut painter, cx, cy, s, wf, title_h);

        // NLE panels (layout computed earlier this frame for hit-testing)
        crate::render::draw_panels(
            &mut painter,
            &mut text,
            &layout,
            &project,
            &playback,
            s,
            sw,
            sh,
        );

        // Preview monitor (draws over the preview panel area)
        preview.draw(
            &mut painter,
            &mut text,
            &layout.preview,
            &playback,
            s,
            sw,
            sh,
        );

        if !state.maximized {
            crate::chrome::draw_border(&mut painter, wf, hf);
        }

        // Menu bar overlay
        menu_bar.context_menu.update(0.016);
        if let Some(evt) = menu_bar
            .context_menu
            .draw(&mut painter, &mut text, &mut ix, sw, sh)
        {
            use lntrn_ui::gpu::MenuEvent;
            if let MenuEvent::Action(id) = evt {
                dispatch_menu_action(
                    id,
                    &mut state.running,
                    &mut pending_pick,
                    &mut project,
                    &playback,
                );
                menu_bar.close();
            }
        }

        // Drain a finished file picker (non-blocking).
        let mut picked_path = None;
        if let Some(pending) = &pending_pick {
            match pending.rx.try_recv() {
                Ok(path) => {
                    picked_path = Some((pending.kind, path));
                }
                Err(crossbeam_channel::TryRecvError::Disconnected) => {
                    pending_pick = None; // user cancelled or picker errored
                }
                Err(crossbeam_channel::TryRecvError::Empty) => {}
            }
        }
        if let Some((kind, path)) = picked_path {
            if let Err(e) = playback.open_file(&path) {
                eprintln!("[video-editor] failed to open {}: {e}", path.display());
            } else {
                project.import_from_playback(&path, &playback);
                // File→Open is the "I'm going to edit this video" gesture, so
                // drop it on the timeline immediately. Import Media is the
                // browse-only staging path — leave the timeline alone.
                if matches!(kind, PickKind::Open) {
                    if let Some(new_id) = project.insert_selected_at_end() {
                        let start = project
                            .timeline_clips
                            .iter()
                            .find(|c| c.id == new_id)
                            .map(|c| c.start)
                            .unwrap_or(0.0);
                        playback.activate_clip_at(start, &project);
                    }
                }
            }
            pending_pick = None;
        }

        // Popup surfaces
        if let Some(backend) = &mut state.popup_backend {
            backend.begin_frame_all();
        }
        if let Some(backend) = &mut state.popup_backend {
            backend.render_all();
        }

        // Submit frame: painter → video texture → text
        if let Ok(mut frame) = gpu.begin_frame("video-editor") {
            let view = frame.view().clone();
            painter.render_pass(&gpu, frame.encoder_mut(), &view, Color::TRANSPARENT);
            preview.render_pass(
                &gpu,
                frame.encoder_mut(),
                &view,
                &layout.preview,
                &playback,
                &project,
                s,
            );
            text.render_queued(&gpu, frame.encoder_mut(), &view);
            frame.submit(&gpu.queue);
        }

        ix.clear_scroll();
        surface.frame(&qh, ());
        surface.commit();
    }

    Ok(())
}

fn dispatch_menu_action(
    id: u32,
    running: &mut bool,
    pending_pick: &mut Option<PendingPick>,
    project: &mut Project,
    playback: &Playback,
) {
    match id {
        actions::ACT_OPEN => {
            *pending_pick = Some(PendingPick {
                kind: PickKind::Open,
                rx: actions::spawn_video_picker("Open Video"),
            });
        }
        actions::ACT_IMPORT_MEDIA => {
            *pending_pick = Some(PendingPick {
                kind: PickKind::Import,
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

fn nudge_selected_speed(project: &mut Project, factor: f32) {
    let Some(id) = project.selected_clip else {
        return;
    };
    let linked = project.clip_by_id(id).and_then(|c| c.linked_id);
    project.nudge_speed(id, factor);
    if let Some(lid) = linked {
        project.nudge_speed(lid, factor);
    }
}

fn unlink_selected(project: &mut Project) {
    if let Some(id) = project.selected_clip {
        if !project.unlink(id) {
            eprintln!("[video-editor] selected clip has no linked partner");
        }
    }
}

fn mute_selected_clip_track(project: &mut Project) {
    if let Some(clip) = project.selected_clip_ref() {
        let tid = clip.track_id;
        project.toggle_mute(tid);
    }
}

fn add_track(project: &mut Project, kind: crate::project::TrackKind) {
    project.add_track(kind);
}

fn apply_inspector_drag(
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

fn split_at_playhead(project: &mut Project, playback: &Playback) {
    if project.split_at(playback.timeline_position).is_none() {
        eprintln!("[video-editor] playhead not inside a clip — nothing to split");
    }
}

fn delete_selected_clip(project: &mut Project) {
    if project.delete_selected_clip().is_none() {
        eprintln!("[video-editor] no selected timeline clip to delete");
    }
}

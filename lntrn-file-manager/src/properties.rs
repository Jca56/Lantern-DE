use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::SystemTime;

use crate::bg::Task;
use crate::props_load::Details;

use lntrn_render::{Color, Painter, Rect, TextRenderer};
use lntrn_ui::gpu::{FoxPalette, GradientStrip, InteractionContext};

const ZONE_PROPS_CLOSE: u32 = 800;
const ZONE_PROPS_BACKDROP: u32 = 801;
/// The panel itself: a press on it keeps the dialog open.
const ZONE_PROPS_PANEL: u32 = 802;
pub(crate) const ZONE_PROPS_CHECKSUM_ROW: u32 = 805;
const ZONE_SECTION_BASE: u32 = 810; // 810..817 for 8 sections

const DIALOG_W: f32 = 520.0;
/// Wider for audio files: cover art + a two-column tag grid.
const AUDIO_DIALOG_W: f32 = 680.0;
const PADDING: f32 = 24.0;
const ROW_H: f32 = 28.0;
const SECTION_H: f32 = 34.0;
const TITLE_FONT: f32 = 22.0;
const SUBTITLE_FONT: f32 = 15.0;
const LABEL_FONT: f32 = 16.0;
const LABEL_W: f32 = 120.0;
const CLOSE_BTN_SIZE: f32 = 28.0;
const CORNER_R: f32 = 12.0;
const ICON_SIZE: f32 = 64.0;
const BAR_H: f32 = 10.0;

// Section indices
pub(crate) const SEC_GENERAL: usize = 0;
const SEC_MEDIA: usize = 1;
pub(crate) const SEC_DISK: usize = 2;
pub(crate) const SEC_SYSTEM: usize = 3;
pub(crate) const SEC_PERMS: usize = 4;
const SEC_SYMLINK: usize = 5;
pub(crate) const SEC_CHECKSUM: usize = 6;
const SEC_AUDIO: usize = 7;

/// Gathered file properties for display.
#[allow(dead_code)]
pub struct FileProperties {
    pub path: PathBuf,
    pub name: String,
    pub file_type: String,
    pub mime_type: String,
    pub size: String,
    pub size_bytes: u64,
    pub location: String,
    pub modified: String,
    pub created: String,
    pub accessed: String,
    pub permissions: String,
    pub permissions_mode: u32,
    pub owner: String,
    pub group: String,
    pub is_dir: bool,
    pub is_symlink: bool,
    pub symlink_target: Option<String>,
    // System
    pub inode: u64,
    pub device_id: u64,
    pub hard_links: u64,
    pub block_size: u64,
    pub blocks: u64,
    // Disk
    pub disk_total: u64,
    pub disk_free: u64,
    pub disk_used_fraction: f32,
    // Media (populated separately via populate_media_info)
    pub image_dimensions: Option<(u32, u32)>,
    pub media_duration: Option<String>,
    /// WAV / MP3 tag editor — present only for supported audio files.
    pub audio: Option<crate::properties_audio::AudioEdit>,
    // UI state
    pub section_open: [bool; 8],
    /// How far the information rows are scrolled (props_scroll.rs).
    pub scroll_offset: f32,
    /// How far the icon picker's grid is scrolled.
    pub picker_scroll: f32,
    /// The scrolling part as it was last drawn: what the wheel, the
    /// scrollbar and the texture pass work from.
    pub scroll_view: Option<crate::props_scroll::ScrollView>,
    /// Lazy SHA-256 — spawned the first time the Checksum section opens.
    pub checksum_job: Option<crate::checksums::ChecksumJob>,
    /// Set by draw_properties_dialog for render.rs to place the icon texture.
    pub icon_rect: Option<(f32, f32, f32, f32)>,
    /// When true, the Properties body is replaced with the icon picker.
    pub picker_open: bool,
    pub picker_tab: IconPickerTab,
    /// Per-cell rects from the icon picker grid, exposed so render.rs can
    /// draw the actual SVG thumbnails (icon_cache is borrowed in the
    /// renderer; the picker body can't touch it). Cleared each frame; each
    /// tuple is (icon_path, x, y, w, h).
    pub picker_cell_rects: Vec<(PathBuf, f32, f32, f32, f32)>,
    /// Icon list of the picker tab last shown, so the folder is read when
    /// the tab changes and not on every frame.
    pub(crate) picker_icons: Option<(IconPickerTab, Vec<PathBuf>)>,
    /// Size and mtime as the directory listing reports them (lstat), i.e.
    /// what the thumbnail cache keys this file's texture on.
    pub listing_size: u64,
    pub listing_modified: Option<SystemTime>,
    /// A folder's custom icon and colour, for the icon in the header.
    pub folder_icon: Option<String>,
    pub folder_color: Option<String>,
    /// The item is on a phone or network mount: no checksum, no tags.
    pub slow: bool,
    /// What the filesystem has to say, on its way from a worker thread
    /// (props_load.rs). The rows show "…" until it is here.
    pub(crate) details: Option<Task<Option<Details>>>,
    /// The folder attributes being read again after an icon change.
    pub(crate) look: Option<Task<(Option<String>, Option<String>)>>,
    /// Raised (with a wake-up) by the custom-icon picker thread once it has
    /// changed the folder's icon: the listings are read again then.
    pub(crate) refresh: Arc<AtomicBool>,
}

/// Shown in a row whose value is still being read.
pub(crate) const PENDING: &str = "\u{2026}";

/// Categories shown as tabs in the icon picker. The first three map to
/// `~/.lantern/icons/folders/{Standard,Colors,Awesome}/`; Custom opens a
/// file picker.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IconPickerTab {
    Standard,
    Colors,
    Awesome,
    Custom,
}

impl IconPickerTab {
    pub fn all() -> &'static [IconPickerTab] {
        &[
            IconPickerTab::Standard,
            IconPickerTab::Colors,
            IconPickerTab::Awesome,
            IconPickerTab::Custom,
        ]
    }
    pub fn label(self) -> &'static str {
        match self {
            IconPickerTab::Standard => "Standard",
            IconPickerTab::Colors => "Colors",
            IconPickerTab::Awesome => "Awesome",
            IconPickerTab::Custom => "Custom",
        }
    }
    pub fn dir_name(self) -> Option<&'static str> {
        match self {
            IconPickerTab::Standard => Some("Standard"),
            IconPickerTab::Colors => Some("Colors"),
            IconPickerTab::Awesome => Some("Awesome"),
            IconPickerTab::Custom => None,
        }
    }
}

/// List the SVG files in a given picker category directory.
pub fn list_picker_icons(tab: IconPickerTab) -> Vec<PathBuf> {
    let Some(sub) = tab.dir_name() else {
        return Vec::new();
    };
    let home = std::env::var("HOME").unwrap_or_else(|_| "/".into());
    let dir = PathBuf::from(home).join(".lantern/icons/folders").join(sub);
    let mut out: Vec<PathBuf> = std::fs::read_dir(&dir)
        .ok()
        .into_iter()
        .flat_map(|rd| rd.flatten())
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("svg"))
        .collect();
    out.sort();
    out
}

impl FileProperties {
    pub fn populate_media_info(&mut self, file_info: &mut crate::file_info::FileInfoCache) {
        // The state the listing showed: a file edited since is read again.
        let info = file_info.get(&self.path, (self.listing_size, self.listing_modified));
        self.image_dimensions = info.dimensions;
        self.media_duration = info.duration.clone();
    }

    fn has_media_section(&self) -> bool {
        // The Audio section owns duration for WAV / MP3.
        (self.image_dimensions.is_some() || self.media_duration.is_some()) && self.audio.is_none()
    }

    fn has_symlink_section(&self) -> bool {
        self.is_symlink
    }
}

// ── Helpers ────────────────────────────────────────────────────────────────

fn format_size(bytes: u64) -> String {
    if bytes < 1024 {
        return format!("{} B", bytes);
    }
    let kb = bytes as f64 / 1024.0;
    if kb < 1024.0 {
        return format!("{:.1} KB", kb);
    }
    let mb = kb / 1024.0;
    if mb < 1024.0 {
        return format!("{:.1} MB", mb);
    }
    let gb = mb / 1024.0;
    format!("{:.2} GB", gb)
}

pub(crate) fn format_size_with_bytes(bytes: u64) -> String {
    let human = format_size(bytes);
    if bytes < 1024 {
        return human;
    }
    // Add comma-separated byte count
    let mut s = bytes.to_string();
    let mut i = s.len() as isize - 3;
    while i > 0 {
        s.insert(i as usize, ',');
        i -= 3;
    }
    format!("{} ({} bytes)", human, s)
}

pub(crate) fn format_time(time: SystemTime) -> String {
    let Some(t) = crate::datetime::local(time) else {
        return "—".into();
    };
    let (h12, ampm) = t.hour12();
    format!(
        "{} {} {}, {:02}:{:02} {}",
        t.month_name(),
        t.day,
        t.year,
        h12,
        t.minute,
        ampm
    )
}

pub(crate) fn format_permissions(mode: u32, is_dir: bool) -> String {
    let d = if is_dir { "d" } else { "-" };
    let r = |bit: u32| if mode & bit != 0 { "r" } else { "-" };
    let w = |bit: u32| if mode & bit != 0 { "w" } else { "-" };
    let x = |bit: u32| if mode & bit != 0 { "x" } else { "-" };
    format!(
        "{}{}{}{}{}{}{}{}{}{} ({:o})",
        d,
        r(0o400),
        w(0o200),
        x(0o100),
        r(0o040),
        w(0o020),
        x(0o010),
        r(0o004),
        w(0o002),
        x(0o001),
        mode & 0o7777,
    )
}

pub(crate) fn mime_from_ext(ext: &str) -> String {
    match ext {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "bmp" => "image/bmp",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "ico" => "image/x-icon",
        "mp4" => "video/mp4",
        "mkv" => "video/x-matroska",
        "avi" => "video/x-msvideo",
        "webm" => "video/webm",
        "mov" => "video/quicktime",
        "mp3" => "audio/mpeg",
        "flac" => "audio/flac",
        "ogg" => "audio/ogg",
        "wav" => "audio/wav",
        "m4a" => "audio/mp4",
        "pdf" => "application/pdf",
        "zip" => "application/zip",
        "gz" | "tgz" => "application/gzip",
        "tar" => "application/x-tar",
        "rs" => "text/x-rust",
        "py" => "text/x-python",
        "js" => "text/javascript",
        "ts" => "text/typescript",
        "html" | "htm" => "text/html",
        "css" => "text/css",
        "json" => "application/json",
        "toml" => "application/toml",
        "yaml" | "yml" => "application/yaml",
        "xml" => "application/xml",
        "md" => "text/markdown",
        "txt" | "log" => "text/plain",
        "sh" | "bash" => "text/x-shellscript",
        "c" => "text/x-c",
        "cpp" | "cc" => "text/x-c++",
        "h" => "text/x-c-header",
        "java" => "text/x-java",
        "go" => "text/x-go",
        _ => "application/octet-stream",
    }
    .to_string()
}

// ── Drawing ────────────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
pub fn draw_properties_dialog(
    props: &mut FileProperties,
    painter: &mut Painter,
    text: &mut TextRenderer,
    ix: &mut InteractionContext,
    fox: &FoxPalette,
    screen_w: f32,
    screen_h: f32,
    s: f32,
    sw: u32,
    sh: u32,
) {
    let dialog_w = if props.audio.is_some() {
        AUDIO_DIALOG_W
    } else {
        DIALOG_W
    } * s;
    let pad = PADDING * s;
    let row_h = ROW_H * s;
    let section_h = SECTION_H * s;
    let label_font = LABEL_FONT * s;
    let label_w = LABEL_W * s;
    let corner_r = CORNER_R * s;
    let close_sz = CLOSE_BTN_SIZE * s;
    let icon_sz = ICON_SIZE * s;
    let bar_h = BAR_H * s;

    // Calculate content height. The header (icon, name, subtitle, divider)
    // stays put; `content_h` counts the rows below it, which scroll when
    // the window is too short for all of them.
    let header_h = pad
        + icon_sz
        + 8.0 * s
        + TITLE_FONT * s
        + 4.0 * s
        + SUBTITLE_FONT * s
        + pad * 0.5
        + 4.0 * s
        + pad * 0.25;
    // The scrolling part stops this far above the panel's bottom edge, clear
    // of its rounded corners.
    let foot = 8.0 * s;
    let mut content_h = 0.0;

    // Audio section (WAV / MP3 tags)
    if let Some(audio) = &props.audio {
        content_h += section_h;
        if props.section_open[SEC_AUDIO] {
            content_h += audio.body_height(s);
        }
    }

    // General section
    content_h += section_h;
    if props.section_open[SEC_GENERAL] {
        content_h += 6.0 * row_h;
    }

    // Media section (conditional)
    if props.has_media_section() {
        content_h += section_h;
        if props.section_open[SEC_MEDIA] {
            if props.image_dimensions.is_some() {
                content_h += row_h;
            }
            if props.media_duration.is_some() {
                content_h += row_h;
            }
        }
    }

    // Disk section
    content_h += section_h;
    if props.section_open[SEC_DISK] {
        content_h += bar_h + 8.0 * s + row_h;
    }

    // System section
    content_h += section_h;
    if props.section_open[SEC_SYSTEM] {
        content_h += 5.0 * row_h;
    }

    // Permissions section
    content_h += section_h;
    if props.section_open[SEC_PERMS] {
        content_h += 4.0 * row_h;
    }

    // Symlink section (conditional)
    if props.has_symlink_section() {
        content_h += section_h;
        if props.section_open[SEC_SYMLINK] {
            content_h += row_h;
        }
    }

    // Checksum section (files only)
    if !props.is_dir {
        content_h += section_h;
        if props.section_open[SEC_CHECKSUM] {
            content_h += row_h;
        }
    }

    content_h += pad - foot; // bottom padding

    let picking = props.picker_open && props.is_dir;
    // The picker is sized for itself (five rows of icons), not for however
    // many information rows happen to be unfolded behind it.
    let body_h = if picking {
        crate::props_picker::PICKER_BODY_H * s
    } else {
        content_h
    };
    let dialog_h =
        crate::props_scroll::panel_height(header_h + body_h + foot, header_h, screen_h, s);
    let dialog_x = (screen_w - dialog_w) / 2.0;
    // Never above the window: the close button is at the top.
    let dialog_y = ((screen_h - dialog_h) / 2.0).max(0.0);
    props.scroll_view = None;

    // Clear picker cell rects up front — stale entries from a previous
    // frame would otherwise keep painting thumbnails over the regular body
    // after the picker closes (the renderer doesn't know on its own that
    // the picker isn't drawing this frame).
    props.picker_cell_rects.clear();
    // Same for the cover art: its rect is only set while the Audio section
    // draws, and a folded section must not leave the art painted on top of
    // whatever moved up into its place.
    if let Some(audio) = props.audio.as_mut() {
        audio.art_rect = None;
    }

    // Backdrop
    let backdrop = Rect::new(0.0, 0.0, screen_w, screen_h);
    ix.add_zone(ZONE_PROPS_BACKDROP, backdrop);
    painter.rect_filled(backdrop, 0.0, Color::rgba(0.0, 0.0, 0.0, 0.55));

    // Shadow + panel
    let panel = Rect::new(dialog_x, dialog_y, dialog_w, dialog_h);
    let shadow = Rect::new(
        panel.x - 8.0 * s,
        panel.y - 4.0 * s,
        panel.w + 16.0 * s,
        panel.h + 16.0 * s,
    );
    painter.rect_filled(shadow, corner_r + 4.0 * s, Color::rgba(0.0, 0.0, 0.0, 0.3));
    painter.rect_filled(panel, corner_r, fox.surface);
    painter.rect_stroke_sdf(panel, corner_r, 1.0 * s, fox.muted.with_alpha(0.2));

    // Panel zone (clicks inside don't close)
    let _panel_zone = ix.add_zone(ZONE_PROPS_PANEL, panel);

    let inner_x = dialog_x + pad;
    let inner_w = dialog_w - pad * 2.0;
    let mut cy = dialog_y + pad;

    // Close button (X) — top right
    let close_rect = Rect::new(dialog_x + dialog_w - pad - close_sz, cy, close_sz, close_sz);
    let close_zone = ix.add_zone(ZONE_PROPS_CLOSE, close_rect);
    let close_bg = if close_zone.is_hovered() {
        fox.danger.with_alpha(0.2)
    } else {
        Color::rgba(0.0, 0.0, 0.0, 0.0)
    };
    painter.rect_filled(close_rect, 4.0 * s, close_bg);
    let bx = close_rect.x + close_sz / 2.0;
    let by = close_rect.y + close_sz / 2.0;
    let cr = 7.0 * s;
    painter.line(
        bx - cr,
        by - cr,
        bx + cr,
        by + cr,
        2.0 * s,
        fox.text_secondary,
    );
    painter.line(
        bx + cr,
        by - cr,
        bx - cr,
        by + cr,
        2.0 * s,
        fox.text_secondary,
    );

    // ── Header: icon + name + subtitle ──────────────────────────────────
    let icon_x = dialog_x + (dialog_w - icon_sz) / 2.0;
    // Store icon rect — render.rs will draw the actual file icon texture here
    props.icon_rect = Some((icon_x, cy, icon_sz, icon_sz));
    // Fallback background circle (visible if no icon texture loads)
    let icon_box = Rect::new(icon_x, cy, icon_sz, icon_sz);
    painter.rect_filled(icon_box, icon_sz / 2.0, fox.accent.with_alpha(0.1));
    // Clickable hint ring — folders only, since file icons aren't customizable.
    // (What a click on it does is in props_click.rs, like every click in
    // this dialog: taken on the press, not read off the zone while drawing.)
    if props.is_dir {
        let icon_zone = ix.add_zone(crate::ZONE_PROPS_ICON, icon_box);
        if icon_zone.is_hovered() {
            painter.rect_stroke_sdf(icon_box, icon_sz / 2.0, 2.0 * s, fox.accent);
        }
    }
    cy += icon_sz + 8.0 * s;

    // Filename centered
    let title_font_s = TITLE_FONT * s;
    let name_w = text.measure_width(&props.name, title_font_s);
    let name_x = dialog_x + (dialog_w - name_w) / 2.0;
    text.queue(
        &props.name,
        title_font_s,
        name_x.max(inner_x),
        cy,
        fox.text,
        inner_w,
        sw,
        sh,
    );
    cy += title_font_s + 4.0 * s;

    // Subtitle: "PNG Image · 2.4 MB"
    let subtitle_font_s = SUBTITLE_FONT * s;
    let subtitle = if props.is_dir {
        props.file_type.clone()
    } else {
        format!("{} · {}", props.file_type, format_size(props.size_bytes))
    };
    let sub_w = text.measure_width(&subtitle, subtitle_font_s);
    let sub_x = dialog_x + (dialog_w - sub_w) / 2.0;
    text.queue(
        &subtitle,
        subtitle_font_s,
        sub_x.max(inner_x),
        cy,
        fox.text_secondary,
        inner_w,
        sw,
        sh,
    );
    cy += subtitle_font_s + pad * 0.5;

    // Gradient strip as the divider between the icon header and whatever
    // sits below (info body or icon picker). Spans the full panel width
    // (edge-to-edge) so it reads as a section break rather than an inset rule.
    let mut gradient = GradientStrip::new(dialog_x, cy, dialog_w);
    gradient.height = 4.0 * s;
    gradient.colors = fox.file_manager_gradient_stops();
    gradient.draw(painter);
    cy += 4.0 * s + pad * 0.25;

    // If the picker is open, replace the rest of the body with it.
    if picking {
        crate::props_picker::draw_icon_picker_body(
            props,
            painter,
            text,
            ix,
            fox,
            inner_x,
            cy,
            inner_w,
            dialog_y + dialog_h - cy - pad,
            s,
            sw,
            sh,
        );
        return;
    }

    // The rows scroll inside what is left of the panel.
    let body = Rect::new(panel.x, cy, panel.w, panel.y + panel.h - foot - cy);
    cy = props.begin_scroll(painter, text, body, content_h);

    // ── Audio section (WAV / MP3 tags) ──────────────────────────────────
    if props.audio.is_some() {
        cy = draw_section_header(
            "Audio", SEC_AUDIO, props, painter, text, ix, fox, inner_x, cy, inner_w, section_h, s,
            sw, sh,
        );
        if props.section_open[SEC_AUDIO] {
            if let Some(audio) = props.audio.as_mut() {
                cy += audio.draw(painter, text, ix, fox, inner_x, cy, inner_w, s, sw, sh);
            }
        }
    }

    // ── General section ─────────────────────────────────────────────────
    cy = draw_section_header(
        "General",
        SEC_GENERAL,
        props,
        painter,
        text,
        ix,
        fox,
        inner_x,
        cy,
        inner_w,
        section_h,
        s,
        sw,
        sh,
    );
    if props.section_open[SEC_GENERAL] {
        cy = draw_row(
            painter,
            text,
            fox,
            "Kind",
            &props.file_type,
            inner_x,
            cy,
            inner_w,
            label_w,
            label_font,
            row_h,
            sw,
            sh,
        );
        let size_display = props.size.clone();
        cy = draw_row(
            painter,
            text,
            fox,
            "Size",
            &size_display,
            inner_x,
            cy,
            inner_w,
            label_w,
            label_font,
            row_h,
            sw,
            sh,
        );
        let location = props.location.clone();
        cy = draw_row(
            painter, text, fox, "Where", &location, inner_x, cy, inner_w, label_w, label_font,
            row_h, sw, sh,
        );
        let created = props.created.clone();
        cy = draw_row(
            painter, text, fox, "Created", &created, inner_x, cy, inner_w, label_w, label_font,
            row_h, sw, sh,
        );
        let modified = props.modified.clone();
        cy = draw_row(
            painter, text, fox, "Modified", &modified, inner_x, cy, inner_w, label_w, label_font,
            row_h, sw, sh,
        );
        let accessed = props.accessed.clone();
        cy = draw_row(
            painter, text, fox, "Accessed", &accessed, inner_x, cy, inner_w, label_w, label_font,
            row_h, sw, sh,
        );
    }

    // ── Image/Media section (conditional) ───────────────────────────────
    if props.has_media_section() {
        cy = draw_section_header(
            "Media Details",
            SEC_MEDIA,
            props,
            painter,
            text,
            ix,
            fox,
            inner_x,
            cy,
            inner_w,
            section_h,
            s,
            sw,
            sh,
        );
        if props.section_open[SEC_MEDIA] {
            if let Some((w, h)) = props.image_dimensions {
                let dim = format!("{} × {}", w, h);
                cy = draw_row(
                    painter,
                    text,
                    fox,
                    "Dimensions",
                    &dim,
                    inner_x,
                    cy,
                    inner_w,
                    label_w,
                    label_font,
                    row_h,
                    sw,
                    sh,
                );
            }
            if let Some(ref dur) = props.media_duration {
                let dur = dur.clone();
                cy = draw_row(
                    painter, text, fox, "Duration", &dur, inner_x, cy, inner_w, label_w,
                    label_font, row_h, sw, sh,
                );
            }
        }
    }

    // ── Disk Usage section ──────────────────────────────────────────────
    cy = draw_section_header(
        "Disk Usage",
        SEC_DISK,
        props,
        painter,
        text,
        ix,
        fox,
        inner_x,
        cy,
        inner_w,
        section_h,
        s,
        sw,
        sh,
    );
    if props.section_open[SEC_DISK] && props.disk_total > 0 {
        // Progress bar
        let bar_w = inner_w;
        let track = Rect::new(inner_x, cy, bar_w, bar_h);
        painter.rect_filled(track, bar_h / 2.0, fox.surface_2);
        let fill_w = bar_w * props.disk_used_fraction;
        let fill = Rect::new(inner_x, cy, fill_w, bar_h);
        let fill_color = if props.disk_used_fraction > 0.9 {
            fox.danger
        } else if props.disk_used_fraction > 0.75 {
            fox.warning
        } else {
            fox.accent
        };
        painter.rect_filled(fill, bar_h / 2.0, fill_color);
        cy += bar_h + 8.0 * s;

        let pct = format!("{:.0}% used", props.disk_used_fraction * 100.0);
        let disk_text = format!(
            "{} free of {}",
            format_size(props.disk_free),
            format_size(props.disk_total)
        );
        let full_text = format!("{} — {}", pct, disk_text);
        cy = draw_row(
            painter, text, fox, "", &full_text, inner_x, cy, inner_w, 0.0, label_font, row_h, sw,
            sh,
        );
    } else if props.section_open[SEC_DISK] {
        cy = draw_row(
            painter,
            text,
            fox,
            "",
            if props.loading() { PENDING } else { "Unavailable" },
            inner_x,
            cy,
            inner_w,
            0.0,
            label_font,
            row_h,
            sw,
            sh,
        );
    }

    // ── System section ──────────────────────────────────────────────────
    cy = draw_section_header(
        "System", SEC_SYSTEM, props, painter, text, ix, fox, inner_x, cy, inner_w, section_h, s,
        sw, sh,
    );
    if props.section_open[SEC_SYSTEM] && props.loading() {
        // Not zeros: the numbers are not known yet.
        for label in ["Inode", "Device", "Hard Links", "Block Size", "Blocks"] {
            cy = draw_row(
                painter, text, fox, label, PENDING, inner_x, cy, inner_w, label_w, label_font,
                row_h, sw, sh,
            );
        }
    } else if props.section_open[SEC_SYSTEM] {
        let inode = format!("{}", props.inode);
        cy = draw_row(
            painter, text, fox, "Inode", &inode, inner_x, cy, inner_w, label_w, label_font, row_h,
            sw, sh,
        );
        // dev_t is not 8+8 bits on Linux: NVMe's major is 259.
        let dev_major = libc::major(props.device_id as libc::dev_t);
        let dev_minor = libc::minor(props.device_id as libc::dev_t);
        let device = format!("{}:{}", dev_major, dev_minor);
        cy = draw_row(
            painter, text, fox, "Device", &device, inner_x, cy, inner_w, label_w, label_font,
            row_h, sw, sh,
        );
        let links = format!("{}", props.hard_links);
        cy = draw_row(
            painter,
            text,
            fox,
            "Hard Links",
            &links,
            inner_x,
            cy,
            inner_w,
            label_w,
            label_font,
            row_h,
            sw,
            sh,
        );
        let blk_sz = format_size(props.block_size);
        cy = draw_row(
            painter,
            text,
            fox,
            "Block Size",
            &blk_sz,
            inner_x,
            cy,
            inner_w,
            label_w,
            label_font,
            row_h,
            sw,
            sh,
        );
        let blocks = format!("{}", props.blocks);
        cy = draw_row(
            painter, text, fox, "Blocks", &blocks, inner_x, cy, inner_w, label_w, label_font,
            row_h, sw, sh,
        );
    }

    // ── Permissions section ─────────────────────────────────────────────
    cy = draw_section_header(
        "Permissions",
        SEC_PERMS,
        props,
        painter,
        text,
        ix,
        fox,
        inner_x,
        cy,
        inner_w,
        section_h,
        s,
        sw,
        sh,
    );
    if props.section_open[SEC_PERMS] {
        let mode = props.permissions_mode;
        cy = draw_perm_row(
            painter,
            text,
            fox,
            "Owner",
            &props.owner.clone(),
            mode,
            6,
            inner_x,
            cy,
            inner_w,
            label_w,
            label_font,
            row_h,
            s,
            sw,
            sh,
        );
        cy = draw_perm_row(
            painter,
            text,
            fox,
            "Group",
            &props.group.clone(),
            mode,
            3,
            inner_x,
            cy,
            inner_w,
            label_w,
            label_font,
            row_h,
            s,
            sw,
            sh,
        );
        cy = draw_perm_row(
            painter, text, fox, "Other", "", mode, 0, inner_x, cy, inner_w, label_w, label_font,
            row_h, s, sw, sh,
        );
        let octal = if props.loading() {
            PENDING.to_string()
        } else {
            format!("{:04o}", mode & 0o7777)
        };
        cy = draw_row(
            painter, text, fox, "Mode", &octal, inner_x, cy, inner_w, label_w, label_font, row_h,
            sw, sh,
        );
    }

    // ── Symlink section (conditional) ───────────────────────────────────
    if props.has_symlink_section() {
        cy = draw_section_header(
            "Symlink",
            SEC_SYMLINK,
            props,
            painter,
            text,
            ix,
            fox,
            inner_x,
            cy,
            inner_w,
            section_h,
            s,
            sw,
            sh,
        );
        if props.section_open[SEC_SYMLINK] {
            cy = {
                let target = props
                    .symlink_target
                    .clone()
                    .unwrap_or_else(|| "Unknown".into());
                draw_row(
                    painter, text, fox, "Target", &target, inner_x, cy, inner_w, label_w,
                    label_font, row_h, sw, sh,
                )
            };
        }
    }

    // ── Checksum section (files only) ───────────────────────────────────
    if !props.is_dir {
        cy = draw_section_header(
            "Checksum",
            SEC_CHECKSUM,
            props,
            painter,
            text,
            ix,
            fox,
            inner_x,
            cy,
            inner_w,
            section_h,
            s,
            sw,
            sh,
        );
        if props.section_open[SEC_CHECKSUM] && props.slow {
            // Hashing reads the whole file, and on a phone reading means
            // downloading all of it under the device's one lock.
            let _ = draw_row(
                painter,
                text,
                fox,
                "SHA-256",
                "Not computed on phones and network folders",
                inner_x,
                cy,
                inner_w,
                label_w,
                label_font,
                row_h,
                sw,
                sh,
            );
        } else if props.section_open[SEC_CHECKSUM] {
            if props.checksum_job.is_none() {
                props.checksum_job = Some(crate::checksums::ChecksumJob::spawn(props.path.clone()));
            }
            match props.checksum_job.as_ref().and_then(|j| j.get()) {
                Some(hash) => {
                    let row_rect = Rect::new(inner_x, cy, inner_w, row_h);
                    let hovered = props
                        .visible_part(row_rect)
                        .is_some_and(|r| ix.add_zone(ZONE_PROPS_CHECKSUM_ROW, r).is_hovered());
                    if hovered {
                        painter.rect_filled(row_rect, 4.0 * s, fox.accent.with_alpha(0.08));
                    }
                    // 64 hex chars overflow the value column — show a prefix,
                    // copy the full hash on click.
                    let display = format!("{}…  (click to copy)", &hash[..20.min(hash.len())]);
                    let _ = draw_row(
                        painter, text, fox, "SHA-256", &display, inner_x, cy, inner_w, label_w,
                        label_font, row_h, sw, sh,
                    );
                }
                None => {
                    let _ = draw_row(
                        painter,
                        text,
                        fox,
                        "SHA-256",
                        "Computing…",
                        inner_x,
                        cy,
                        inner_w,
                        label_w,
                        label_font,
                        row_h,
                        sw,
                        sh,
                    );
                }
            }
        }
    }

    props.end_scroll(painter, text, ix, fox, s);
    // Rows scrolled out of the body keep their zones; put the panel (and
    // above and below it the backdrop) over them, then the header's own
    // zones back on top.
    crate::props_scroll::mask_outside_viewport(
        ix,
        panel,
        body,
        screen_h,
        ZONE_PROPS_PANEL,
        ZONE_PROPS_BACKDROP,
    );
    ix.add_zone(ZONE_PROPS_CLOSE, close_rect);
    if props.is_dir {
        ix.add_zone(crate::ZONE_PROPS_ICON, icon_box);
    }
}

// ── Section header with toggle triangle ────────────────────────────────────

#[allow(clippy::too_many_arguments)]
fn draw_section_header(
    label: &str,
    section_idx: usize,
    props: &FileProperties,
    painter: &mut Painter,
    text: &mut TextRenderer,
    ix: &mut InteractionContext,
    fox: &FoxPalette,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    s: f32,
    sw: u32,
    sh: u32,
) -> f32 {
    let zone_id = ZONE_SECTION_BASE + section_idx as u32;
    let rect = Rect::new(x, y, w, h);
    // A header scrolled part-way out of the body is a button only where it
    // can be seen.
    let hovered = props
        .visible_part(rect)
        .is_some_and(|r| ix.add_zone(zone_id, r).is_hovered());

    // Subtle separator line above
    painter.rect_filled(Rect::new(x, y, w, 1.0 * s), 0.0, fox.muted.with_alpha(0.12));

    // Hover highlight
    if hovered {
        painter.rect_filled(rect, 4.0 * s, fox.text.with_alpha(0.04));
    }

    // Triangle indicator
    let tri_sz = 8.0 * s;
    let tri_x = x + 2.0 * s;
    let tri_cy = y + h / 2.0;
    let open = props.section_open[section_idx];
    if open {
        // Down-pointing triangle
        painter.triangle(
            tri_x,
            tri_cy - tri_sz * 0.3,
            tri_x + tri_sz,
            tri_cy - tri_sz * 0.3,
            tri_x + tri_sz * 0.5,
            tri_cy + tri_sz * 0.4,
            fox.text_secondary,
        );
    } else {
        // Right-pointing triangle
        painter.triangle(
            tri_x + 2.0 * s,
            tri_cy - tri_sz * 0.5,
            tri_x + tri_sz,
            tri_cy,
            tri_x + 2.0 * s,
            tri_cy + tri_sz * 0.5,
            fox.text_secondary,
        );
    }

    // Section label
    let font_sz = 15.0 * s;
    text.queue(
        label,
        font_sz,
        x + tri_sz + 8.0 * s,
        y + (h - font_sz) / 2.0,
        fox.text,
        w - tri_sz - 8.0 * s,
        sw,
        sh,
    );

    y + h
}

// ── Row helpers ────────────────────────────────────────────────────────────

/// Extra layout width for text already cut to fit: queued with exactly its
/// own measured width as the limit, its last glyph can wrap away.
const WRAP_SLACK: f32 = 4.0;

#[allow(clippy::too_many_arguments)]
fn draw_row(
    _painter: &mut Painter,
    text: &mut TextRenderer,
    fox: &FoxPalette,
    label: &str,
    value: &str,
    x: f32,
    y: f32,
    w: f32,
    label_w: f32,
    font: f32,
    row_h: f32,
    sw: u32,
    sh: u32,
) -> f32 {
    let ty = y + (row_h - font) / 2.0;
    if !label.is_empty() {
        text.queue(label, font, x, ty, fox.text_secondary, label_w, sw, sh);
    }
    // Cut to the column. Inside the scrolling body text is bounded by the
    // body, not by its own line: a value left to wrap (a long path) would
    // show its second line on top of the next row.
    let value_w = w - label_w;
    let (shown, _) = crate::sections::fit_label(text, value, value_w, font);
    text.queue(&shown, font, x + label_w, ty, fox.text, value_w + WRAP_SLACK, sw, sh);
    y + row_h
}

/// Draw a permissions row with visual [r][w][x] indicators.
#[allow(clippy::too_many_arguments)]
fn draw_perm_row(
    painter: &mut Painter,
    text: &mut TextRenderer,
    fox: &FoxPalette,
    role: &str,
    name: &str,
    mode: u32,
    shift: u32,
    x: f32,
    y: f32,
    _w: f32,
    label_w: f32,
    font: f32,
    row_h: f32,
    s: f32,
    sw: u32,
    sh: u32,
) -> f32 {
    let ty = y + (row_h - font) / 2.0;
    // Role label
    text.queue(role, font, x, ty, fox.text_secondary, label_w * 0.5, sw, sh);
    // Name (owner/group), cut to its column like the values in `draw_row`.
    if !name.is_empty() {
        let (shown, _) = crate::sections::fit_label(text, name, label_w * 0.6, font);
        text.queue(
            &shown,
            font,
            x + label_w * 0.5,
            ty,
            fox.text,
            label_w * 0.6 + WRAP_SLACK,
            sw,
            sh,
        );
    }

    // rwx boxes
    let box_sz = 22.0 * s;
    let box_gap = 4.0 * s;
    let box_x = x + label_w + 12.0 * s;
    let box_y = y + (row_h - box_sz) / 2.0;
    let perms = [("r", 2), ("w", 1), ("x", 0)];

    for (i, &(ch, bit_offset)) in perms.iter().enumerate() {
        let bx = box_x + i as f32 * (box_sz + box_gap);
        let active = mode & (1 << (shift + bit_offset)) != 0;
        let rect = Rect::new(bx, box_y, box_sz, box_sz);
        let bg = if active {
            fox.accent.with_alpha(0.2)
        } else {
            fox.muted.with_alpha(0.08)
        };
        let fg = if active {
            fox.accent
        } else {
            fox.muted.with_alpha(0.3)
        };
        painter.rect_filled(rect, 4.0 * s, bg);
        let char_w = text.measure_width(ch, font);
        text.queue(
            ch,
            font,
            bx + (box_sz - char_w) / 2.0,
            box_y + (box_sz - font) / 2.0,
            fg,
            box_sz,
            sw,
            sh,
        );
    }

    y + row_h
}

/// What a click in the dialog asks the app to do (props_click.rs).
#[derive(Debug, PartialEq)]
pub enum PropertiesEvent {
    /// User picked an icon — apply via icons::set_folder_icon and close picker.
    IconChosen(PathBuf),
    /// "Reset" — clear the folder icon xattr.
    IconReset,
    /// Put this text on the clipboard (checksum click-to-copy).
    CopyText(String),
}

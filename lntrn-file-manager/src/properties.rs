use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use lntrn_render::{Color, Painter, Rect, TextRenderer};
use lntrn_ui::gpu::{FoxPalette, GradientStrip, InteractionContext};

const ZONE_PROPS_CLOSE: u32 = 800;
const ZONE_PROPS_BACKDROP: u32 = 801;
const ZONE_SECTION_BASE: u32 = 810; // 810..815 for 6 sections

const DIALOG_W: f32 = 520.0;
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
const SEC_GENERAL: usize = 0;
const SEC_MEDIA: usize = 1;
const SEC_DISK: usize = 2;
const SEC_SYSTEM: usize = 3;
const SEC_PERMS: usize = 4;
const SEC_SYMLINK: usize = 5;

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
    // UI state
    pub section_open: [bool; 6],
    pub scroll_offset: f32,
    /// Set by draw_properties_dialog for render.rs to place the icon texture.
    pub icon_rect: Option<(f32, f32, f32, f32)>,
    /// When true, the Properties body is replaced with the icon picker.
    pub picker_open: bool,
    pub picker_tab: IconPickerTab,
    /// Edge-detect tracking — `is_active()` returns true every frame while
    /// the mouse is held, so a simple "if active → toggle" pumps the picker
    /// open/closed at 60Hz. Track previous frame to fire on the press edge.
    pub icon_was_active: bool,
    /// Per-cell rects from the icon picker grid, exposed so render.rs can
    /// draw the actual SVG thumbnails (icon_cache is borrowed in the
    /// renderer; the picker body can't touch it). Cleared each frame; each
    /// tuple is (icon_path, x, y, w, h).
    pub picker_cell_rects: Vec<(PathBuf, f32, f32, f32, f32)>,
}

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
            IconPickerTab::Standard, IconPickerTab::Colors,
            IconPickerTab::Awesome, IconPickerTab::Custom,
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
    let Some(sub) = tab.dir_name() else { return Vec::new(); };
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
    pub fn from_path(path: &Path) -> Option<Self> {
        let sym_meta = std::fs::symlink_metadata(path).ok()?;
        let is_symlink = sym_meta.file_type().is_symlink();
        let symlink_target = if is_symlink {
            std::fs::read_link(path).ok().map(|t| t.to_string_lossy().to_string())
        } else {
            None
        };

        let meta = std::fs::metadata(path).ok()?;
        let name = path.file_name()?.to_string_lossy().to_string();
        let is_dir = meta.is_dir();

        let ext = path.extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_default();

        let file_type = if is_dir {
            "Folder".to_string()
        } else if ext.is_empty() {
            "File".to_string()
        } else {
            format!("{} File", ext.to_uppercase())
        };

        let mime_type = if is_dir { "inode/directory".into() } else { mime_from_ext(&ext) };

        let size_bytes = if is_dir { 0 } else { meta.len() };
        let size = if is_dir {
            let count = std::fs::read_dir(path).map(|d| d.count()).unwrap_or(0);
            format!("{} items", count)
        } else {
            format_size_with_bytes(size_bytes)
        };

        let location = path.parent()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();

        let modified = meta.modified().ok().map(format_time).unwrap_or_else(|| "Unknown".into());
        let created = meta.created().ok().map(format_time).unwrap_or_else(|| "Unknown".into());
        let accessed = meta.accessed().ok().map(format_time).unwrap_or_else(|| "Unknown".into());

        let mode = meta.mode();
        let permissions = format_permissions(mode, is_dir);
        let owner = get_username(meta.uid()).unwrap_or_else(|| format!("{}", meta.uid()));
        let group = get_groupname(meta.gid()).unwrap_or_else(|| format!("{}", meta.gid()));

        let (disk_total, disk_free, disk_used_fraction) = disk_usage(path);

        Some(Self {
            path: path.to_path_buf(), name, file_type, mime_type,
            size, size_bytes, location, modified, created, accessed,
            permissions, permissions_mode: mode, owner, group,
            is_dir, is_symlink, symlink_target,
            inode: meta.ino(), device_id: meta.dev(),
            hard_links: meta.nlink(), block_size: meta.blksize(),
            blocks: meta.blocks(),
            disk_total, disk_free, disk_used_fraction,
            image_dimensions: None, media_duration: None,
            section_open: [true; 6], scroll_offset: 0.0, icon_rect: None,
            picker_open: false,
            picker_tab: IconPickerTab::Standard,
            icon_was_active: false,
            picker_cell_rects: Vec::new(),
        })
    }

    pub fn populate_media_info(&mut self, file_info: &mut crate::file_info::FileInfoCache) {
        let info = file_info.get(&self.path);
        self.image_dimensions = info.dimensions;
        self.media_duration = info.duration.clone();
    }

    fn has_media_section(&self) -> bool {
        self.image_dimensions.is_some() || self.media_duration.is_some()
    }

    fn has_symlink_section(&self) -> bool {
        self.is_symlink
    }
}

// ── Helpers ────────────────────────────────────────────────────────────────

fn format_size(bytes: u64) -> String {
    if bytes < 1024 { return format!("{} B", bytes); }
    let kb = bytes as f64 / 1024.0;
    if kb < 1024.0 { return format!("{:.1} KB", kb); }
    let mb = kb / 1024.0;
    if mb < 1024.0 { return format!("{:.1} MB", mb); }
    let gb = mb / 1024.0;
    format!("{:.2} GB", gb)
}

fn format_size_with_bytes(bytes: u64) -> String {
    let human = format_size(bytes);
    if bytes < 1024 { return human; }
    // Add comma-separated byte count
    let mut s = bytes.to_string();
    let mut i = s.len() as isize - 3;
    while i > 0 {
        s.insert(i as usize, ',');
        i -= 3;
    }
    format!("{} ({} bytes)", human, s)
}

fn format_time(time: SystemTime) -> String {
    let secs = time.duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let days = secs / 86400;
    let time_of_day = secs % 86400;
    let hours = time_of_day / 3600;
    let minutes = (time_of_day % 3600) / 60;
    let (year, month, day) = days_to_date(days);
    let months = ["Jan","Feb","Mar","Apr","May","Jun","Jul","Aug","Sep","Oct","Nov","Dec"];
    let month_str = months.get(month as usize).unwrap_or(&"???");
    let h12 = if hours == 0 { 12 } else if hours > 12 { hours - 12 } else { hours };
    let ampm = if hours < 12 { "AM" } else { "PM" };
    format!("{} {} {}, {:02}:{:02} {}", month_str, day, year, h12, minutes, ampm)
}

fn days_to_date(days: u64) -> (u64, u64, u64) {
    let mut y = 1970;
    let mut remaining = days;
    loop {
        let days_in_year = if is_leap(y) { 366 } else { 365 };
        if remaining < days_in_year { break; }
        remaining -= days_in_year;
        y += 1;
    }
    let dim: [u64; 12] = if is_leap(y) {
        [31,29,31,30,31,30,31,31,30,31,30,31]
    } else {
        [31,28,31,30,31,30,31,31,30,31,30,31]
    };
    let mut m = 0;
    for (i, &d) in dim.iter().enumerate() {
        if remaining < d { m = i as u64; break; }
        remaining -= d;
    }
    (y, m, remaining + 1)
}

fn is_leap(y: u64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

fn format_permissions(mode: u32, is_dir: bool) -> String {
    let d = if is_dir { "d" } else { "-" };
    let r = |bit: u32| if mode & bit != 0 { "r" } else { "-" };
    let w = |bit: u32| if mode & bit != 0 { "w" } else { "-" };
    let x = |bit: u32| if mode & bit != 0 { "x" } else { "-" };
    format!("{}{}{}{}{}{}{}{}{}{} ({:o})",
        d,
        r(0o400), w(0o200), x(0o100),
        r(0o040), w(0o020), x(0o010),
        r(0o004), w(0o002), x(0o001),
        mode & 0o7777,
    )
}

fn get_username(uid: u32) -> Option<String> {
    let pw = unsafe { libc::getpwuid(uid) };
    if pw.is_null() { return None; }
    let name = unsafe { std::ffi::CStr::from_ptr((*pw).pw_name) };
    Some(name.to_string_lossy().to_string())
}

fn get_groupname(gid: u32) -> Option<String> {
    let gr = unsafe { libc::getgrgid(gid) };
    if gr.is_null() { return None; }
    let name = unsafe { std::ffi::CStr::from_ptr((*gr).gr_name) };
    Some(name.to_string_lossy().to_string())
}

fn disk_usage(path: &Path) -> (u64, u64, f32) {
    let dir = if path.is_dir() { path } else { path.parent().unwrap_or(path) };
    let c_path = match std::ffi::CString::new(dir.to_string_lossy().as_bytes()) {
        Ok(c) => c,
        Err(_) => return (0, 0, 0.0),
    };
    unsafe {
        let mut stat: libc::statvfs = std::mem::zeroed();
        if libc::statvfs(c_path.as_ptr(), &mut stat) != 0 {
            return (0, 0, 0.0);
        }
        let total = stat.f_blocks as u64 * stat.f_frsize as u64;
        let free = stat.f_bavail as u64 * stat.f_frsize as u64;
        let used_frac = if total > 0 { 1.0 - (free as f32 / total as f32) } else { 0.0 };
        (total, free, used_frac)
    }
}

fn mime_from_ext(ext: &str) -> String {
    match ext {
        "png" => "image/png", "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif", "bmp" => "image/bmp", "webp" => "image/webp",
        "svg" => "image/svg+xml", "ico" => "image/x-icon",
        "mp4" => "video/mp4", "mkv" => "video/x-matroska",
        "avi" => "video/x-msvideo", "webm" => "video/webm", "mov" => "video/quicktime",
        "mp3" => "audio/mpeg", "flac" => "audio/flac", "ogg" => "audio/ogg",
        "wav" => "audio/wav", "m4a" => "audio/mp4",
        "pdf" => "application/pdf", "zip" => "application/zip",
        "gz" | "tgz" => "application/gzip", "tar" => "application/x-tar",
        "rs" => "text/x-rust", "py" => "text/x-python", "js" => "text/javascript",
        "ts" => "text/typescript", "html" | "htm" => "text/html",
        "css" => "text/css", "json" => "application/json",
        "toml" => "application/toml", "yaml" | "yml" => "application/yaml",
        "xml" => "application/xml", "md" => "text/markdown",
        "txt" | "log" => "text/plain", "sh" | "bash" => "text/x-shellscript",
        "c" => "text/x-c", "cpp" | "cc" => "text/x-c++",
        "h" => "text/x-c-header", "java" => "text/x-java",
        "go" => "text/x-go",
        _ => "application/octet-stream",
    }.to_string()
}

// ── Drawing ────────────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
pub fn draw_properties_dialog(
    props: &mut FileProperties,
    painter: &mut Painter,
    text: &mut TextRenderer,
    ix: &mut InteractionContext,
    fox: &FoxPalette,
    screen_w: f32, screen_h: f32,
    s: f32, sw: u32, sh: u32,
) -> Option<PropertiesEvent> {
    let dialog_w = DIALOG_W * s;
    let pad = PADDING * s;
    let row_h = ROW_H * s;
    let section_h = SECTION_H * s;
    let label_font = LABEL_FONT * s;
    let label_w = LABEL_W * s;
    let corner_r = CORNER_R * s;
    let close_sz = CLOSE_BTN_SIZE * s;
    let icon_sz = ICON_SIZE * s;
    let bar_h = BAR_H * s;

    // Calculate content height
    let header_h = pad + icon_sz + 8.0 * s + TITLE_FONT * s + 4.0 * s + SUBTITLE_FONT * s + pad * 0.5;
    let mut content_h = header_h + 1.0 * s; // separator

    // General section
    content_h += section_h;
    if props.section_open[SEC_GENERAL] { content_h += 6.0 * row_h; }

    // Media section (conditional)
    if props.has_media_section() {
        content_h += section_h;
        if props.section_open[SEC_MEDIA] {
            if props.image_dimensions.is_some() { content_h += row_h; }
            if props.media_duration.is_some() { content_h += row_h; }
        }
    }

    // Disk section
    content_h += section_h;
    if props.section_open[SEC_DISK] { content_h += bar_h + 8.0 * s + row_h; }

    // System section
    content_h += section_h;
    if props.section_open[SEC_SYSTEM] { content_h += 5.0 * row_h; }

    // Permissions section
    content_h += section_h;
    if props.section_open[SEC_PERMS] { content_h += 4.0 * row_h; }

    // Symlink section (conditional)
    if props.has_symlink_section() {
        content_h += section_h;
        if props.section_open[SEC_SYMLINK] { content_h += row_h; }
    }

    content_h += pad; // bottom padding

    let dialog_h = content_h.min(screen_h - 40.0 * s);
    let dialog_x = (screen_w - dialog_w) / 2.0;
    let dialog_y = (screen_h - dialog_h) / 2.0;

    // Clear picker cell rects up front — stale entries from a previous
    // frame would otherwise keep painting thumbnails over the regular body
    // after the picker closes (the renderer doesn't know on its own that
    // the picker isn't drawing this frame).
    props.picker_cell_rects.clear();

    // Backdrop
    let backdrop = Rect::new(0.0, 0.0, screen_w, screen_h);
    ix.add_zone(ZONE_PROPS_BACKDROP, backdrop);
    painter.rect_filled(backdrop, 0.0, Color::rgba(0.0, 0.0, 0.0, 0.55));

    // Shadow + panel
    let panel = Rect::new(dialog_x, dialog_y, dialog_w, dialog_h);
    let shadow = Rect::new(panel.x - 8.0 * s, panel.y - 4.0 * s, panel.w + 16.0 * s, panel.h + 16.0 * s);
    painter.rect_filled(shadow, corner_r + 4.0 * s, Color::rgba(0.0, 0.0, 0.0, 0.3));
    painter.rect_filled(panel, corner_r, fox.surface);
    painter.rect_stroke_sdf(panel, corner_r, 1.0 * s, fox.muted.with_alpha(0.2));

    // Panel zone (clicks inside don't close)
    let _panel_zone = ix.add_zone(802, panel);

    let inner_x = dialog_x + pad;
    let inner_w = dialog_w - pad * 2.0;
    let mut cy = dialog_y + pad;

    // Close button (X) — top right
    let close_rect = Rect::new(dialog_x + dialog_w - pad - close_sz, cy, close_sz, close_sz);
    let close_zone = ix.add_zone(ZONE_PROPS_CLOSE, close_rect);
    let close_bg = if close_zone.is_hovered() { fox.danger.with_alpha(0.2) } else { Color::rgba(0.0, 0.0, 0.0, 0.0) };
    painter.rect_filled(close_rect, 4.0 * s, close_bg);
    let bx = close_rect.x + close_sz / 2.0;
    let by = close_rect.y + close_sz / 2.0;
    let cr = 7.0 * s;
    painter.line(bx - cr, by - cr, bx + cr, by + cr, 2.0 * s, fox.text_secondary);
    painter.line(bx + cr, by - cr, bx - cr, by + cr, 2.0 * s, fox.text_secondary);

    // ── Header: icon + name + subtitle ──────────────────────────────────
    let icon_x = dialog_x + (dialog_w - icon_sz) / 2.0;
    // Store icon rect — render.rs will draw the actual file icon texture here
    props.icon_rect = Some((icon_x, cy, icon_sz, icon_sz));
    // Fallback background circle (visible if no icon texture loads)
    let icon_box = Rect::new(icon_x, cy, icon_sz, icon_sz);
    painter.rect_filled(icon_box, icon_sz / 2.0, fox.accent.with_alpha(0.1));
    // Clickable hint ring — folders only, since file icons aren't customizable.
    if props.is_dir {
        let icon_zone = ix.add_zone(crate::ZONE_PROPS_ICON, icon_box);
        if icon_zone.is_hovered() {
            painter.rect_stroke_sdf(icon_box, icon_sz / 2.0, 2.0 * s, fox.accent);
        }
        let active = icon_zone.is_active();
        if active && !props.icon_was_active {
            props.picker_open = !props.picker_open;
        }
        props.icon_was_active = active;
    }
    cy += icon_sz + 8.0 * s;

    // Filename centered
    let title_font_s = TITLE_FONT * s;
    let name_w = text.measure_width(&props.name, title_font_s);
    let name_x = dialog_x + (dialog_w - name_w) / 2.0;
    text.queue(&props.name, title_font_s, name_x.max(inner_x), cy, fox.text,
        inner_w, sw, sh);
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
    text.queue(&subtitle, subtitle_font_s, sub_x.max(inner_x), cy,
        fox.text_secondary, inner_w, sw, sh);
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
    if props.picker_open && props.is_dir {
        if let Some(evt) = draw_icon_picker_body(
            props, painter, text, ix, fox,
            inner_x, cy, inner_w, dialog_y + dialog_h - cy - pad,
            s, sw, sh,
        ) {
            return Some(evt);
        }
        if close_zone.is_active() {
            return Some(PropertiesEvent::Close);
        }
        return None;
    }

    // ── General section ─────────────────────────────────────────────────
    cy = draw_section_header(
        "General", SEC_GENERAL, props, painter, text, ix, fox,
        inner_x, cy, inner_w, section_h, s, sw, sh,
    );
    if props.section_open[SEC_GENERAL] {
        cy = draw_row(painter, text, fox, "Kind", &props.file_type, inner_x, cy, inner_w, label_w, label_font, row_h, sw, sh);
        let size_display = props.size.clone();
        cy = draw_row(painter, text, fox, "Size", &size_display, inner_x, cy, inner_w, label_w, label_font, row_h, sw, sh);
        let location = props.location.clone();
        cy = draw_row(painter, text, fox, "Where", &location, inner_x, cy, inner_w, label_w, label_font, row_h, sw, sh);
        let created = props.created.clone();
        cy = draw_row(painter, text, fox, "Created", &created, inner_x, cy, inner_w, label_w, label_font, row_h, sw, sh);
        let modified = props.modified.clone();
        cy = draw_row(painter, text, fox, "Modified", &modified, inner_x, cy, inner_w, label_w, label_font, row_h, sw, sh);
        let accessed = props.accessed.clone();
        cy = draw_row(painter, text, fox, "Accessed", &accessed, inner_x, cy, inner_w, label_w, label_font, row_h, sw, sh);
    }

    // ── Image/Media section (conditional) ───────────────────────────────
    if props.has_media_section() {
        cy = draw_section_header(
            "Media Details", SEC_MEDIA, props, painter, text, ix, fox,
            inner_x, cy, inner_w, section_h, s, sw, sh,
        );
        if props.section_open[SEC_MEDIA] {
            if let Some((w, h)) = props.image_dimensions {
                let dim = format!("{} × {}", w, h);
                cy = draw_row(painter, text, fox, "Dimensions", &dim, inner_x, cy, inner_w, label_w, label_font, row_h, sw, sh);
            }
            if let Some(ref dur) = props.media_duration {
                let dur = dur.clone();
                cy = draw_row(painter, text, fox, "Duration", &dur, inner_x, cy, inner_w, label_w, label_font, row_h, sw, sh);
            }
        }
    }

    // ── Disk Usage section ──────────────────────────────────────────────
    cy = draw_section_header(
        "Disk Usage", SEC_DISK, props, painter, text, ix, fox,
        inner_x, cy, inner_w, section_h, s, sw, sh,
    );
    if props.section_open[SEC_DISK] && props.disk_total > 0 {
        // Progress bar
        let bar_w = inner_w;
        let track = Rect::new(inner_x, cy, bar_w, bar_h);
        painter.rect_filled(track, bar_h / 2.0, fox.surface_2);
        let fill_w = bar_w * props.disk_used_fraction;
        let fill = Rect::new(inner_x, cy, fill_w, bar_h);
        let fill_color = if props.disk_used_fraction > 0.9 { fox.danger }
            else if props.disk_used_fraction > 0.75 { fox.warning }
            else { fox.accent };
        painter.rect_filled(fill, bar_h / 2.0, fill_color);
        cy += bar_h + 8.0 * s;

        let pct = format!("{:.0}% used", props.disk_used_fraction * 100.0);
        let disk_text = format!("{} free of {}", format_size(props.disk_free), format_size(props.disk_total));
        let full_text = format!("{} — {}", pct, disk_text);
        cy = draw_row(painter, text, fox, "", &full_text, inner_x, cy, inner_w, 0.0, label_font, row_h, sw, sh);
    } else if props.section_open[SEC_DISK] {
        cy = draw_row(painter, text, fox, "", "Unavailable", inner_x, cy, inner_w, 0.0, label_font, row_h, sw, sh);
    }

    // ── System section ──────────────────────────────────────────────────
    cy = draw_section_header(
        "System", SEC_SYSTEM, props, painter, text, ix, fox,
        inner_x, cy, inner_w, section_h, s, sw, sh,
    );
    if props.section_open[SEC_SYSTEM] {
        let inode = format!("{}", props.inode);
        cy = draw_row(painter, text, fox, "Inode", &inode, inner_x, cy, inner_w, label_w, label_font, row_h, sw, sh);
        let dev_major = (props.device_id >> 8) & 0xFF;
        let dev_minor = props.device_id & 0xFF;
        let device = format!("{}:{}", dev_major, dev_minor);
        cy = draw_row(painter, text, fox, "Device", &device, inner_x, cy, inner_w, label_w, label_font, row_h, sw, sh);
        let links = format!("{}", props.hard_links);
        cy = draw_row(painter, text, fox, "Hard Links", &links, inner_x, cy, inner_w, label_w, label_font, row_h, sw, sh);
        let blk_sz = format_size(props.block_size);
        cy = draw_row(painter, text, fox, "Block Size", &blk_sz, inner_x, cy, inner_w, label_w, label_font, row_h, sw, sh);
        let blocks = format!("{}", props.blocks);
        cy = draw_row(painter, text, fox, "Blocks", &blocks, inner_x, cy, inner_w, label_w, label_font, row_h, sw, sh);
    }

    // ── Permissions section ─────────────────────────────────────────────
    cy = draw_section_header(
        "Permissions", SEC_PERMS, props, painter, text, ix, fox,
        inner_x, cy, inner_w, section_h, s, sw, sh,
    );
    if props.section_open[SEC_PERMS] {
        let mode = props.permissions_mode;
        cy = draw_perm_row(painter, text, fox, "Owner", &props.owner.clone(), mode, 6, inner_x, cy, inner_w, label_w, label_font, row_h, s, sw, sh);
        cy = draw_perm_row(painter, text, fox, "Group", &props.group.clone(), mode, 3, inner_x, cy, inner_w, label_w, label_font, row_h, s, sw, sh);
        cy = draw_perm_row(painter, text, fox, "Other", "", mode, 0, inner_x, cy, inner_w, label_w, label_font, row_h, s, sw, sh);
        let octal = format!("{:04o}", mode & 0o7777);
        cy = draw_row(painter, text, fox, "Mode", &octal, inner_x, cy, inner_w, label_w, label_font, row_h, sw, sh);
    }

    // ── Symlink section (conditional) ───────────────────────────────────
    if props.has_symlink_section() {
        cy = draw_section_header(
            "Symlink", SEC_SYMLINK, props, painter, text, ix, fox,
            inner_x, cy, inner_w, section_h, s, sw, sh,
        );
        if props.section_open[SEC_SYMLINK] {
            let target = props.symlink_target.clone().unwrap_or_else(|| "Unknown".into());
            let _ = draw_row(painter, text, fox, "Target", &target, inner_x, cy, inner_w, label_w, label_font, row_h, sw, sh);
        }
    }

    // Handle events
    if close_zone.is_active() {
        return Some(PropertiesEvent::Close);
    }
    None
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
    x: f32, y: f32, w: f32, h: f32,
    s: f32, sw: u32, sh: u32,
) -> f32 {
    let zone_id = ZONE_SECTION_BASE + section_idx as u32;
    let rect = Rect::new(x, y, w, h);
    let zone = ix.add_zone(zone_id, rect);

    // Subtle separator line above
    painter.rect_filled(
        Rect::new(x, y, w, 1.0 * s), 0.0,
        fox.muted.with_alpha(0.12),
    );

    // Hover highlight
    if zone.is_hovered() {
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
            tri_x, tri_cy - tri_sz * 0.3,
            tri_x + tri_sz, tri_cy - tri_sz * 0.3,
            tri_x + tri_sz * 0.5, tri_cy + tri_sz * 0.4,
            fox.text_secondary,
        );
    } else {
        // Right-pointing triangle
        painter.triangle(
            tri_x + 2.0 * s, tri_cy - tri_sz * 0.5,
            tri_x + tri_sz, tri_cy,
            tri_x + 2.0 * s, tri_cy + tri_sz * 0.5,
            fox.text_secondary,
        );
    }

    // Section label
    let font_sz = 15.0 * s;
    text.queue(label, font_sz, x + tri_sz + 8.0 * s, y + (h - font_sz) / 2.0,
        fox.text, w - tri_sz - 8.0 * s, sw, sh);

    y + h
}

// ── Row helpers ────────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
fn draw_row(
    _painter: &mut Painter,
    text: &mut TextRenderer,
    fox: &FoxPalette,
    label: &str, value: &str,
    x: f32, y: f32, w: f32, label_w: f32,
    font: f32, row_h: f32,
    sw: u32, sh: u32,
) -> f32 {
    let ty = y + (row_h - font) / 2.0;
    if !label.is_empty() {
        text.queue(label, font, x, ty, fox.text_secondary, label_w, sw, sh);
    }
    text.queue(value, font, x + label_w, ty, fox.text, w - label_w, sw, sh);
    y + row_h
}

/// Draw a permissions row with visual [r][w][x] indicators.
#[allow(clippy::too_many_arguments)]
fn draw_perm_row(
    painter: &mut Painter,
    text: &mut TextRenderer,
    fox: &FoxPalette,
    role: &str, name: &str,
    mode: u32, shift: u32,
    x: f32, y: f32, _w: f32, label_w: f32,
    font: f32, row_h: f32,
    s: f32, sw: u32, sh: u32,
) -> f32 {
    let ty = y + (row_h - font) / 2.0;
    // Role label
    text.queue(role, font, x, ty, fox.text_secondary, label_w * 0.5, sw, sh);
    // Name (owner/group)
    if !name.is_empty() {
        text.queue(name, font, x + label_w * 0.5, ty, fox.text, label_w * 0.6, sw, sh);
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
        let bg = if active { fox.accent.with_alpha(0.2) } else { fox.muted.with_alpha(0.08) };
        let fg = if active { fox.accent } else { fox.muted.with_alpha(0.3) };
        painter.rect_filled(rect, 4.0 * s, bg);
        let char_w = text.measure_width(ch, font);
        text.queue(ch, font, bx + (box_sz - char_w) / 2.0, box_y + (box_sz - font) / 2.0,
            fg, box_sz, sw, sh);
    }

    y + row_h
}

pub enum PropertiesEvent {
    Close,
    /// User picked an icon — apply via icons::set_folder_icon and close picker.
    IconChosen(PathBuf),
    /// "Reset" — clear the folder icon xattr.
    IconReset,
}

// ── Icon picker body ──────────────────────────────────────────────────────

/// Cache of icon path strings per tab so we don't hammer std::fs::read_dir
/// every frame. Keyed by (tab, modified) — kept extremely simple: read once
/// per dialog session (cleared on close).
fn picker_icons_cached(props: &FileProperties) -> Vec<PathBuf> {
    list_picker_icons(props.picker_tab)
}

#[allow(clippy::too_many_arguments)]
fn draw_icon_picker_body(
    props: &mut FileProperties,
    painter: &mut Painter,
    text: &mut TextRenderer,
    ix: &mut InteractionContext,
    fox: &FoxPalette,
    x: f32, y_start: f32, w: f32, h: f32,
    s: f32, sw: u32, sh: u32,
) -> Option<PropertiesEvent> {
    let pad = 12.0 * s;
    let tab_h = 36.0 * s;
    let tab_gap = 6.0 * s;
    let footer_h = 48.0 * s;
    // Reset per-frame so removed cells (eg after switching tab) don't leak
    // their textures into the next frame's draw list.
    props.picker_cell_rects.clear();

    // ── Tabs ──────────────────────────────────────────────────────────
    let tabs = IconPickerTab::all();
    let tab_w = (w - tab_gap * (tabs.len() - 1) as f32) / tabs.len() as f32;
    for (i, t) in tabs.iter().enumerate() {
        let tx = x + (tab_w + tab_gap) * i as f32;
        let tr = Rect::new(tx, y_start, tab_w, tab_h);
        let zone_id = crate::ZONE_PROPS_PICKER_TAB_BASE + i as u32;
        let state = ix.add_zone(zone_id, tr);
        let active = *t == props.picker_tab;
        let bg = if active {
            fox.accent.with_alpha(0.20)
        } else if state.is_hovered() {
            fox.surface_2.with_alpha(0.8)
        } else {
            fox.surface_2.with_alpha(0.4)
        };
        painter.rect_filled(tr, 8.0 * s, bg);
        if active {
            painter.rect_stroke_sdf(tr, 8.0 * s, 1.5 * s, fox.accent.with_alpha(0.7));
        }
        let label_font = 16.0 * s;
        let lw = text.measure_width(t.label(), label_font);
        let lx = tr.x + (tr.w - lw) * 0.5;
        let ly = tr.y + (tr.h - label_font) * 0.5;
        text.queue(t.label(), label_font, lx, ly,
            if active { fox.text } else { fox.text_secondary },
            tr.w, sw, sh);
        if state.is_active() {
            props.picker_tab = *t;
        }
    }

    let mut y = y_start + tab_h + pad;
    let grid_h = (h - tab_h - pad - footer_h - pad).max(80.0 * s);

    // ── Grid of icons ────────────────────────────────────────────────
    if props.picker_tab == IconPickerTab::Custom {
        // Custom tab: just a "Choose Custom Image..." button.
        let btn = Rect::new(x + w * 0.25, y + grid_h * 0.4, w * 0.5, 48.0 * s);
        let state = ix.add_zone(crate::ZONE_PROPS_PICKER_BACK, btn);
        let bg = if state.is_hovered() { fox.accent } else { fox.accent.with_alpha(0.85) };
        painter.rect_filled(btn, 8.0 * s, bg);
        let label = "Choose Custom Image\u{2026}";
        let label_font = 16.0 * s;
        let lw = text.measure_width(label, label_font);
        text.queue(label, label_font, btn.x + (btn.w - lw) * 0.5,
            btn.y + (btn.h - label_font) * 0.5,
            Color::WHITE, btn.w, sw, sh);
        if state.is_active() {
            // Spawn the existing file-picker flow inline. Reuses CTX_CHANGE_ICON's
            // logic via a thread that re-applies the chosen icon on success.
            let folder = props.path.clone();
            std::thread::spawn(move || {
                let output = std::process::Command::new("lntrn-file-manager")
                    .args([
                        "--pick",
                        "--title", "Choose Folder Icon",
                        "--filters", "Images:*.png,*.svg,*.jpg,*.jpeg,*.webp,*.ico",
                    ])
                    .output();
                if let Ok(out) = output {
                    if out.status.success() {
                        let chosen = String::from_utf8_lossy(&out.stdout).trim().to_string();
                        if !chosen.is_empty() {
                            crate::icons::set_folder_icon(&folder, &chosen);
                        }
                    }
                }
            });
            props.picker_open = false;
        }
    } else {
        let icons = picker_icons_cached(props);
        let cell = 80.0 * s;
        let cell_gap = 10.0 * s;
        let cols = ((w + cell_gap) / (cell + cell_gap)).max(1.0) as usize;

        for (i, path) in icons.iter().enumerate() {
            let col = i % cols;
            let row = i / cols;
            let cx = x + col as f32 * (cell + cell_gap);
            let cy = y + row as f32 * (cell + cell_gap);
            if cy + cell > y + grid_h { break; } // overflow — needs scrolling, future polish
            let r = Rect::new(cx, cy, cell, cell);
            let zone_id = crate::ZONE_PROPS_ICON_BASE + i as u32;
            let state = ix.add_zone(zone_id, r);
            let bg = if state.is_hovered() {
                fox.accent.with_alpha(0.18)
            } else {
                fox.surface_2.with_alpha(0.5)
            };
            painter.rect_filled(r, 8.0 * s, bg);
            // Stash the cell rect so render.rs can draw the SVG thumbnail
            // (icon_cache is borrowed there; we can't touch it from here).
            // Inset slightly so the texture doesn't paint over the rounded edge.
            let inset = 8.0 * s;
            props.picker_cell_rects.push((
                path.clone(),
                r.x + inset, r.y + inset,
                r.w - inset * 2.0, r.h - inset * 2.0,
            ));
            if state.is_active() {
                return Some(PropertiesEvent::IconChosen(path.clone()));
            }
        }
        // Empty-state hint
        if icons.is_empty() {
            let msg = "No icons found in this category.";
            let font = 16.0 * s;
            let mw = text.measure_width(msg, font);
            text.queue(msg, font, x + (w - mw) * 0.5, y + grid_h * 0.4,
                fox.muted, w, sw, sh);
        }
    }

    y += grid_h + pad;

    // ── Footer: Reset + Back ──────────────────────────────────────────
    let btn_w = 130.0 * s;
    let btn_h = 40.0 * s;
    let by = y;
    let reset_rect = Rect::new(x, by, btn_w, btn_h);
    let reset_state = ix.add_zone(crate::ZONE_PROPS_PICKER_RESET, reset_rect);
    let reset_bg = if reset_state.is_hovered() {
        fox.danger.with_alpha(0.85)
    } else {
        fox.danger.with_alpha(0.6)
    };
    painter.rect_filled(reset_rect, 8.0 * s, reset_bg);
    let lbl = "Reset to Default";
    let lf = 15.0 * s;
    let lw = text.measure_width(lbl, lf);
    text.queue(lbl, lf, reset_rect.x + (reset_rect.w - lw) * 0.5,
        reset_rect.y + (reset_rect.h - lf) * 0.5,
        Color::WHITE, reset_rect.w, sw, sh);
    if reset_state.is_active() {
        return Some(PropertiesEvent::IconReset);
    }

    let back_rect = Rect::new(x + w - btn_w, by, btn_w, btn_h);
    let back_state = ix.add_zone(crate::ZONE_PROPS_PICKER_BACK + 1, back_rect);
    let back_bg = if back_state.is_hovered() {
        fox.surface_2.with_alpha(1.0)
    } else {
        fox.surface_2.with_alpha(0.7)
    };
    painter.rect_filled(back_rect, 8.0 * s, back_bg);
    let lbl = "Back";
    let lw = text.measure_width(lbl, lf);
    text.queue(lbl, lf, back_rect.x + (back_rect.w - lw) * 0.5,
        back_rect.y + (back_rect.h - lf) * 0.5,
        fox.text, back_rect.w, sw, sh);
    if back_state.is_active() {
        props.picker_open = false;
    }

    None
}

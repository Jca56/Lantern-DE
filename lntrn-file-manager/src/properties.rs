use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use lntrn_render::{Color, Painter, Rect, TextRenderer};
use lntrn_ui::gpu::{FoxPalette, InteractionContext};

const ZONE_PROPS_CLOSE: u32 = 800;
const ZONE_PROPS_BACKDROP: u32 = 801;

const DIALOG_W: f32 = 420.0;
const PADDING: f32 = 24.0;
const ROW_H: f32 = 30.0;
const TITLE_FONT: f32 = 22.0;
const LABEL_FONT: f32 = 16.0;
const VALUE_FONT: f32 = 16.0;
const LABEL_W: f32 = 120.0;
const CLOSE_BTN_SIZE: f32 = 28.0;
const CORNER_R: f32 = 12.0;

/// Gathered file properties for display.
pub struct FileProperties {
    pub path: PathBuf,
    pub name: String,
    pub file_type: String,
    pub size: String,
    pub location: String,
    pub modified: String,
    pub created: String,
    pub permissions: String,
    pub owner: String,
    pub is_dir: bool,
}

impl FileProperties {
    pub fn from_path(path: &Path) -> Option<Self> {
        let meta = std::fs::metadata(path).ok()?;
        let name = path.file_name()?.to_string_lossy().to_string();
        let is_dir = meta.is_dir();

        let file_type = if is_dir {
            "Folder".to_string()
        } else {
            let ext = path.extension()
                .map(|e| e.to_string_lossy().to_uppercase())
                .unwrap_or_default();
            if ext.is_empty() { "File".to_string() } else { format!("{} File", ext) }
        };

        let size = if is_dir {
            let count = std::fs::read_dir(path).map(|d| d.count()).unwrap_or(0);
            format!("{} items", count)
        } else {
            format_size(meta.len())
        };

        let location = path.parent()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();

        let modified = meta.modified().ok().map(format_time).unwrap_or_else(|| "Unknown".into());
        let created = meta.created().ok().map(format_time).unwrap_or_else(|| "Unknown".into());

        let mode = meta.mode();
        let permissions = format_permissions(mode, is_dir);

        let uid = meta.uid();
        let owner = get_username(uid).unwrap_or_else(|| format!("{}", uid));

        Some(Self { path: path.to_path_buf(), name, file_type, size, location, modified, created, permissions, owner, is_dir })
    }
}

fn format_size(bytes: u64) -> String {
    if bytes < 1024 { return format!("{} B", bytes); }
    let kb = bytes as f64 / 1024.0;
    if kb < 1024.0 { return format!("{:.1} KB", kb); }
    let mb = kb / 1024.0;
    if mb < 1024.0 { return format!("{:.1} MB", mb); }
    let gb = mb / 1024.0;
    format!("{:.2} GB", gb)
}

fn format_time(time: SystemTime) -> String {
    let secs = time.duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    // Simple date formatting without chrono
    let days = secs / 86400;
    let time_of_day = secs % 86400;
    let hours = time_of_day / 3600;
    let minutes = (time_of_day % 3600) / 60;

    // Approximate date from days since epoch
    let (year, month, day) = days_to_date(days);
    let months = ["Jan","Feb","Mar","Apr","May","Jun","Jul","Aug","Sep","Oct","Nov","Dec"];
    let month_str = months.get(month as usize).unwrap_or(&"???");

    let h12 = if hours == 0 { 12 } else if hours > 12 { hours - 12 } else { hours };
    let ampm = if hours < 12 { "AM" } else { "PM" };
    format!("{} {} {}, {:02}:{:02} {}", month_str, day, year, h12, minutes, ampm)
}

fn days_to_date(days: u64) -> (u64, u64, u64) {
    // Simplified Gregorian calendar calculation
    let mut y = 1970;
    let mut remaining = days;
    loop {
        let days_in_year = if is_leap(y) { 366 } else { 365 };
        if remaining < days_in_year { break; }
        remaining -= days_in_year;
        y += 1;
    }
    let days_in_months: [u64; 12] = if is_leap(y) {
        [31,29,31,30,31,30,31,31,30,31,30,31]
    } else {
        [31,28,31,30,31,30,31,31,30,31,30,31]
    };
    let mut m = 0;
    for (i, &dim) in days_in_months.iter().enumerate() {
        if remaining < dim { m = i as u64; break; }
        remaining -= dim;
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

// ── Drawing ─────────────────────────────────────────────────────────────────

pub fn draw_properties_dialog(
    props: &FileProperties,
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
    let title_font = TITLE_FONT * s;
    let label_font = LABEL_FONT * s;
    let value_font = VALUE_FONT * s;
    let label_w = LABEL_W * s;
    let corner_r = CORNER_R * s;
    let close_sz = CLOSE_BTN_SIZE * s;

    // Layout rows
    let rows: &[(&str, &str)] = &[
        ("Name", &props.name),
        ("Type", &props.file_type),
        ("Size", &props.size),
        ("Location", &props.location),
        ("Modified", &props.modified),
        ("Created", &props.created),
        ("Permissions", &props.permissions),
        ("Owner", &props.owner),
    ];

    let content_h = title_font + pad + rows.len() as f32 * row_h;
    let dialog_h = pad * 2.0 + content_h;
    let dialog_x = (screen_w - dialog_w) / 2.0;
    let dialog_y = (screen_h - dialog_h) / 2.0;

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

    let mut cy = dialog_y + pad;

    // Title bar with close button
    let icon_char = if props.is_dir { "📁" } else { "📄" };
    text.queue(icon_char, title_font, dialog_x + pad, cy, fox.text, title_font * 2.0, sw, sh);
    text.queue(&props.name, title_font, dialog_x + pad + title_font * 1.5, cy, fox.text,
        dialog_w - pad * 2.0 - title_font * 1.5 - close_sz, sw, sh);

    // Close button (X)
    let close_rect = Rect::new(dialog_x + dialog_w - pad - close_sz, cy, close_sz, close_sz);
    let close_zone = ix.add_zone(ZONE_PROPS_CLOSE, close_rect);
    let close_bg = if close_zone.is_hovered() { fox.danger.with_alpha(0.2) } else { Color::rgba(0.0, 0.0, 0.0, 0.0) };
    painter.rect_filled(close_rect, 4.0 * s, close_bg);
    let cx = close_rect.x + close_sz / 2.0;
    let ccy = close_rect.y + close_sz / 2.0;
    let cr = 7.0 * s;
    painter.line(cx - cr, ccy - cr, cx + cr, ccy + cr, 2.0 * s, fox.text_secondary);
    painter.line(cx + cr, ccy - cr, cx - cr, ccy + cr, 2.0 * s, fox.text_secondary);

    cy += title_font + pad * 0.5;

    // Separator
    painter.rect_filled(
        Rect::new(dialog_x + pad, cy, dialog_w - pad * 2.0, 1.0 * s),
        0.0, fox.muted.with_alpha(0.2),
    );
    cy += pad * 0.5;

    // Property rows
    for (label, value) in rows {
        let row_y = cy + (row_h - label_font) / 2.0;
        text.queue(label, label_font, dialog_x + pad, row_y, fox.text_secondary,
            label_w, sw, sh);
        text.queue(value, value_font, dialog_x + pad + label_w, row_y, fox.text,
            dialog_w - pad * 2.0 - label_w, sw, sh);
        cy += row_h;
    }

    // Handle events
    if close_zone.is_active() {
        return Some(PropertiesEvent::Close);
    }
    None
}

pub enum PropertiesEvent {
    Close,
}

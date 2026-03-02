use eframe::egui;
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::process::Command;
use std::sync::Mutex;

// ── Cached icon theme ────────────────────────────────────────────────────────

static ICON_THEME_CACHE: std::sync::LazyLock<Mutex<Option<String>>> =
    std::sync::LazyLock::new(|| Mutex::new(None));

static ICON_PATH_CACHE: std::sync::LazyLock<Mutex<HashMap<String, Option<String>>>> =
    std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));

// ── Icon theme detection ─────────────────────────────────────────────────────

pub fn get_icon_theme() -> String {
    if let Ok(cache) = ICON_THEME_CACHE.lock() {
        if let Some(ref theme) = *cache {
            return theme.clone();
        }
    }

    let theme = detect_icon_theme();

    if let Ok(mut cache) = ICON_THEME_CACHE.lock() {
        *cache = Some(theme.clone());
    }

    theme
}

fn detect_icon_theme() -> String {
    // gsettings — authoritative for GNOME/Cinnamon
    if let Ok(output) = Command::new("gsettings")
        .args(["get", "org.gnome.desktop.interface", "icon-theme"])
        .output()
    {
        let raw = String::from_utf8_lossy(&output.stdout);
        let name = raw.trim().trim_matches('\'').trim_matches('"').to_string();
        if !name.is_empty() {
            return name;
        }
    }

    // GTK ini files — fallback for KDE / other DEs
    let home = std::env::var("HOME").unwrap_or_default();
    let candidates = [
        format!("{}/.config/gtk-3.0/settings.ini", home),
        format!("{}/.config/gtk-4.0/settings.ini", home),
    ];
    for settings_path in &candidates {
        if let Ok(content) = fs::read_to_string(settings_path) {
            for line in content.lines() {
                if line.trim_start().starts_with("gtk-icon-theme-name") {
                    if let Some(val) = line.splitn(2, '=').nth(1) {
                        let name = val.trim().trim_matches('"').to_string();
                        if !name.is_empty() {
                            return name;
                        }
                    }
                }
            }
        }
    }

    "hicolor".to_string()
}

// ── Theme inheritance chain ──────────────────────────────────────────────────

fn theme_search_order(theme: &str) -> Vec<String> {
    let mut order = vec![theme.to_string()];
    let index_path = format!("/usr/share/icons/{}/index.theme", theme);

    if let Ok(content) = fs::read_to_string(&index_path) {
        for line in content.lines() {
            if line.trim_start().starts_with("Inherits") {
                if let Some(val) = line.splitn(2, '=').nth(1) {
                    for parent in val.split(',') {
                        let p = parent.trim().to_string();
                        if !p.is_empty() && !order.contains(&p) {
                            order.push(p);
                        }
                    }
                }
                break;
            }
        }
    }

    if !order.contains(&"hicolor".to_string()) {
        order.push("hicolor".to_string());
    }
    order
}

// ── Find icon file path ─────────────────────────────────────────────────

pub fn find_icon(icon_name: &str, theme: &str) -> Option<String> {
    let cache_key = format!("{}::{}", theme, icon_name);
    if let Ok(cache) = ICON_PATH_CACHE.lock() {
        if let Some(result) = cache.get(&cache_key) {
            return result.clone();
        }
    }

    let result = find_icon_uncached(icon_name, theme);

    if let Ok(mut cache) = ICON_PATH_CACHE.lock() {
        cache.insert(cache_key, result.clone());
    }

    result
}

fn find_icon_uncached(icon_name: &str, theme: &str) -> Option<String> {
    let themes = theme_search_order(theme);
    // Freedesktop size dirs — covers both "48x48" (Adwaita) and "48" (Breeze)
    let sizes = [
        "scalable", "48", "48x48", "64", "64x64", "32", "32x32",
        "96", "256", "256x256", "128", "128x128", "24", "24x24",
        "22", "22x22", "16", "16x16",
    ];
    let categories = [
        "places",
        "mimetypes",
        "apps",
        "devices",
        "status",
        "actions",
        "emblems",
        "categories",
    ];
    let exts = ["svg", "png", "xpm"];

    let home = std::env::var("HOME").unwrap_or_default();
    let icon_roots = [
        format!("{}/.local/share/icons", home),
        "/usr/share/icons".to_string(),
        "/usr/local/share/icons".to_string(),
    ];

    for t in &themes {
        for root in &icon_roots {
            let base = format!("{}/{}", root, t);
            for size in &sizes {
                for cat in &categories {
                    for ext in &exts {
                        // Layout A: {size}/{category} (Adwaita, hicolor)
                        let p1 = format!("{}/{}/{}/{}.{}", base, size, cat, icon_name, ext);
                        if Path::new(&p1).exists() {
                            return Some(p1);
                        }
                        // Layout B: {category}/{size} (Breeze, Papirus)
                        let p2 = format!("{}/{}/{}/{}.{}", base, cat, size, icon_name, ext);
                        if Path::new(&p2).exists() {
                            return Some(p2);
                        }
                    }
                }
            }
        }
    }

    // Pixmaps fallback
    for ext in &exts {
        let p = format!("/usr/share/pixmaps/{}.{}", icon_name, ext);
        if Path::new(&p).exists() {
            return Some(p);
        }
    }

    None
}

// ── Icon loader (converts icon files to egui textures) ───────────────────────

pub struct IconLoader;

impl IconLoader {
    pub fn new() -> Self {
        Self
    }

    /// Load an icon file (PNG or SVG) into an egui-compatible ColorImage
    pub fn load_icon(&self, path: &str) -> Option<egui::ColorImage> {
        if path.ends_with(".svg") || path.ends_with(".svgz") {
            self.load_svg(path)
        } else {
            self.load_raster(path)
        }
    }

    fn load_raster(&self, path: &str) -> Option<egui::ColorImage> {
        let img = image::open(path).ok()?.to_rgba8();
        let size = [img.width() as usize, img.height() as usize];
        let pixels: Vec<egui::Color32> = img
            .pixels()
            .map(|p| egui::Color32::from_rgba_unmultiplied(p[0], p[1], p[2], p[3]))
            .collect();
        Some(egui::ColorImage {
            size,
            pixels,
            source_size: egui::Vec2::new(size[0] as f32, size[1] as f32),
        })
    }

    fn load_svg(&self, path: &str) -> Option<egui::ColorImage> {
        let data = fs::read(path).ok()?;
        let tree = resvg::usvg::Tree::from_data(&data, &resvg::usvg::Options::default()).ok()?;

        let target_size = 48_u32;
        let svg_size = tree.size();
        let scale_x = target_size as f32 / svg_size.width();
        let scale_y = target_size as f32 / svg_size.height();
        let scale = scale_x.min(scale_y);

        let width = (svg_size.width() * scale).ceil() as u32;
        let height = (svg_size.height() * scale).ceil() as u32;

        let mut pixmap = resvg::tiny_skia::Pixmap::new(width, height)?;
        let transform = resvg::tiny_skia::Transform::from_scale(scale, scale);
        resvg::render(&tree, transform, &mut pixmap.as_mut());

        let pixels: Vec<egui::Color32> = pixmap
            .pixels()
            .iter()
            .map(|p| {
                egui::Color32::from_rgba_unmultiplied(p.red(), p.green(), p.blue(), p.alpha())
            })
            .collect();

        Some(egui::ColorImage {
            size: [width as usize, height as usize],
            pixels,
            source_size: egui::Vec2::new(width as f32, height as f32),
        })
    }

    /// Load an image file and resize it to a thumbnail
    pub fn load_thumbnail(&self, path: &str, max_size: u32) -> Option<egui::ColorImage> {
        if path.ends_with(".svg") || path.ends_with(".svgz") {
            return self.load_svg(path);
        }
        let img = image::open(path).ok()?;
        let thumb = img.thumbnail(max_size, max_size);
        let rgba = thumb.to_rgba8();
        let size = [rgba.width() as usize, rgba.height() as usize];
        let pixels: Vec<egui::Color32> = rgba
            .pixels()
            .map(|p| egui::Color32::from_rgba_unmultiplied(p[0], p[1], p[2], p[3]))
            .collect();
        Some(egui::ColorImage {
            size,
            pixels,
            source_size: egui::Vec2::new(size[0] as f32, size[1] as f32),
        })
    }
}

use std::collections::HashMap;

use lntrn_render::{GpuContext, GpuTexture, TexturePass};
use resvg::tiny_skia;
use resvg::usvg;

use crate::icons::{FileKind, IconKind};

/// Embedded folder + file icon assets.
const FOLDER_DEFAULT: &[u8] =
    include_bytes!("assets/icons/Folder Icons/Standard/lntrn-folder-desktop.svg");
const FOLDER_DOWNLOADS: &[u8] =
    include_bytes!("assets/icons/Folder Icons/Standard/lntrn-folder-downloads.svg");
const FOLDER_DOCUMENTS: &[u8] =
    include_bytes!("assets/icons/Folder Icons/Standard/lntrn-folder-documents.svg");
const FOLDER_PICTURES: &[u8] =
    include_bytes!("assets/icons/Folder Icons/Standard/lntrn-folder-pictures.svg");
const FOLDER_MUSIC: &[u8] =
    include_bytes!("assets/icons/Folder Icons/Standard/lntrn-folder-music.svg");
const FOLDER_VIDEOS: &[u8] =
    include_bytes!("assets/icons/Folder Icons/Standard/lntrn-folder-videos.svg");
const FOLDER_PROJECTS: &[u8] =
    include_bytes!("assets/icons/Folder Icons/Standard/lntrn-folder-projects.svg");

/// Generic file glyph — gradient body, corner fold, accent ribbon at bottom.
/// No `<text>` so we never need a font database (saves 50-200ms of startup).
const FILE_SVG_TEMPLATE: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 96 96">
  <defs>
    <linearGradient id="g" x1="0" y1="0" x2="0" y2="1">
      <stop offset="0%" stop-color="__C1__"/>
      <stop offset="100%" stop-color="__C2__"/>
    </linearGradient>
  </defs>
  <path d="M16 8 h44 l20 20 v52 a4 4 0 0 1 -4 4 h-60 a4 4 0 0 1 -4 -4 v-68 a4 4 0 0 1 4 -4 z"
        fill="url(#g)" stroke="rgba(0,0,0,0.18)" stroke-width="1"/>
  <path d="M60 8 v18 a2 2 0 0 0 2 2 h18 z" fill="rgba(255,255,255,0.18)"/>
  <rect x="20" y="68" width="56" height="10" rx="2" fill="__ACCENT__" opacity="0.85"/>
</svg>"##;

/// Cache key for rasterized icons.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum IconSlot {
    FolderDefault,
    FolderDownloads,
    FolderDocuments,
    FolderPictures,
    FolderMusic,
    FolderVideos,
    FolderProjects,
    FileGeneric,
    FileText,
    FileCode,
    FileImage,
    FileVideo,
    FileAudio,
    FilePdf,
    FileArchive,
    FileExecutable,
}

pub struct IconCache {
    textures: HashMap<IconSlot, GpuTexture>,
    size_px: u32,
}

impl IconCache {
    pub fn new(size_px: u32) -> Self {
        Self {
            textures: HashMap::new(),
            size_px: size_px.max(32),
        }
    }

    /// Render only the slots needed by the current item set. Any new kinds
    /// that show up later get rasterized lazily on first lookup.
    pub fn prime_for_items(
        &mut self,
        gpu: &GpuContext,
        tex_pass: &TexturePass,
        items: &[crate::icons::DesktopItem],
    ) {
        let mut needed: std::collections::HashSet<IconSlot> = items
            .iter()
            .map(|it| slot_for_item(&it.name, it.kind))
            .collect();
        // Always have the default folder cached for the New Folder path.
        needed.insert(IconSlot::FolderDefault);
        for slot in needed {
            self.ensure(gpu, tex_pass, slot);
        }
    }

    fn ensure(&mut self, gpu: &GpuContext, tex_pass: &TexturePass, slot: IconSlot) {
        if self.textures.contains_key(&slot) {
            return;
        }
        if let Some(rgba) = rasterize(slot, self.size_px) {
            let tex = tex_pass.upload(gpu, &rgba, self.size_px, self.size_px);
            self.textures.insert(slot, tex);
        }
    }

    /// Look up a texture, rasterizing on demand if not yet cached.
    pub fn texture_for(
        &mut self,
        gpu: &GpuContext,
        tex_pass: &TexturePass,
        item_name: &str,
        kind: IconKind,
    ) -> Option<&GpuTexture> {
        let slot = slot_for_item(item_name, kind);
        self.ensure(gpu, tex_pass, slot);
        self.textures.get(&slot)
    }

    /// Immutable lookup — must call `texture_for` (or `prime_for_items`) first
    /// to ensure the slot is cached. Returns None if not yet primed.
    pub fn get_cached_for(&self, item_name: &str, kind: IconKind) -> Option<&GpuTexture> {
        self.textures.get(&slot_for_item(item_name, kind))
    }

    /// Reset cache (e.g. on scale change). Caller should prime again.
    pub fn clear(&mut self, size_px: u32) {
        self.textures.clear();
        self.size_px = size_px.max(32);
    }
}

fn slot_for_item(name: &str, kind: IconKind) -> IconSlot {
    match kind {
        IconKind::Folder => {
            let n = name.to_ascii_lowercase();
            if n == "downloads" {
                IconSlot::FolderDownloads
            } else if n == "documents" {
                IconSlot::FolderDocuments
            } else if n == "pictures" || n == "images" {
                IconSlot::FolderPictures
            } else if n == "music" {
                IconSlot::FolderMusic
            } else if n == "videos" || n == "movies" {
                IconSlot::FolderVideos
            } else if n == "projects" || n == "code" || n == "src" {
                IconSlot::FolderProjects
            } else {
                IconSlot::FolderDefault
            }
        }
        IconKind::File(fk) => match fk {
            FileKind::Generic => IconSlot::FileGeneric,
            FileKind::Text => IconSlot::FileText,
            FileKind::Code => IconSlot::FileCode,
            FileKind::Image => IconSlot::FileImage,
            FileKind::Video => IconSlot::FileVideo,
            FileKind::Audio => IconSlot::FileAudio,
            FileKind::Pdf => IconSlot::FilePdf,
            FileKind::Archive => IconSlot::FileArchive,
            FileKind::Executable => IconSlot::FileExecutable,
        },
    }
}

fn rasterize(slot: IconSlot, size: u32) -> Option<Vec<u8>> {
    let svg_data: Vec<u8> = match slot {
        IconSlot::FolderDefault => FOLDER_DEFAULT.to_vec(),
        IconSlot::FolderDownloads => FOLDER_DOWNLOADS.to_vec(),
        IconSlot::FolderDocuments => FOLDER_DOCUMENTS.to_vec(),
        IconSlot::FolderPictures => FOLDER_PICTURES.to_vec(),
        IconSlot::FolderMusic => FOLDER_MUSIC.to_vec(),
        IconSlot::FolderVideos => FOLDER_VIDEOS.to_vec(),
        IconSlot::FolderProjects => FOLDER_PROJECTS.to_vec(),
        s => build_file_svg(s).into_bytes(),
    };
    rasterize_svg(&svg_data, size)
}

fn build_file_svg(slot: IconSlot) -> String {
    // (body gradient top, body gradient bottom, accent ribbon)
    let (c1, c2, accent) = match slot {
        IconSlot::FileGeneric => ("#e8e4d8", "#a8a09a", "#6e6660"),
        IconSlot::FileText => ("#f0e2cc", "#b89a78", "#7a5f3d"),
        IconSlot::FileCode => ("#b8d4ff", "#3d7bd8", "#1f4a96"),
        IconSlot::FileImage => ("#f8c4d2", "#d65d8a", "#8a2f54"),
        IconSlot::FileVideo => ("#e0baf2", "#8b4cc0", "#562878"),
        IconSlot::FileAudio => ("#b8e0b8", "#4c9c4c", "#205a20"),
        IconSlot::FilePdf => ("#ff9c9c", "#c54040", "#7a1c1c"),
        IconSlot::FileArchive => ("#f8de9a", "#b88f30", "#705414"),
        IconSlot::FileExecutable => ("#d0d0d0", "#606060", "#2a2a2a"),
        _ => ("#dcdcdc", "#9c9c9c", "#5a5a5a"),
    };
    FILE_SVG_TEMPLATE
        .replace("__C1__", c1)
        .replace("__C2__", c2)
        .replace("__ACCENT__", accent)
}

fn rasterize_svg(data: &[u8], size: u32) -> Option<Vec<u8>> {
    // No fontdb load: none of our SVGs use `<text>`, so we skip the system
    // font scan entirely.
    let opt = usvg::Options::default();
    let tree = usvg::Tree::from_data(data, &opt).ok()?;
    let svg_size = tree.size();
    let scale = size as f32 / svg_size.width().max(svg_size.height());
    let mut pixmap = tiny_skia::Pixmap::new(size, size)?;
    let transform = tiny_skia::Transform::from_scale(scale, scale);
    resvg::render(&tree, transform, &mut pixmap.as_mut());
    Some(pixmap.data().to_vec())
}

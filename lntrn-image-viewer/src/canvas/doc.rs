//! Serializable canvas document — the `.lcanvas` file contents.
//!
//! Item positions/sizes are in *canvas units*: a screen-independent plane
//! where an image placed at natural size occupies its pixel dimensions.
//! `items` order is the z-order — `items[0]` is bottommost.

use serde::{Deserialize, Serialize};

pub const CANVAS_VERSION: u32 = 1;

#[derive(Serialize, Deserialize, Clone)]
pub struct CanvasDoc {
    pub version: u32,
    pub name: String,
    pub view: CanvasView,
    pub items: Vec<CanvasItem>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct CanvasItem {
    /// Absolute path to the source image. The file is referenced, not copied.
    pub path: String,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    /// Reserved for a future rotation feature — always 0.0 in v1, but kept in
    /// the schema so v2 files with angles stay loadable here.
    #[serde(default)]
    pub angle: f32,
}

#[derive(Serialize, Deserialize, Clone, Copy)]
pub struct CanvasView {
    pub zoom: f32,
    pub pan_x: f32,
    pub pan_y: f32,
}

impl Default for CanvasView {
    fn default() -> Self {
        Self { zoom: 1.0, pan_x: 0.0, pan_y: 0.0 }
    }
}

impl CanvasDoc {
    pub fn new_empty() -> Self {
        Self {
            version: CANVAS_VERSION,
            name: String::new(),
            view: CanvasView::default(),
            items: Vec::new(),
        }
    }
}

impl CanvasItem {
    pub fn contains(&self, cx: f32, cy: f32) -> bool {
        cx >= self.x && cx < self.x + self.w && cy >= self.y && cy < self.y + self.h
    }
}

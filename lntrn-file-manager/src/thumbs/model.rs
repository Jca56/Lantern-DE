//! 3D model thumbnails: glTF 2.0 scenes (`.glb`, and `.gltf` with its
//! buffers and pictures beside it), read by our own reader and drawn on the
//! CPU by our own rasterizer, so the thumbnail workers need no GPU.
//!
//! What a picture of a model at icon size needs: its triangles where its
//! nodes (and skins, at rest) put them, base colours from factors, pictures
//! and vertex colours, cut-out and see-through materials, and light that
//! shows the shape. Geometry compressed with Draco or meshopt can't be
//! read: those files keep the generic icon.

use std::path::Path;

mod gltf;
mod material;
mod math;
mod raster;
mod scene;
#[cfg(test)]
mod tests;

/// The model at `path` drawn to fit a `fit` px square, clear round it.
/// `None` when it can't be read or holds nothing to draw.
pub(super) fn thumbnail(path: &Path, fit: u32) -> Option<image::RgbaImage> {
    let model = scene::load(path)?;
    raster::render(&model, fit)
}

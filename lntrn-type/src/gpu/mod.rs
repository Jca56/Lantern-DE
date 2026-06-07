//! GPU layer: coverage atlas + glyph quad render pipeline.

mod atlas;
mod pipeline;

pub use atlas::GlyphAtlas;
pub use pipeline::{GlyphPipeline, Quad};

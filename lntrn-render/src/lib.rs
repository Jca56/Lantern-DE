mod gpu;
mod painter;
mod shader;
mod text;

pub use gpu::GpuContext;
pub use painter::{Color, Painter, Rect};
pub use text::TextRenderer;
pub use wgpu::SurfaceError;

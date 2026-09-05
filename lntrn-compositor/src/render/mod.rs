//! Render pipeline. The `render_elements!` macro lives here because both
//! [`surface`] (the per-frame submitter) and the sibling `blur` and
//! `udev_device` modules import [`CustomRenderElements`].
//!
//! [`surface::render_surface`] is intentionally a single long function
//! (~1900 lines): it's the per-frame DRM-output pipeline, builds three
//! parallel Vec<Element> collections (windows / bottom-layers /
//! blur backdrops), and mutates `state` + `udev` + `renderer` in
//! lock-step. Each phase depends on what the previous one queued, so
//! the few extracted helpers ([`helpers`]) are the parts that *can*
//! cleanly stand alone — the rest stays together intentionally.

use smithay::backend::renderer::{
    element::{
        memory::MemoryRenderBufferRenderElement, render_elements, solid::SolidColorRenderElement,
        surface::WaylandSurfaceRenderElement, texture::TextureRenderElement,
        utils::RescaleRenderElement,
    },
    gles::{
        element::{PixelShaderElement, TextureShaderElement},
        GlesRenderer, GlesTexture,
    },
};
use smithay::desktop::space::SpaceRenderElements;

mod helpers;
pub(crate) mod scanout;
pub(crate) mod surface;

pub use crate::layer_position::layer_surface_position_logical;
pub use helpers::{SlowRenderStats, WindowSnapshot};
pub use surface::render_surface;

// Combined render element enum: space windows + cursor overlay
render_elements! {
    pub CustomRenderElements<=GlesRenderer>;
    Memory=MemoryRenderBufferRenderElement<GlesRenderer>,
    Space=SpaceRenderElements<GlesRenderer, WaylandSurfaceRenderElement<GlesRenderer>>,
    Overlay=SolidColorRenderElement,
    Surface=WaylandSurfaceRenderElement<GlesRenderer>,
    Shader=PixelShaderElement,
    Rescaled=RescaleRenderElement<WaylandSurfaceRenderElement<GlesRenderer>>,
    TextureShader=TextureShaderElement,
    Backdrop=TextureRenderElement<GlesTexture>,
    RoundedBackdrop=crate::rounded_element::RoundedBackdropElement,
    RoundedSurface=crate::rounded_element::RoundedSurfaceElement,
}

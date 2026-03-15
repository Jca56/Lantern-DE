use lntrn_render::{GpuContext, Painter, TextRenderer};

use super::input::InteractionContext;

pub struct PopupRenderContext {
    pub gpu: GpuContext,
    pub painter: Painter,
    pub text: TextRenderer,
    pub interaction: InteractionContext,
}

pub trait PopupSurface {
    /// Create a popup at (x, y) relative to a parent.
    /// `parent_popup` is `None` for root popups (parented to the window),
    /// or `Some(id)` for submenus (parented to another popup).
    fn create_popup(&mut self, parent_popup: Option<u32>, x: i32, y: i32, width: u32, height: u32) -> u32;

    /// Resize an existing popup.
    fn resize_popup(&mut self, id: u32, width: u32, height: u32);

    /// Destroy a popup surface.
    fn destroy_popup(&mut self, id: u32);

    /// Get the render context for a popup.
    fn popup_render(&mut self, id: u32) -> Option<&mut PopupRenderContext>;
}

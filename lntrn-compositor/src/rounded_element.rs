/// Wrapper element that renders a WaylandSurfaceRenderElement with a
/// rounded-corner texture shader, clipping corners via SDF in the fragment shader.

use smithay::{
    backend::renderer::{
        element::{
            surface::WaylandSurfaceRenderElement, Element, Id, Kind, RenderElement,
        },
        gles::{GlesError, GlesFrame, GlesRenderer, GlesTexProgram, Uniform},
        utils::CommitCounter,
    },
    utils::{Buffer as BufferCoords, Physical, Rectangle, Scale, Transform},
};

pub struct RoundedSurfaceElement {
    inner: WaylandSurfaceRenderElement<GlesRenderer>,
    shader: GlesTexProgram,
    tex_size: [f32; 2],
    corner_radius: f32,
}

impl RoundedSurfaceElement {
    pub fn new(
        inner: WaylandSurfaceRenderElement<GlesRenderer>,
        shader: GlesTexProgram,
        tex_size: [f32; 2],
        corner_radius: f32,
    ) -> Self {
        Self { inner, shader, tex_size, corner_radius }
    }
}

impl Element for RoundedSurfaceElement {
    fn id(&self) -> &Id { self.inner.id() }
    fn current_commit(&self) -> CommitCounter { self.inner.current_commit() }
    fn src(&self) -> Rectangle<f64, BufferCoords> { self.inner.src() }
    fn transform(&self) -> Transform { self.inner.transform() }
    fn geometry(&self, scale: Scale<f64>) -> Rectangle<i32, Physical> { self.inner.geometry(scale) }
    fn alpha(&self) -> f32 { self.inner.alpha() }
    fn kind(&self) -> Kind { self.inner.kind() }
}

impl RenderElement<GlesRenderer> for RoundedSurfaceElement {
    fn draw(
        &self,
        frame: &mut GlesFrame<'_, '_>,
        src: Rectangle<f64, BufferCoords>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        opaque_regions: &[Rectangle<i32, Physical>],
    ) -> Result<(), GlesError> {
        frame.override_default_tex_program(
            self.shader.clone(),
            vec![
                Uniform::new("tex_size", self.tex_size),
                Uniform::new("corner_radius", self.corner_radius),
            ],
        );
        let result = self.inner.draw(frame, src, dst, damage, opaque_regions);
        frame.clear_tex_program_override();
        result
    }
}

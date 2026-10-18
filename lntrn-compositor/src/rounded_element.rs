//! Rounded-corner wrappers around surface and backdrop elements, clipping
//! corners via an SDF in the fragment shader.
use smithay::{
    backend::renderer::{
        element::{
            surface::{WaylandSurfaceRenderElement, WaylandSurfaceTexture},
            texture::TextureRenderElement,
            utils::RescaleRenderElement,
            Element, Id, Kind, RenderElement,
        },
        gles::{
            GlesError, GlesFrame, GlesPixelProgram, GlesRenderer, GlesTexProgram, GlesTexture,
            Uniform,
        },
        utils::{CommitCounter, DamageSet},
        Texture,
    },
    utils::{Buffer as BufferCoords, Physical, Point, Rectangle, Scale, Size, Transform},
};

// ── Rounded window surface ──────────────────────────────────────────────────

/// Where a window's rounded clip sits: the same physical space its surface
/// elements are built in.
#[derive(Clone, Copy)]
pub struct WindowClip {
    /// The window geometry rect, the box the border ring hugs. Anything a
    /// surface draws outside it (CSD shadow margins) is cut away.
    pub rect: Rectangle<f64, Physical>,
    pub corner_radius: f32,
    /// Physical px virtually added above the rect: 0 rounds all four
    /// corners, `>= 2 * corner_radius` leaves the top pair square (window
    /// content under a server-side titlebar).
    pub top_extend: f32,
}

/// The shaders a [`RoundedSurfaceElement`] draws with.
#[derive(Clone)]
pub struct RoundedShaders {
    pub texture: GlesTexProgram,
    /// For single-pixel (solid colour) buffers, which Smithay draws without
    /// any texture program. `None` draws them unclipped.
    pub solid: Option<GlesPixelProgram>,
}

/// What a surface's buffer is, captured before the rescale wrapper hides it.
enum SurfaceContent {
    Texture {
        size: Size<i32, BufferCoords>,
        y_inverted: bool,
    },
    Solid([f32; 4]),
}

/// One surface of a window's tree, clipped to the window's rounded geometry.
///
/// The clip is evaluated in window space, not per buffer: a window's buffer
/// is not its geometry. Clients with CSD shadow margins (Electron, GTK)
/// have buffers larger than the geometry, and subsurfaces cover only part
/// of the window. Each fragment's texture coordinate is mapped back through
/// the inverse of the texture matrix Smithay draws with, so the mask lands
/// on the window's real corners whatever the surface's size, offset, crop
/// or buffer transform.
pub struct RoundedSurfaceElement {
    inner: RescaleRenderElement<WaylandSurfaceRenderElement<GlesRenderer>>,
    shaders: RoundedShaders,
    content: SurfaceContent,
    /// Element geometry when built. `draw`'s dst is this, relocated (or
    /// rescaled) by whatever draws it.
    built: Rectangle<i32, Physical>,
    clip: WindowClip,
}

impl RoundedSurfaceElement {
    /// `surface` is wrapped in a rescale about `rescale_origin`;
    /// `output_scale` is the scale the element tree was built at.
    pub fn new(
        surface: WaylandSurfaceRenderElement<GlesRenderer>,
        rescale_origin: Point<i32, Physical>,
        rescale: Scale<f64>,
        output_scale: Scale<f64>,
        clip: WindowClip,
        shaders: RoundedShaders,
    ) -> Self {
        let content = match surface.texture() {
            WaylandSurfaceTexture::Texture(tex) => SurfaceContent::Texture {
                size: tex.size(),
                y_inverted: tex.is_y_inverted(),
            },
            WaylandSurfaceTexture::SolidColor(color) => SurfaceContent::Solid(color.components()),
        };
        let inner = RescaleRenderElement::from_element(surface, rescale_origin, rescale);
        let built = inner.geometry(output_scale);
        Self {
            inner,
            shaders,
            content,
            built,
            clip,
        }
    }

    /// The shader uniforms mapping texture coordinates (`v_coords`) to
    /// clip-local px, plus the clip size and radius. `uv_to_dst` maps
    /// `v_coords` to px inside the drawn `dst`.
    fn clip_uniforms(&self, uv_to_dst: Affine, dst: Rectangle<i32, Physical>) -> [Uniform<'static>; 4] {
        // Drawn px → built px (undoes any rescale applied after building),
        // then shift from the element's origin to the clip's, pushed down
        // by top_extend so the virtual rows above the rect come first.
        let to_clip = uv_to_dst
            .then(Affine::scale(
                self.built.size.w as f64 / dst.size.w as f64,
                self.built.size.h as f64 / dst.size.h as f64,
            ))
            .then(Affine::translate(
                self.built.loc.x as f64 - self.clip.rect.loc.x,
                self.built.loc.y as f64 - self.clip.rect.loc.y + self.clip.top_extend as f64,
            ));
        let (row_x, row_y) = to_clip.rows();
        let clip_size = [
            self.clip.rect.size.w as f32,
            self.clip.rect.size.h as f32 + self.clip.top_extend,
        ];
        [
            Uniform::new("uv_to_clip_x", row_x),
            Uniform::new("uv_to_clip_y", row_y),
            Uniform::new("clip_size", clip_size),
            Uniform::new("corner_radius", self.clip.corner_radius),
        ]
    }
}

impl Element for RoundedSurfaceElement {
    fn id(&self) -> &Id {
        self.inner.id()
    }
    fn current_commit(&self) -> CommitCounter {
        self.inner.current_commit()
    }
    fn src(&self) -> Rectangle<f64, BufferCoords> {
        self.inner.src()
    }
    fn transform(&self) -> Transform {
        self.inner.transform()
    }
    fn geometry(&self, scale: Scale<f64>) -> Rectangle<i32, Physical> {
        self.inner.geometry(scale)
    }
    fn alpha(&self) -> f32 {
        self.inner.alpha()
    }
    fn kind(&self) -> Kind {
        self.inner.kind()
    }
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
        if dst.size.is_empty() || self.built.size.is_empty() {
            return Ok(());
        }
        match self.content {
            SurfaceContent::Texture { size, y_inverted } => {
                let uv_to_dst = texture_matrix(src, dst, size, self.inner.transform(), y_inverted)
                    .and_then(Affine::inverse);
                let Some(uv_to_dst) = uv_to_dst else {
                    return Ok(());
                };
                frame.override_default_tex_program(
                    self.shaders.texture.clone(),
                    self.clip_uniforms(uv_to_dst, dst).to_vec(),
                );
                let result = self.inner.draw(frame, src, dst, damage, opaque_regions);
                frame.clear_tex_program_override();
                result
            }
            SurfaceContent::Solid(color) => {
                let Some(ref shader) = self.shaders.solid else {
                    return self.inner.draw(frame, src, dst, damage, opaque_regions);
                };
                // A unit src over a unit "texture" makes v_coords run 0..1
                // across dst.
                let uv_to_dst = Affine::scale(dst.size.w as f64, dst.size.h as f64);
                let [x, y, size, radius] = self.clip_uniforms(uv_to_dst, dst);
                frame.render_pixel_shader_to(
                    shader,
                    Rectangle::from_size(Size::from((1.0, 1.0))),
                    dst,
                    Size::from((1, 1)),
                    Some(damage),
                    self.inner.alpha(),
                    &[x, y, size, radius, Uniform::new("color", color)],
                )
            }
        }
    }
}

/// A 2-D affine map: `x' = a·x + c·y + e`, `y' = b·x + d·y + f`.
#[derive(Clone, Copy)]
struct Affine {
    a: f64,
    b: f64,
    c: f64,
    d: f64,
    e: f64,
    f: f64,
}

impl Affine {
    fn scale(sx: f64, sy: f64) -> Self {
        Self { a: sx, b: 0.0, c: 0.0, d: sy, e: 0.0, f: 0.0 }
    }

    fn translate(tx: f64, ty: f64) -> Self {
        Self { a: 1.0, b: 0.0, c: 0.0, d: 1.0, e: tx, f: ty }
    }

    /// `self` first, then `next`.
    fn then(self, next: Affine) -> Affine {
        Affine {
            a: next.a * self.a + next.c * self.b,
            b: next.b * self.a + next.d * self.b,
            c: next.a * self.c + next.c * self.d,
            d: next.b * self.c + next.d * self.d,
            e: next.a * self.e + next.c * self.f + next.e,
            f: next.b * self.e + next.d * self.f + next.f,
        }
    }

    fn inverse(self) -> Option<Affine> {
        let det = self.a * self.d - self.b * self.c;
        if det.abs() < 1e-12 {
            return None;
        }
        let (a, b, c, d) = (self.d / det, -self.b / det, -self.c / det, self.a / det);
        Some(Affine {
            a,
            b,
            c,
            d,
            e: -(a * self.e + c * self.f),
            f: -(b * self.e + d * self.f),
        })
    }

    /// The two rows, as the shader's `dot(row, vec3(uv, 1.0))` reads them.
    fn rows(self) -> ([f32; 3], [f32; 3]) {
        (
            [self.a as f32, self.c as f32, self.e as f32],
            [self.b as f32, self.d as f32, self.f as f32],
        )
    }
}

/// The map from px inside `dst` to texture coordinates that Smithay's
/// `render_texture_from_to` draws with: a mirror of its private
/// `build_texture_mat` plus the y-inversion it applies after.
fn texture_matrix(
    src: Rectangle<f64, BufferCoords>,
    dst: Rectangle<i32, Physical>,
    tex: Size<i32, BufferCoords>,
    transform: Transform,
    y_inverted: bool,
) -> Option<Affine> {
    if dst.size.is_empty() || tex.is_empty() || src.size.w <= 0.0 || src.size.h <= 0.0 {
        return None;
    }
    let turned = transform.transform_size(src.size);
    let (w, h) = (turned.w, turned.h);
    // Point in the turned src box → buffer orientation.
    #[rustfmt::skip]
    let orient = match transform {
        Transform::Normal =>     Affine { a:  1.0, b:  0.0, c:  0.0, d:  1.0, e: 0.0, f: 0.0 },
        Transform::_90 =>        Affine { a:  0.0, b: -1.0, c:  1.0, d:  0.0, e: 0.0, f: w   },
        Transform::_180 =>       Affine { a: -1.0, b:  0.0, c:  0.0, d: -1.0, e: w,   f: h   },
        Transform::_270 =>       Affine { a:  0.0, b:  1.0, c: -1.0, d:  0.0, e: h,   f: 0.0 },
        Transform::Flipped =>    Affine { a: -1.0, b:  0.0, c:  0.0, d:  1.0, e: w,   f: 0.0 },
        Transform::Flipped90 =>  Affine { a:  0.0, b:  1.0, c:  1.0, d:  0.0, e: 0.0, f: 0.0 },
        Transform::Flipped180 => Affine { a:  1.0, b:  0.0, c:  0.0, d: -1.0, e: 0.0, f: h   },
        Transform::Flipped270 => Affine { a:  0.0, b: -1.0, c: -1.0, d:  0.0, e: h,   f: w   },
    };
    let m = Affine::scale(w / dst.size.w as f64, h / dst.size.h as f64)
        .then(orient)
        .then(Affine::translate(src.loc.x, src.loc.y))
        .then(Affine::scale(1.0 / tex.w as f64, 1.0 / tex.h as f64));
    Some(if y_inverted {
        m.then(Affine::scale(1.0, -1.0))
    } else {
        m
    })
}

// ── Rounded backdrop (blur texture with corner rounding) ────────────────────

pub struct RoundedBackdropElement {
    inner: TextureRenderElement<GlesTexture>,
    shader: GlesTexProgram,
    tex_size: [f32; 2],
    corner_radius: f32,
    full_tex_size: [f32; 2],
}

impl RoundedBackdropElement {
    pub fn new(
        inner: TextureRenderElement<GlesTexture>,
        shader: GlesTexProgram,
        tex_size: [f32; 2],
        corner_radius: f32,
        full_tex_size: [f32; 2],
    ) -> Self {
        Self {
            inner,
            shader,
            tex_size,
            corner_radius,
            full_tex_size,
        }
    }
}

impl Element for RoundedBackdropElement {
    fn id(&self) -> &Id {
        self.inner.id()
    }
    fn current_commit(&self) -> CommitCounter {
        self.inner.current_commit()
    }
    fn src(&self) -> Rectangle<f64, BufferCoords> {
        self.inner.src()
    }
    fn transform(&self) -> Transform {
        self.inner.transform()
    }
    fn geometry(&self, scale: Scale<f64>) -> Rectangle<i32, Physical> {
        self.inner.geometry(scale)
    }
    fn damage_since(
        &self,
        scale: Scale<f64>,
        commit: Option<CommitCounter>,
    ) -> DamageSet<i32, Physical> {
        self.inner.damage_since(scale, commit)
    }
    fn alpha(&self) -> f32 {
        self.inner.alpha()
    }
    fn kind(&self) -> Kind {
        self.inner.kind()
    }
}

impl RenderElement<GlesRenderer> for RoundedBackdropElement {
    fn draw(
        &self,
        frame: &mut GlesFrame<'_, '_>,
        src: Rectangle<f64, BufferCoords>,
        dst: Rectangle<i32, Physical>,
        damage: &[Rectangle<i32, Physical>],
        opaque_regions: &[Rectangle<i32, Physical>],
    ) -> Result<(), GlesError> {
        // Compute src_rect in texture-coordinate space (0-1) so the shader
        // can normalise v_coords to element-local position for the SDF.
        let tw = self.full_tex_size[0];
        let th = self.full_tex_size[1];
        let src_rect = [
            src.loc.x as f32 / tw,
            src.loc.y as f32 / th,
            (src.loc.x + src.size.w) as f32 / tw,
            (src.loc.y + src.size.h) as f32 / th,
        ];
        frame.override_default_tex_program(
            self.shader.clone(),
            vec![
                Uniform::new("tex_size", self.tex_size),
                Uniform::new("corner_radius", self.corner_radius),
                Uniform::new("src_rect", src_rect),
            ],
        );
        let result = RenderElement::<GlesRenderer>::draw(
            &self.inner,
            frame,
            src,
            dst,
            damage,
            opaque_regions,
        );
        frame.clear_tex_program_override();
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TRANSFORMS: [Transform; 8] = [
        Transform::Normal,
        Transform::_90,
        Transform::_180,
        Transform::_270,
        Transform::Flipped,
        Transform::Flipped90,
        Transform::Flipped180,
        Transform::Flipped270,
    ];

    fn apply(m: Affine, x: f64, y: f64) -> (f64, f64) {
        (m.a * x + m.c * y + m.e, m.b * x + m.d * y + m.f)
    }

    fn close(a: (f64, f64), b: (f64, f64)) -> bool {
        (a.0 - b.0).abs() < 1e-9 && (a.1 - b.1).abs() < 1e-9
    }

    /// Every transform maps dst's corners onto the buffer's corners, and the
    /// inverse brings them back.
    #[test]
    fn texture_matrix_corners_round_trip() {
        let src = Rectangle::<f64, BufferCoords>::from_size(Size::from((200.0, 100.0)));
        let tex = Size::<i32, BufferCoords>::from((200, 100));
        for t in TRANSFORMS {
            let turned = t.transform_size(Size::<i32, Physical>::from((200, 100)));
            let dst = Rectangle::<i32, Physical>::from_size(turned);
            let m = texture_matrix(src, dst, tex, t, false).unwrap();
            let inv = m.inverse().unwrap();
            let (w, h) = (turned.w as f64, turned.h as f64);
            let mut hit = Vec::new();
            for p in [(0.0, 0.0), (w, 0.0), (0.0, h), (w, h)] {
                let uv = apply(m, p.0, p.1);
                assert!(
                    [(0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0)].iter().any(|c| close(uv, *c)),
                    "{t:?}: {p:?} -> {uv:?}"
                );
                assert!(!hit.iter().any(|h| close(*h, uv)), "{t:?}: corner hit twice");
                hit.push(uv);
                assert!(close(apply(inv, uv.0, uv.1), p), "{t:?}: inverse");
            }
        }
    }

    /// A buffer with CSD shadow margins: the clip must start at the window
    /// geometry, inside the margin, not at the buffer edge.
    #[test]
    fn shadow_margin_maps_to_geometry() {
        let src = Rectangle::<f64, BufferCoords>::from_size(Size::from((1040.0, 840.0)));
        let dst = Rectangle::<i32, Physical>::new((300, 200).into(), (1040, 840).into());
        let tex = Size::<i32, BufferCoords>::from((1040, 840));
        let to_clip = texture_matrix(src, dst, tex, Transform::Normal, false)
            .and_then(Affine::inverse)
            .unwrap()
            .then(Affine::translate(-20.0, -20.0));
        assert!(close(apply(to_clip, 20.0 / 1040.0, 20.0 / 840.0), (0.0, 0.0)));
        assert!(close(apply(to_clip, 1020.0 / 1040.0, 820.0 / 840.0), (1000.0, 800.0)));
    }
}

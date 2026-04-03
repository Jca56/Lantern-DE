/// Dual Kawase window blur pipeline.
///
/// Renders background elements to half-res offscreen texture, applies
/// dual-kawase downsample/upsample blur, creates backdrop elements.

use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            element::{texture::TextureRenderElement, Element, Id, Kind, RenderElement},
            gles::{
                GlesError, GlesRenderer, GlesTexProgram, GlesTexture, Uniform,
            },
            Bind, Color32F, Frame, Offscreen, Renderer,
        },
    },
    utils::{Buffer as BufferCoords, Physical, Point, Rectangle, Size, Transform},
};

use crate::render::CustomRenderElements;

// ── Blur state ─────────────────────────────────────────────────────────────

pub struct BlurState {
    /// Chain of textures at decreasing resolutions for downsample.
    /// textures[0] = half-res, textures[1] = quarter-res, etc.
    pub textures: Vec<GlesTexture>,
    /// An extra texture at half-res for the final upsample result.
    pub result: GlesTexture,
    pub full_size: Size<i32, Physical>,
    pub passes: usize,
}

/// Ensure blur textures exist and match the output size / pass count.
pub fn ensure_textures(
    renderer: &mut GlesRenderer,
    phys_size: Size<i32, Physical>,
    passes: usize,
    existing: &mut Option<BlurState>,
) -> bool {
    if let Some(state) = existing {
        if state.full_size == phys_size && state.passes == passes {
            return true;
        }
    }

    let mut textures = Vec::with_capacity(passes + 1);
    let mut w = phys_size.w / 2;
    let mut h = phys_size.h / 2;

    // Create progressively smaller textures: half, quarter, eighth...
    for _ in 0..=passes {
        w = w.max(1);
        h = h.max(1);
        let buf: Size<i32, BufferCoords> = Size::from((w, h));
        match Offscreen::<GlesTexture>::create_buffer(renderer, Fourcc::Abgr8888, buf) {
            Ok(t) => textures.push(t),
            Err(e) => {
                tracing::warn!("blur: texture {}x{} failed: {:?}", w, h, e);
                return false;
            }
        }
        w /= 2;
        h /= 2;
    }

    // Result texture at half-res (same size as textures[0])
    let result_w = (phys_size.w / 2).max(1);
    let result_h = (phys_size.h / 2).max(1);
    let result = match Offscreen::<GlesTexture>::create_buffer(
        renderer, Fourcc::Abgr8888, Size::from((result_w, result_h)),
    ) {
        Ok(t) => t,
        Err(e) => {
            tracing::warn!("blur: result texture failed: {:?}", e);
            return false;
        }
    };

    *existing = Some(BlurState { textures, result, full_size: phys_size, passes });
    true
}

// ── Render background + blur ───────────────────────────────────────────────

/// Render background elements to the first blur texture (half-res), then
/// run dual-kawase downsample + upsample passes.
pub fn render_and_blur(
    renderer: &mut GlesRenderer,
    state: &mut BlurState,
    bg_elements: &[CustomRenderElements],
    bg_color: Color32F,
    output_phys: Size<i32, Physical>,
    down_shader: &GlesTexProgram,
    up_shader: &GlesTexProgram,
) -> Result<(), GlesError> {
    if state.textures.is_empty() { return Ok(()); }

    let half_w = (output_phys.w / 2).max(1);
    let half_h = (output_phys.h / 2).max(1);
    let half_size = Size::<i32, Physical>::from((half_w, half_h));
    let scale_x = half_w as f64 / output_phys.w as f64;
    let scale_y = half_h as f64 / output_phys.h as f64;

    // Step 1: Render background elements into textures[0] at half-res
    {
        let mut target = renderer.bind(&mut state.textures[0])?;
        let mut frame = renderer.render(&mut target, half_size, Transform::Normal)?;
        frame.clear(bg_color, &[Rectangle::from_size(half_size)])?;

        for elem in bg_elements.iter().rev() {
            let geo = elem.geometry(smithay::utils::Scale::from(1.0));
            let src = elem.src();
            let dst = Rectangle::<i32, Physical>::new(
                Point::from((
                    (geo.loc.x as f64 * scale_x).round() as i32,
                    (geo.loc.y as f64 * scale_y).round() as i32,
                )),
                Size::from((
                    (geo.size.w as f64 * scale_x).round() as i32,
                    (geo.size.h as f64 * scale_y).round() as i32,
                )),
            );
            if dst.size.w > 0 && dst.size.h > 0 {
                let _ = elem.draw(&mut frame, src, dst, &[dst], &[]);
            }
        }
        let _ = frame.finish();
    }

    // Step 2: Dual Kawase downsample chain
    for i in 0..state.passes {
        // Clone source to break borrow conflict (Arc-backed, cheap)
        let src_tex = state.textures[i].clone();
        let src_size = tex_size(&src_tex);
        let dst_w = (src_size.w / 2).max(1);
        let dst_h = (src_size.h / 2).max(1);
        let dst_size = Size::<i32, Physical>::from((dst_w, dst_h));
        let halfpixel = [0.5 / src_size.w as f32, 0.5 / src_size.h as f32];

        let src_rect = Rectangle::<f64, BufferCoords>::new(
            Point::from((0.0, 0.0)),
            Size::from((src_size.w as f64, src_size.h as f64)),
        );
        let dst_rect = Rectangle::<i32, Physical>::from_size(dst_size);

        let mut target = renderer.bind(&mut state.textures[i + 1])?;
        let mut frame = renderer.render(&mut target, dst_size, Transform::Normal)?;
        frame.clear(Color32F::from([0.0, 0.0, 0.0, 0.0]), &[dst_rect])?;
        frame.render_texture_from_to(
            &src_tex, src_rect, dst_rect,
            &[dst_rect], &[], Transform::Normal, 1.0,
            Some(down_shader),
            &[Uniform::new("halfpixel", halfpixel)],
        )?;
        let _ = frame.finish();
    }

    // Step 3: Dual Kawase upsample chain (back up to half-res → result texture)
    for i in (0..state.passes).rev() {
        // Source is the smaller texture (or last downsample level)
        let src_tex = if i == state.passes - 1 {
            state.textures[state.passes].clone()
        } else {
            // Previous upsample wrote into textures[i+1] (or result if i+1==0)
            if i + 1 == 0 { state.result.clone() } else { state.textures[i + 1].clone() }
        };
        let src_size = tex_size(&src_tex);

        let dst_tex_size = tex_size(&state.textures[i]);
        let dst_size = Size::<i32, Physical>::from((dst_tex_size.w, dst_tex_size.h));
        let halfpixel = [0.5 / dst_size.w as f32, 0.5 / dst_size.h as f32];

        let src_rect = Rectangle::<f64, BufferCoords>::new(
            Point::from((0.0, 0.0)),
            Size::from((src_size.w as f64, src_size.h as f64)),
        );
        let dst_rect = Rectangle::<i32, Physical>::from_size(dst_size);

        // Final upsample goes into result, intermediate into textures[i]
        let target_tex = if i == 0 { &mut state.result } else { &mut state.textures[i] };
        let mut target = renderer.bind(target_tex)?;
        let mut frame = renderer.render(&mut target, dst_size, Transform::Normal)?;
        frame.clear(Color32F::from([0.0, 0.0, 0.0, 0.0]), &[dst_rect])?;
        frame.render_texture_from_to(
            &src_tex, src_rect, dst_rect,
            &[dst_rect], &[], Transform::Normal, 1.0,
            Some(up_shader),
            &[Uniform::new("halfpixel", halfpixel)],
        )?;
        let _ = frame.finish();
    }

    Ok(())
}

fn tex_size(tex: &GlesTexture) -> Size<i32, Physical> {
    use smithay::backend::renderer::Texture;
    let s = tex.size();
    Size::from((s.w, s.h))
}

// ── Backdrop element ───────────────────────────────────────────────────────

/// Create a backdrop element sampling the blurred result texture.
pub fn create_backdrop(
    state: &BlurState,
    ctx_id: smithay::backend::renderer::ContextId<GlesTexture>,
    win_rect: Rectangle<i32, smithay::utils::Logical>,
    output_logical: Size<i32, smithay::utils::Logical>,
    output_scale: f64,
) -> TextureRenderElement<GlesTexture> {
    let half_w = (state.full_size.w / 2).max(1) as f64;
    let half_h = (state.full_size.h / 2).max(1) as f64;

    // Map window logical position to src rect in the half-res blur texture
    let src_x = win_rect.loc.x as f64 / output_logical.w as f64 * half_w;
    let src_y = win_rect.loc.y as f64 / output_logical.h as f64 * half_h;
    let src_w = win_rect.size.w as f64 / output_logical.w as f64 * half_w;
    let src_h = win_rect.size.h as f64 / output_logical.h as f64 * half_h;

    let loc = Point::<f64, Physical>::from((
        win_rect.loc.x as f64 * output_scale,
        win_rect.loc.y as f64 * output_scale,
    ));

    TextureRenderElement::from_static_texture(
        Id::new(),
        ctx_id,
        loc,
        state.result.clone(),
        1,
        Transform::Normal,
        Some(1.0),
        Some(Rectangle::new(
            Point::from((src_x, src_y)),
            Size::from((src_w, src_h)),
        )),
        Some(Size::from((win_rect.size.w, win_rect.size.h))),
        None,
        Kind::Unspecified,
    )
}

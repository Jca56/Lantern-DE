//! Two small render-pipeline helpers that don't need access to the
//! whole [`Lantern`] state — kept out of `surface.rs` so the giant
//! `render_surface` body stays focused on the pipeline.

use std::time::{Duration, Instant};

use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            element::{
                surface::WaylandSurfaceRenderElement,
                AsRenderElements, Element, RenderElement,
            },
            gles::{GlesRenderer, GlesTexture},
            Bind, Color32F, Frame, Offscreen, Renderer,
        },
    },
    utils::{Buffer as BufferCoords, Logical, Physical, Point, Rectangle, Scale, Size, Transform},
};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;

/// Drain a pending presentation-time feedback for every surface in `space`
/// + each live layer surface, and either signal it as `presented` or
/// `discarded` depending on `rendered`.
pub(super) fn send_presentation_feedback(
    space: &smithay::desktop::Space<smithay::desktop::Window>,
    layer_surfaces: &[smithay::wayland::shell::wlr_layer::LayerSurface],
    start_time: Instant,
    output: &smithay::output::Output,
    rendered: bool,
) {
    use smithay::wayland::compositor::SurfaceData;
    use smithay::wayland::presentation::{PresentationFeedbackCachedState, Refresh};
    use smithay::reexports::wayland_protocols::wp::presentation_time::server::wp_presentation_feedback;

    let timestamp = start_time.elapsed();
    let refresh_ns = output
        .current_mode()
        .map(|m| 1_000_000_000_000u64 / m.refresh.max(1) as u64)
        .unwrap_or(16_666_666);
    let refresh = Refresh::fixed(Duration::from_nanos(refresh_ns));
    let seq = 0u64;

    // Use the SurfaceData passed by with_surfaces_surface_tree directly —
    // re-entering with_states from inside its callback deadlocks on the
    // surface state mutex (caused freeze on first toplevel map).
    let drain = |_surface: &WlSurface, states: &SurfaceData| {
        let feedbacks = std::mem::take(
            &mut states
                .cached_state
                .get::<PresentationFeedbackCachedState>()
                .current()
                .callbacks,
        );
        for feedback in feedbacks {
            if rendered {
                feedback.presented(output, timestamp, refresh, seq, wp_presentation_feedback::Kind::Vsync);
            } else {
                feedback.discarded();
            }
        }
    };

    let surfaces: Vec<WlSurface> = space
        .elements()
        .filter_map(|w| crate::window_ext::WindowExt::get_wl_surface(w))
        .collect();
    for s in &surfaces {
        smithay::desktop::utils::with_surfaces_surface_tree(s, drain);
    }
    for ls in layer_surfaces {
        if ls.alive() {
            smithay::desktop::utils::with_surfaces_surface_tree(ls.wl_surface(), drain);
        }
    }
}

/// Capture a window's surface content into an offscreen texture so the
/// close animation can keep rendering after the surface has been
/// destroyed / unmapped.
pub(super) fn capture_window_snapshot(
    renderer: &mut GlesRenderer,
    window: &smithay::desktop::Window,
    win_size: Size<i32, Logical>,
    output_scale: f64,
) -> Option<(GlesTexture, Size<i32, Physical>)> {
    let snap_w = (win_size.w as f64 * output_scale).round() as i32;
    let snap_h = (win_size.h as f64 * output_scale).round() as i32;
    // Tiny surfaces (< 16px) are usually transient bootstrap buffers from
    // Proton/Wine that resize themselves a frame later. Capturing them
    // racing against the client's realloc triggers GL_INVALID_VALUE.
    if snap_w < 16 || snap_h < 16 { return None; }

    let snap_size = Size::<i32, Physical>::from((snap_w, snap_h));
    let buf_size: Size<i32, BufferCoords> = Size::from((snap_w, snap_h));

    // Render the surface tree shifted by -geometry().loc so the texture
    // contains exactly the visible geometry box. Clients that draw CSD
    // shadow margins outside their geometry (Firefox/GTK) have a nonzero
    // offset — capturing at (0,0) shifts their content down-right and
    // crops it, which made the close animation visibly jump.
    let geo_loc = window.geometry().loc;
    let origin: Point<i32, Physical> = Point::from((
        -((geo_loc.x as f64 * output_scale).round() as i32),
        -((geo_loc.y as f64 * output_scale).round() as i32),
    ));
    let elements: Vec<WaylandSurfaceRenderElement<GlesRenderer>> =
        window.render_elements(renderer, origin, Scale::from(output_scale), 1.0);
    if elements.is_empty() { return None; }

    let mut tex = Offscreen::<GlesTexture>::create_buffer(renderer, Fourcc::Abgr8888, buf_size).ok()?;
    {
        let mut target = renderer.bind(&mut tex).ok()?;
        let mut frame = renderer.render(&mut target, snap_size, Transform::Normal).ok()?;
        frame.clear(Color32F::from([0.0, 0.0, 0.0, 0.0]), &[Rectangle::from_size(snap_size)]).ok()?;

        let scale = Scale::from(output_scale);
        for elem in &elements {
            let geo = elem.geometry(scale);
            let src = elem.src();
            let dst = Rectangle::<i32, Physical>::new(geo.loc, geo.size);
            if dst.size.w > 0 && dst.size.h > 0 {
                let _ = elem.draw(&mut frame, src, dst, &[dst], &[]);
            }
        }
        let _ = frame.finish();
    }

    Some((tex, snap_size))
}

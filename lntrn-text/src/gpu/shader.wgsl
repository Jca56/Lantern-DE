// Lantern text engine — glyph quad shader (Phase 0).
//
// Draws textured quads sampling a single-channel (R8) coverage atlas. Color is
// passed per-vertex in LINEAR space; the sRGB render target hardware-encodes on
// write, so we output premultiplied-alpha linear color and blend with
// (One, OneMinusSrcAlpha).

struct Viewport {
    size: vec2<f32>,
    // Atlas edge length in texels: quad UVs arrive in texel space (so the
    // atlas can grow without invalidating them) and normalize here.
    atlas_size: vec2<f32>,
};

@group(0) @binding(0) var<uniform> viewport: Viewport;
@group(0) @binding(1) var atlas_tex: texture_2d<f32>;
@group(0) @binding(2) var atlas_samp: sampler;

struct VsOut {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
};

@vertex
fn vs_main(
    @location(0) pos: vec2<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) color: vec4<f32>,
) -> VsOut {
    // Pixel coordinates (origin top-left, y down) → NDC.
    let ndc = vec2<f32>(
        pos.x / viewport.size.x * 2.0 - 1.0,
        1.0 - pos.y / viewport.size.y * 2.0,
    );
    var out: VsOut;
    out.clip_pos = vec4<f32>(ndc, 0.0, 1.0);
    out.uv = uv / viewport.atlas_size;
    out.color = color;
    return out;
}

// How much to fatten a glyph's coverage, from the ink's own lightness.
//
// Compositing is a lerp in *linear light* weighted by geometric coverage,
// which is physically right and perceptually wrong. A pixel half covered by
// black ink on a white ground lands at linear 0.5, and linear 0.5 encodes to
// sRGB 188 — it looks three quarters white, not half. At UI sizes most of a
// glyph *is* partly-covered edge, so dark text on a light ground dissolves.
//
// The shader cannot read the ground it is drawing onto, so the ink's own
// lightness stands in for it: dark ink is assumed to sit on something light.
// The correction only ever *thickens* — light text is left exactly as it
// was, because it is already the case that reads correctly and quietly
// thinning it would be a regression nobody asked for.
fn coverage_gamma(rgb: vec3<f32>) -> f32 {
    let lin = clamp(dot(rgb, vec3<f32>(0.2126, 0.7152, 0.0722)), 0.0, 1.0);
    // Compared in the space lightness is judged in, not in linear light.
    let lightness = pow(lin, 1.0 / 2.2);
    return mix(1.0 / 1.6, 1.0, lightness);
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    // Atlas texels are premultiplied: text = white × coverage (tinted by the
    // quad color), emoji = premultiplied sRGB pixels (quad color is white +
    // alpha). One multiply serves both; output stays premultiplied.
    let texel = textureSample(atlas_tex, atlas_samp, in.uv);
    let cov = pow(texel.a, coverage_gamma(in.color.rgb));
    // Applied as a *ratio* so the one expression still serves both kinds.
    // A coverage glyph has rgb == a, so this comes out as color × cov. A
    // color glyph is drawn with a white quad color, whose gamma is exactly
    // 1.0, so its ratio is 1.0 and its pixels pass through untouched.
    let boost = select(1.0, cov / texel.a, texel.a > 0.0);
    return vec4<f32>(
        in.color.rgb * in.color.a * texel.rgb * boost,
        in.color.a * cov,
    );
}

/// Shader constants and GLSL source code for window rendering effects.

/// Pixel shader for corner rounding.
/// Placed at each window corner (radius x radius square).
/// `corner` uniform: (0,0)=TL, (1,0)=TR, (0,1)=BL, (1,1)=BR
/// The sharp corner point is at the specified corner of the element.
pub const CORNER_SHADER_SRC: &str = r#"
precision mediump float;
varying vec2 v_coords;
uniform float alpha;
uniform vec2 size;
uniform float corner_radius;
uniform vec2 corner;

#if defined(DEBUG_FLAGS)
uniform float tint;
#endif

void main() {
    // Pixel position within this element
    vec2 pos = v_coords * size;

    // Flip coords so (0,0) is always the sharp corner point
    vec2 p = mix(pos, size - pos, corner);

    // Circle center is at (radius, radius) from the corner
    float dx = corner_radius - p.x;
    float dy = corner_radius - p.y;

    // Only apply rounding in the corner quadrant
    if (dx > 0.0 && dy > 0.0) {
        float dist = length(vec2(dx, dy)) - corner_radius;
        float mask = smoothstep(-0.5, 0.5, dist);
        if (mask < 0.01) discard;
        gl_FragColor = vec4(0.0, 0.0, 0.0, mask * alpha);
    } else {
        discard;
    }
}
"#;

/// Custom texture shader for rounded-corner alpha masking.
/// Applied to window textures rendered offscreen; clips corners via SDF.
/// Custom uniforms: `tex_size` (vec2, physical px), `corner_radius` (float, physical px)
pub const ROUNDED_TEX_SHADER_SRC: &str = r#"
//_DEFINES_

#if defined(EXTERNAL)
#extension GL_OES_EGL_image_external : require
#endif

precision mediump float;

#if defined(EXTERNAL)
uniform samplerExternalOES tex;
#else
uniform sampler2D tex;
#endif

uniform float alpha;
uniform vec2 tex_size;
uniform float corner_radius;
varying vec2 v_coords;

#if defined(DEBUG_FLAGS)
uniform float tint;
#endif

void main() {
    vec4 color = texture2D(tex, v_coords);

#if defined(NO_ALPHA)
    color = vec4(color.rgb, 1.0) * alpha;
#else
    color = color * alpha;
#endif

    // SDF rounded-rect mask
    vec2 pos = v_coords * tex_size;
    vec2 half_size = tex_size * 0.5;
    vec2 q = abs(pos - half_size) - half_size + vec2(corner_radius);
    float dist = length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - corner_radius;
    float mask = 1.0 - smoothstep(-0.5, 0.5, dist);

    // Premultiplied alpha: multiply all channels by mask
    color *= mask;

#if defined(DEBUG_FLAGS)
    if (tint == 1.0)
        color = vec4(0.0, 0.2, 0.0, 0.2) + color * 0.8;
#endif

    gl_FragColor = color;
}
"#;

// ── Hot corner glow ──────────────────────────────────────────────
pub const HOT_CORNER_GLOW_SIZE: i32 = 100; // logical pixels per side
pub const HOT_CORNER_GLOW_SIGMA: f32 = 40.0; // falloff softness (logical px)
pub const HOT_CORNER_GLOW_COLOR: [f32; 4] = [1.0, 0.75, 0.0, 0.6]; // amber accent

/// Pixel shader for hot corner feedback glow.
/// Renders a radial gradient emanating from a screen corner.
/// `corner` uniform: (0,0)=TL, (1,0)=TR, (0,1)=BL, (1,1)=BR
pub const HOT_CORNER_GLOW_SHADER_SRC: &str = r#"
precision mediump float;
varying vec2 v_coords;
uniform float alpha;
uniform vec2 size;
uniform vec2 corner;
uniform vec4 glow_color;
uniform float sigma;

#if defined(DEBUG_FLAGS)
uniform float tint;
#endif

void main() {
    vec2 pos = v_coords * size;
    vec2 origin = corner * size;
    float dist = length(pos - origin);
    float norm = dist / sigma;
    float glow = exp(-0.5 * norm * norm);
    if (glow < 0.004) discard;

    float a = glow_color.a * glow * alpha;
    gl_FragColor = vec4(glow_color.rgb * a, a);
}
"#;

// ── SSD window control icons ─────────────────────────────────────
/// Pixel shader for SSD titlebar icons (close X, maximize square, minimize dash).
/// `icon_type` uniform: 0.0=close(X), 1.0=maximize(□), 2.0=minimize(─)
/// `icon_color` uniform: [r, g, b, a] sRGB
/// Element size should be BTN_W × BAR_HEIGHT.
pub const SSD_ICON_SHADER_SRC: &str = r#"
precision mediump float;
varying vec2 v_coords;
uniform float alpha;
uniform vec2 size;
uniform float icon_type;
uniform vec4 icon_color;

#if defined(DEBUG_FLAGS)
uniform float tint;
#endif

void main() {
    vec2 pos = v_coords * size;
    vec2 center = size * 0.5;
    float icon_sz = min(size.x, size.y) * 0.35;
    float line_w = max(1.5, icon_sz * 0.15);

    float d = 1e10;

    if (icon_type < 0.5) {
        // Close: X shape — two diagonal lines
        vec2 p = pos - center;
        // Distance to line from (-half,-half) to (half,half)
        float half = icon_sz * 0.5;
        float d1 = abs(p.x - p.y) / 1.41421;
        float d2 = abs(p.x + p.y) / 1.41421;
        // Clamp to line segment length
        float len_check = max(abs(p.x), abs(p.y));
        d1 = len_check > half ? 1e10 : d1;
        d2 = len_check > half ? 1e10 : d2;
        d = min(d1, d2);
    } else if (icon_type < 1.5) {
        // Maximize: square outline
        vec2 p = pos - center;
        float half = icon_sz * 0.5;
        vec2 ap = abs(p);
        // SDF of a hollow rectangle
        float outer = max(ap.x - half, ap.y - half);
        float inner = max(ap.x - (half - line_w), ap.y - (half - line_w));
        d = max(outer, -inner);
        d = -d; // flip so positive = inside the outline
    } else {
        // Minimize: horizontal dash
        vec2 p = pos - center;
        float half = icon_sz * 0.5;
        d = max(abs(p.x) - half, abs(p.y) - line_w * 0.5);
        d = -d;
    }

    float mask;
    if (icon_type < 0.5) {
        mask = 1.0 - smoothstep(line_w * 0.5 - 0.5, line_w * 0.5 + 0.5, d);
    } else {
        mask = smoothstep(-0.5, 0.5, d);
    }

    if (mask < 0.01) discard;
    float a = icon_color.a * mask * alpha;
    gl_FragColor = vec4(icon_color.rgb * a, a);
}
"#;

// ── Window shadow / glow (disabled until settings page) ──────────
#[allow(dead_code)]
pub const SHADOW_SPREAD: i32 = 18; // logical pixels
#[allow(dead_code)]
pub const SHADOW_SIGMA: f32 = 10.0; // blur softness (physical pixels, scaled at use)
#[allow(dead_code)]
pub const SHADOW_COLOR: [f32; 4] = [0.0, 0.0, 0.0, 0.45];
#[allow(dead_code)]
pub const GLOW_SPREAD: i32 = 22;
#[allow(dead_code)]
pub const GLOW_SIGMA: f32 = 8.0;
#[allow(dead_code)]
pub const GLOW_COLOR: [f32; 4] = [1.0, 0.75, 0.0, 0.45]; // amber accent

/// Pixel shader for window shadow / focused glow.
/// Covers window + spread area. Uses SDF of a rounded rectangle
/// with gaussian-inspired falloff for soft edges.
pub const SHADOW_SHADER_SRC: &str = r#"
precision mediump float;
varying vec2 v_coords;
uniform float alpha;
uniform vec2 size;
uniform vec2 window_size;
uniform float sigma;
uniform float corner_radius;
uniform vec4 shadow_color;

#if defined(DEBUG_FLAGS)
uniform float tint;
#endif

// Signed distance from a rounded rectangle centered at origin
float roundedBoxSDF(vec2 p, vec2 half_size, float radius) {
    vec2 q = abs(p) - half_size + vec2(radius);
    return length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - radius;
}

void main() {
    vec2 pos = v_coords * size;
    vec2 center = size * 0.5;
    vec2 half_win = window_size * 0.5;

    float dist = roundedBoxSDF(pos - center, half_win, corner_radius);

    // Only draw outside the window; inside is covered by the window element.
    if (dist <= 0.0) discard;

    float norm = dist / sigma;
    float shadow = exp(-0.5 * norm * norm);
    if (shadow < 0.004) discard;

    // Premultiplied alpha output (Smithay uses GL_ONE, GL_ONE_MINUS_SRC_ALPHA)
    float a = shadow_color.a * shadow * alpha;
    gl_FragColor = vec4(shadow_color.rgb * a, a);
}
"#;

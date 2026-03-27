pub const SHADER_2D: &str = r#"
struct Globals {
    screen_size: vec2<f32>,
    _pad: vec2<f32>,
};

@group(0) @binding(0) var<uniform> globals: Globals;

struct InstanceInput {
    @location(0) bounds: vec4<f32>,
    @location(1) color: vec4<f32>,
    @location(2) params: vec4<f32>,
    @location(3) color_b: vec4<f32>,
};

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) bounds: vec4<f32>,
    @location(3) params: vec4<f32>,
    @location(4) local_px: vec2<f32>,
    @location(5) color_b: vec4<f32>,
};

const SHAPE_RECT: f32 = 0.0;
const SHAPE_CIRCLE: f32 = 1.0;
const SHAPE_LINE: f32 = 2.0;
const SHAPE_RING: f32 = 3.0;
const SHAPE_GRADIENT_LINEAR: f32 = 4.0;
const SHAPE_GRADIENT_RADIAL: f32 = 5.0;
const SHAPE_RECT_STROKE: f32 = 6.0;
const SHAPE_RECT_4CORNER: f32 = 7.0;
const SHAPE_TRIANGLE: f32 = 8.0;
const SHAPE_SHADOW: f32 = 9.0;
const SHAPE_ARC: f32 = 10.0;
const SHAPE_DASHED_LINE: f32 = 11.0;

@vertex
fn vs_main(@builtin(vertex_index) vi: u32, instance: InstanceInput) -> VertexOutput {
    let uv = vec2<f32>(f32(vi & 1u), f32((vi >> 1u) & 1u));

    var quad_pos: vec2<f32>;
    var quad_size: vec2<f32>;

    if instance.params.w == SHAPE_LINE || instance.params.w == SHAPE_DASHED_LINE {
        let p1 = instance.bounds.xy;
        let p2 = instance.params.yz;
        let half_w = instance.params.x * 0.5 + 1.0;
        let min_pt = min(p1, p2) - vec2<f32>(half_w);
        let max_pt = max(p1, p2) + vec2<f32>(half_w);
        quad_pos = min_pt;
        quad_size = max_pt - min_pt;
    } else if instance.params.w == SHAPE_TRIANGLE {
        // Triangle: bounds.xy = p1, bounds.zw = p2, params.xy = p3
        let p1 = instance.bounds.xy;
        let p2 = instance.bounds.zw;
        let p3 = instance.params.xy;
        let min_pt = min(min(p1, p2), p3) - vec2<f32>(2.0);
        let max_pt = max(max(p1, p2), p3) + vec2<f32>(2.0);
        quad_pos = min_pt;
        quad_size = max_pt - min_pt;
    } else {
        quad_pos = instance.bounds.xy;
        quad_size = instance.bounds.zw;
    }

    let px = quad_pos + uv * quad_size;
    let ndc = vec2<f32>(
        px.x / globals.screen_size.x * 2.0 - 1.0,
        1.0 - px.y / globals.screen_size.y * 2.0,
    );

    var out: VertexOutput;
    out.position = vec4<f32>(ndc, 0.0, 1.0);
    out.uv = uv;
    out.color = instance.color;
    out.bounds = instance.bounds;
    out.params = instance.params;
    out.local_px = px;
    out.color_b = instance.color_b;
    return out;
}

// ── SDF helpers ─────────────────────────────────────────────────────────────

fn sdf_rounded_rect(p: vec2<f32>, center: vec2<f32>, half_size: vec2<f32>, radius: f32) -> f32 {
    let d = abs(p - center) - half_size + vec2<f32>(radius);
    return length(max(d, vec2<f32>(0.0))) + min(max(d.x, d.y), 0.0) - radius;
}

// Per-corner radii: selects the correct radius based on which quadrant the pixel is in.
fn sdf_rounded_rect_4(p: vec2<f32>, center: vec2<f32>, half_size: vec2<f32>,
                       r_tl: f32, r_tr: f32, r_bl: f32, r_br: f32) -> f32 {
    let rel = p - center;
    var r: f32;
    if rel.x < 0.0 {
        if rel.y < 0.0 { r = r_tl; } else { r = r_bl; }
    } else {
        if rel.y < 0.0 { r = r_tr; } else { r = r_br; }
    }
    let d = abs(rel) - half_size + vec2<f32>(r);
    return length(max(d, vec2<f32>(0.0))) + min(max(d.x, d.y), 0.0) - r;
}

fn sdf_line(p: vec2<f32>, a: vec2<f32>, b: vec2<f32>) -> f32 {
    let pa = p - a;
    let ba = b - a;
    let h = clamp(dot(pa, ba) / dot(ba, ba), 0.0, 1.0);
    return length(pa - ba * h);
}

// Signed distance to a triangle (2D). Negative = inside.
fn sdf_triangle(p: vec2<f32>, a: vec2<f32>, b: vec2<f32>, c: vec2<f32>) -> f32 {
    let e0 = b - a; let e1 = c - b; let e2 = a - c;
    let v0 = p - a; let v1 = p - b; let v2 = p - c;

    let pq0 = v0 - e0 * clamp(dot(v0, e0) / dot(e0, e0), 0.0, 1.0);
    let pq1 = v1 - e1 * clamp(dot(v1, e1) / dot(e1, e1), 0.0, 1.0);
    let pq2 = v2 - e2 * clamp(dot(v2, e2) / dot(e2, e2), 0.0, 1.0);

    let s = sign(e0.x * e2.y - e0.y * e2.x);
    let d0 = vec2<f32>(dot(pq0, pq0), s * (v0.x * e0.y - v0.y * e0.x));
    let d1 = vec2<f32>(dot(pq1, pq1), s * (v1.x * e1.y - v1.y * e1.x));
    let d2 = vec2<f32>(dot(pq2, pq2), s * (v2.x * e2.y - v2.y * e2.x));

    let d = min(min(d0, d1), d2);
    return -sqrt(d.x) * sign(d.y);
}

// Approximate gaussian blur for shadows. Uses a polynomial approximation
// of erf() to avoid expensive exp() calls.
fn shadow_mask(dist: f32, sigma: f32) -> f32 {
    let x = dist / (sigma + 0.001);
    // Approximation of 1 - erf(x/sqrt(2)) * 0.5 for the outer region
    return 1.0 - smoothstep(-3.0 * sigma, 3.0 * sigma, dist);
}

// ── Fragment shader ─────────────────────────────────────────────────────────

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    var color = in.color;
    var mask: f32 = 1.0;

    if in.params.w == SHAPE_RECT {
        let radius = in.params.x;
        if radius < 0.5 {
            mask = 1.0;
        } else {
            let center = in.bounds.xy + in.bounds.zw * 0.5;
            let half_size = in.bounds.zw * 0.5;
            let dist = sdf_rounded_rect(in.local_px, center, half_size, radius);
            mask = 1.0 - smoothstep(-0.5, 0.5, dist);
        }

    } else if in.params.w == SHAPE_CIRCLE {
        let center = in.bounds.xy + in.bounds.zw * 0.5;
        let radius = min(in.bounds.z, in.bounds.w) * 0.5;
        let dist = length(in.local_px - center) - radius;
        mask = 1.0 - smoothstep(-1.0, 1.0, dist);

    } else if in.params.w == SHAPE_LINE {
        let dist = sdf_line(in.local_px, in.bounds.xy, in.params.yz) - in.params.x * 0.5;
        mask = 1.0 - smoothstep(-1.0, 1.0, dist);

    } else if in.params.w == SHAPE_RING {
        let center = in.bounds.xy + in.bounds.zw * 0.5;
        let radius = in.params.y;
        let dist = abs(length(in.local_px - center) - radius) - in.params.x * 0.5;
        mask = 1.0 - smoothstep(-1.0, 1.0, dist);

    } else if in.params.w == SHAPE_GRADIENT_LINEAR {
        let radius = in.params.x;
        let center = in.bounds.xy + in.bounds.zw * 0.5;
        let half_size = in.bounds.zw * 0.5;
        let dist = sdf_rounded_rect(in.local_px, center, half_size, radius);
        mask = 1.0 - smoothstep(-1.0, 1.0, dist);
        let angle = in.params.y;
        let dir = vec2<f32>(cos(angle), sin(angle));
        let rel = (in.local_px - in.bounds.xy) / in.bounds.zw - 0.5;
        let t = clamp(dot(rel, dir) + 0.5, 0.0, 1.0);
        color = mix(in.color, in.color_b, vec4<f32>(t));

    } else if in.params.w == SHAPE_GRADIENT_RADIAL {
        let radius = in.params.x;
        let center = in.bounds.xy + in.bounds.zw * 0.5;
        let half_size = in.bounds.zw * 0.5;
        let dist = sdf_rounded_rect(in.local_px, center, half_size, radius);
        mask = 1.0 - smoothstep(-1.0, 1.0, dist);
        let max_dist = length(in.bounds.zw * 0.5);
        let d = length(in.local_px - center) / max_dist;
        let t = clamp(d, 0.0, 1.0);
        color = mix(in.color, in.color_b, vec4<f32>(t));

    } else if in.params.w == SHAPE_RECT_STROKE {
        // Proper rounded rect outline via SDF
        // params: [corner_radius, stroke_width, 0, shape_id]
        let radius = in.params.x;
        let stroke_w = in.params.y;
        let center = in.bounds.xy + in.bounds.zw * 0.5;
        let half_size = in.bounds.zw * 0.5;
        let dist = sdf_rounded_rect(in.local_px, center, half_size, radius);
        // Hollow: abs(dist) gives distance to the boundary ring
        let ring_dist = abs(dist + stroke_w * 0.5) - stroke_w * 0.5;
        mask = 1.0 - smoothstep(-1.0, 1.0, ring_dist);

    } else if in.params.w == SHAPE_RECT_4CORNER {
        // Per-corner radii rect
        // params: [tl, tr, 0, shape_id], color_b: [bl, br, 0, 0]
        let center = in.bounds.xy + in.bounds.zw * 0.5;
        let half_size = in.bounds.zw * 0.5;
        let dist = sdf_rounded_rect_4(
            in.local_px, center, half_size,
            in.params.x, in.params.y, in.color_b.x, in.color_b.y
        );
        mask = 1.0 - smoothstep(-1.0, 1.0, dist);
        // color_b is used for radii, so use the main color
        color = in.color;

    } else if in.params.w == SHAPE_TRIANGLE {
        // bounds.xy = p1, bounds.zw = p2, params.xy = p3
        let dist = sdf_triangle(in.local_px, in.bounds.xy, in.bounds.zw, in.params.xy);
        mask = 1.0 - smoothstep(-1.0, 1.0, dist);

    } else if in.params.w == SHAPE_SHADOW {
        // Soft shadow for rounded rects
        // params: [corner_radius, blur_sigma, 0, shape_id]
        let radius = in.params.x;
        let sigma = in.params.y;
        let center = in.bounds.xy + in.bounds.zw * 0.5;
        let half_size = in.bounds.zw * 0.5;
        let dist = sdf_rounded_rect(in.local_px, center, half_size, radius);
        mask = shadow_mask(dist, sigma);

    } else if in.params.w == SHAPE_ARC {
        // Arc / pie slice
        // bounds.xy = center, bounds.zw = [outer_radius*2, outer_radius*2]
        // params: [stroke_width, start_angle, sweep_angle, shape_id]
        // If stroke_width == 0, draws a filled pie. Otherwise draws an arc stroke.
        let center = in.bounds.xy + in.bounds.zw * 0.5;
        let outer_r = min(in.bounds.z, in.bounds.w) * 0.5;
        let stroke_w = in.params.x;
        let start_a = in.params.y;
        let sweep = in.params.z;
        // color_b.x = inner_radius (for donut arcs), 0 = full pie
        let inner_r = in.color_b.x;

        let rel = in.local_px - center;
        let d = length(rel);
        let angle = atan2(rel.y, rel.x);

        // Normalize angle relative to start, wrap to [0, 2*PI)
        var a = angle - start_a;
        a = a - floor(a / 6.2831853) * 6.2831853;
        // Check if within sweep
        let in_sweep = a >= 0.0 && a <= sweep;

        if stroke_w > 0.0 {
            // Arc stroke mode
            let ring_dist = abs(d - (outer_r + inner_r) * 0.5) - stroke_w * 0.5;
            let radial_mask = 1.0 - smoothstep(-1.0, 1.0, ring_dist);
            if in_sweep {
                mask = radial_mask;
            } else {
                mask = 0.0;
            }
        } else {
            // Filled pie mode
            let outer_dist = d - outer_r;
            let outer_mask = 1.0 - smoothstep(-1.0, 1.0, outer_dist);
            let inner_mask = 1.0 - smoothstep(-1.0, 1.0, -(d - inner_r));
            if in_sweep {
                mask = outer_mask * inner_mask;
            } else {
                mask = 0.0;
            }
        }
        color = in.color;

    } else if in.params.w == SHAPE_DASHED_LINE {
        // Dashed line
        // bounds.xy = p1, params.yz = p2, params.x = width
        // color_b.x = dash_length, color_b.y = gap_length
        let p1 = in.bounds.xy;
        let p2 = in.params.yz;
        let dist = sdf_line(in.local_px, p1, p2) - in.params.x * 0.5;
        let line_mask = 1.0 - smoothstep(-1.0, 1.0, dist);

        // Project pixel onto line to get parametric position
        let line_dir = p2 - p1;
        let line_len = length(line_dir);
        let t = dot(in.local_px - p1, line_dir) / (line_len * line_len + 0.001);
        let along = t * line_len;
        let dash_len = in.color_b.x;
        let gap_len = in.color_b.y;
        let period = dash_len + gap_len;
        let phase = along - floor(along / period) * period;
        let dash_mask = 1.0 - smoothstep(dash_len - 1.0, dash_len, phase);

        mask = line_mask * dash_mask;
        color = in.color;
    }

    let alpha = color.a * mask;
    if alpha < 0.004 {
        discard;
    }

    return vec4<f32>(color.rgb * alpha, alpha);
}
"#;

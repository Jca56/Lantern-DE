pub const SHADER_2D: &str = r#"
// Lantern 2D Renderer Shader
// Renders rects, circles, and lines via instanced quads with SDF masking.

struct Globals {
    screen_size: vec2<f32>,
    _pad: vec2<f32>,
};

@group(0) @binding(0) var<uniform> globals: Globals;

// Per-instance data packed into vertex buffer
struct InstanceInput {
    // Quad bounds in pixels: (x, y, width, height)
    @location(0) bounds: vec4<f32>,
    // RGBA color
    @location(1) color: vec4<f32>,
    // Shape params:
    //   For rect:   (corner_radius, border_width, 0, SHAPE_RECT=0)
    //   For circle: (0, 0, 0, SHAPE_CIRCLE=1)
    //   For line:   (line_width, x2, y2, SHAPE_LINE=2)
    @location(2) params: vec4<f32>,
};

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) bounds: vec4<f32>,
    @location(3) params: vec4<f32>,
    // Pixel position within the quad
    @location(4) local_px: vec2<f32>,
};

const SHAPE_RECT: f32 = 0.0;
const SHAPE_CIRCLE: f32 = 1.0;
const SHAPE_LINE: f32 = 2.0;
const SHAPE_RING: f32 = 3.0;

@vertex
fn vs_main(
    @builtin(vertex_index) vi: u32,
    instance: InstanceInput,
) -> VertexOutput {
    let uv = vec2<f32>(
        f32(vi & 1u),
        f32((vi >> 1u) & 1u),
    );

    var quad_pos: vec2<f32>;
    var quad_size: vec2<f32>;

    let shape = instance.params.w;

    if shape == SHAPE_LINE {
        // For lines: bounds.xy = start point, params.yz = end point
        let p1 = instance.bounds.xy;
        let p2 = instance.params.yz;
        let half_w = instance.params.x * 0.5 + 1.0; // +1 for AA

        let d = p2 - p1;
        let len = length(d);
        let perp = vec2<f32>(-d.y, d.x) / max(len, 0.001);

        // Expand quad to encompass the line with padding
        let min_pt = min(p1, p2) - vec2<f32>(half_w);
        let max_pt = max(p1, p2) + vec2<f32>(half_w);
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
    return out;
}

// SDF for a rounded rectangle
fn sdf_rounded_rect(p: vec2<f32>, center: vec2<f32>, half_size: vec2<f32>, radius: f32) -> f32 {
    let d = abs(p - center) - half_size + vec2<f32>(radius);
    return length(max(d, vec2<f32>(0.0))) + min(max(d.x, d.y), 0.0) - radius;
}

// SDF for a line segment
fn sdf_line(p: vec2<f32>, a: vec2<f32>, b: vec2<f32>) -> f32 {
    let pa = p - a;
    let ba = b - a;
    let h = clamp(dot(pa, ba) / dot(ba, ba), 0.0, 1.0);
    return length(pa - ba * h);
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let shape = in.params.w;
    var alpha = in.color.a;

    if shape == SHAPE_RECT {
        let radius = in.params.x;
        let center = in.bounds.xy + in.bounds.zw * 0.5;
        let half_size = in.bounds.zw * 0.5;
        let dist = sdf_rounded_rect(in.local_px, center, half_size, radius);
        // Anti-aliased edge
        alpha *= 1.0 - smoothstep(-1.0, 1.0, dist);
    } else if shape == SHAPE_CIRCLE {
        let center = in.bounds.xy + in.bounds.zw * 0.5;
        let radius = min(in.bounds.z, in.bounds.w) * 0.5;
        let dist = length(in.local_px - center) - radius;
        alpha *= 1.0 - smoothstep(-1.0, 1.0, dist);
    } else if shape == SHAPE_LINE {
        let p1 = in.bounds.xy;
        let p2 = in.params.yz;
        let half_w = in.params.x * 0.5;
        let dist = sdf_line(in.local_px, p1, p2) - half_w;
        alpha *= 1.0 - smoothstep(-1.0, 1.0, dist);
    } else if shape == SHAPE_RING {
        let center = in.bounds.xy + in.bounds.zw * 0.5;
        let radius = min(in.bounds.z, in.bounds.w) * 0.5;
        let stroke_w = in.params.x;
        let dist = abs(length(in.local_px - center) - radius) - stroke_w * 0.5;
        alpha *= 1.0 - smoothstep(-1.0, 1.0, dist);
    }

    if alpha < 0.004 {
        discard;
    }

    return vec4<f32>(in.color.rgb * alpha, alpha);
}
"#;

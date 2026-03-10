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

@vertex
fn vs_main(@builtin(vertex_index) vi: u32, instance: InstanceInput) -> VertexOutput {
    let uv = vec2<f32>(f32(vi & 1u), f32((vi >> 1u) & 1u));

    var quad_pos: vec2<f32>;
    var quad_size: vec2<f32>;

    if instance.params.w == SHAPE_LINE {
        let p1 = instance.bounds.xy;
        let p2 = instance.params.yz;
        let half_w = instance.params.x * 0.5 + 1.0;
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
    out.color_b = instance.color_b;
    return out;
}

fn sdf_rounded_rect(p: vec2<f32>, center: vec2<f32>, half_size: vec2<f32>, radius: f32) -> f32 {
    let d = abs(p - center) - half_size + vec2<f32>(radius);
    return length(max(d, vec2<f32>(0.0))) + min(max(d.x, d.y), 0.0) - radius;
}

fn sdf_line(p: vec2<f32>, a: vec2<f32>, b: vec2<f32>) -> f32 {
    let pa = p - a;
    let ba = b - a;
    let h = clamp(dot(pa, ba) / dot(ba, ba), 0.0, 1.0);
    return length(pa - ba * h);
}

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
            mask = 1.0 - smoothstep(-1.0, 1.0, dist);
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
    }

    let alpha = color.a * mask;
    if alpha < 0.004 {
        discard;
    }

    return vec4<f32>(color.rgb * alpha, alpha);
}
"#;
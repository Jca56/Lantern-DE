pub const SHADER_TEX: &str = r#"
struct Globals {
    screen_size: vec2<f32>,
    _pad: vec2<f32>,
};

@group(0) @binding(0) var<uniform> globals: Globals;
@group(1) @binding(0) var tex: texture_2d<f32>;
@group(1) @binding(1) var tex_sampler: sampler;

struct InstanceInput {
    @location(0) bounds: vec4<f32>,
    @location(1) uv_rect: vec4<f32>,
    @location(2) tint: vec4<f32>,
    // Bits 0-1: quarter turns clockwise; bit 2: then mirrored left to right.
    @location(3) orient: u32,
};

struct VOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) tint: vec4<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vi: u32, inst: InstanceInput) -> VOut {
    let corner = vec2<f32>(f32(vi & 1u), f32((vi >> 1u) & 1u));
    let px = inst.bounds.xy + corner * inst.bounds.zw;
    let ndc = vec2<f32>(
        px.x / globals.screen_size.x * 2.0 - 1.0,
        1.0 - px.y / globals.screen_size.y * 2.0,
    );
    // The texel corner this screen corner shows: undo the mirror, then the turn.
    var t = corner;
    if ((inst.orient & 4u) != 0u) {
        t.x = 1.0 - t.x;
    }
    switch (inst.orient & 3u) {
        case 1u: { t = vec2<f32>(t.y, 1.0 - t.x); }
        case 2u: { t = vec2<f32>(1.0 - t.x, 1.0 - t.y); }
        case 3u: { t = vec2<f32>(1.0 - t.y, t.x); }
        default: {}
    }
    let uv = inst.uv_rect.xy + t * (inst.uv_rect.zw - inst.uv_rect.xy);

    var out: VOut;
    out.position = vec4<f32>(ndc, 0.0, 1.0);
    out.uv = uv;
    out.tint = inst.tint;
    return out;
}

@fragment
fn fs_main(in: VOut) -> @location(0) vec4<f32> {
    let texel = textureSample(tex, tex_sampler, in.uv);
    let color = texel * in.tint;
    // Premultiply for correct blending
    return vec4<f32>(color.rgb * color.a, color.a);
}
"#;

#[cfg(test)]
mod tests {
    use wgpu::naga;

    /// The shader only compiles when an app starts; a WGSL slip should fail
    /// here instead of taking every app that draws textures down with it.
    #[test]
    fn shader_validates() {
        let module = naga::front::wgsl::parse_str(super::SHADER_TEX).expect("WGSL parses");
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::empty(),
        )
        .validate(&module)
        .expect("WGSL validates");
    }
}

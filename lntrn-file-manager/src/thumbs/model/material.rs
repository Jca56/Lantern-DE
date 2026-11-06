//! Materials as the thumbnail shades them: the base colour (factor and
//! picture; vertex colours come with the geometry), whether it is cut out
//! or see-through, and roughly how metallic and rough. Normal, occlusion
//! and metal/roughness pictures are not read: at icon size they would cost
//! a decode each for nothing a viewer could see.

use std::io::Cursor;

use serde_json::Value;

use super::gltf::{numbers, number, uint, Gltf};
use super::math::srgb_to_linear;
use super::scene::Prim;

/// Base colour pictures are shrunk to fit this square: the drawing is at
/// most a few hundred pixels across, so more would be averaged away.
const TEX_DIM: u32 = 512;
/// Distinct pictures decoded for one model; materials past them show their
/// base colour factor alone.
const MAX_TEXTURES: usize = 32;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Alpha {
    Opaque,
    /// Cut out where the alpha is below this.
    Mask(f32),
    Blend,
}

/// A material's base colour picture.
pub(super) struct TexRef {
    /// Index into the file's textures.
    pub texture: usize,
    /// Which TEXCOORD_n it reads.
    pub set: u32,
    /// KHR_texture_transform as `u' = a·u + b·v + c`, `v' = d·u + e·v + f`.
    pub transform: Option<[f32; 6]>,
}

pub(super) struct Material {
    /// Linear RGBA.
    pub base: [f32; 4],
    pub texture: Option<TexRef>,
    pub metallic: f32,
    pub roughness: f32,
    /// Linear glow, strength applied.
    pub emissive: [f32; 3],
    pub alpha: Alpha,
    /// KHR_materials_unlit: the base colour as it is, no light.
    pub unlit: bool,
}

impl Default for Material {
    /// What a primitive without a material is drawn with. Not the spec's
    /// default (white, fully metallic): a light clay reads better as a
    /// shape.
    fn default() -> Self {
        Material {
            base: [0.62, 0.62, 0.64, 1.0],
            texture: None,
            metallic: 0.0,
            roughness: 0.6,
            emissive: [0.0; 3],
            alpha: Alpha::Opaque,
            unlit: false,
        }
    }
}

/// Every material in the file, in its order.
pub(super) fn parse_all(json: &Value) -> Vec<Material> {
    json.get("materials")
        .and_then(Value::as_array)
        .map(|ms| ms.iter().map(parse).collect())
        .unwrap_or_default()
}

fn parse(m: &Value) -> Material {
    let ext = |name: &str| m.get("extensions").and_then(|e| e.get(name));
    let pbr = m.get("pbrMetallicRoughness");
    // Older files (and some exporters still) write specular-glossiness: its
    // diffuse is the base colour.
    let sg = ext("KHR_materials_pbrSpecularGlossiness");
    let (base, texture) = match (pbr, sg) {
        (_, Some(sg)) => (numbers::<4>(sg, "diffuseFactor"), sg.get("diffuseTexture")),
        (Some(p), None) => (numbers::<4>(p, "baseColorFactor"), p.get("baseColorTexture")),
        (None, None) => (None, None),
    };
    let (mut metallic, roughness) = match (pbr, sg) {
        (_, Some(sg)) => (0.0, 1.0 - number(sg, "glossinessFactor").unwrap_or(1.0)),
        (Some(p), None) => (
            number(p, "metallicFactor").unwrap_or(1.0),
            number(p, "roughnessFactor").unwrap_or(1.0),
        ),
        (None, None) => (1.0, 1.0),
    };
    // With a metal/roughness picture the factor only scales it, and the
    // picture is usually metal in places: halve it rather than decode it.
    if pbr.is_some_and(|p| p.get("metallicRoughnessTexture").is_some()) {
        metallic *= 0.5;
    }
    // An emissive picture masks where the glow is; without decoding it the
    // factor alone would light the whole surface, so it is left out.
    let emissive = if m.get("emissiveTexture").is_some() {
        [0.0; 3]
    } else {
        let strength = ext("KHR_materials_emissive_strength")
            .and_then(|e| number(e, "emissiveStrength"))
            .unwrap_or(1.0);
        numbers::<3>(m, "emissiveFactor").unwrap_or([0.0; 3]).map(|c| c * strength)
    };
    let alpha = match m.get("alphaMode").and_then(Value::as_str) {
        Some("MASK") => Alpha::Mask(number(m, "alphaCutoff").unwrap_or(0.5)),
        Some("BLEND") => Alpha::Blend,
        _ => Alpha::Opaque,
    };
    Material {
        base: base.unwrap_or([1.0; 4]),
        texture: texture.and_then(tex_ref),
        metallic: metallic.clamp(0.0, 1.0),
        roughness: roughness.clamp(0.0, 1.0),
        emissive,
        alpha,
        unlit: ext("KHR_materials_unlit").is_some(),
    }
}

fn tex_ref(t: &Value) -> Option<TexRef> {
    let tt = t.get("extensions").and_then(|e| e.get("KHR_texture_transform"));
    let set = tt
        .and_then(|tt| uint(tt, "texCoord"))
        .or_else(|| uint(t, "texCoord"))
        .unwrap_or(0) as u32;
    let transform = tt.map(|tt| {
        let [ox, oy] = numbers::<2>(tt, "offset").unwrap_or([0.0, 0.0]);
        let [sx, sy] = numbers::<2>(tt, "scale").unwrap_or([1.0, 1.0]);
        let r = number(tt, "rotation").unwrap_or(0.0);
        let (sin, cos) = r.sin_cos();
        // Translation × rotation × scale, as the extension defines it.
        [cos * sx, sin * sy, ox, -sin * sx, cos * sy, oy]
    });
    Some(TexRef { texture: uint(t, "index")?, set, transform })
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Wrap {
    Repeat,
    Clamp,
    Mirror,
}

/// A decoded base colour picture, shrunk to [`TEX_DIM`].
pub(super) struct Texture {
    w: usize,
    h: usize,
    /// sRGB RGBA, straight alpha.
    rgba: Vec<u8>,
    wrap: [Wrap; 2],
}

impl Texture {
    /// The picture at `uv` (bilinear, wrapped as its sampler says): linear
    /// RGB, straight alpha.
    pub fn sample(&self, uv: [f32; 2]) -> [f32; 4] {
        let x = uv[0] * self.w as f32 - 0.5;
        let y = uv[1] * self.h as f32 - 0.5;
        let (fx, fy) = (x.floor(), y.floor());
        let (tx, ty) = (x - fx, y - fy);
        if !(tx.is_finite() && ty.is_finite()) {
            return self.texel(0, 0);
        }
        let (x0, y0) = (fx as i64, fy as i64);
        let xs = [wrap(x0, self.w, self.wrap[0]), wrap(x0.saturating_add(1), self.w, self.wrap[0])];
        let ys = [wrap(y0, self.h, self.wrap[1]), wrap(y0.saturating_add(1), self.h, self.wrap[1])];
        let lerp4 = |a: [f32; 4], b: [f32; 4], t: f32| std::array::from_fn(|i| a[i] + (b[i] - a[i]) * t);
        let top = lerp4(self.texel(xs[0], ys[0]), self.texel(xs[1], ys[0]), tx);
        let bottom = lerp4(self.texel(xs[0], ys[1]), self.texel(xs[1], ys[1]), tx);
        lerp4(top, bottom, ty)
    }

    fn texel(&self, x: usize, y: usize) -> [f32; 4] {
        let i = (y * self.w + x) * 4;
        let p = &self.rgba[i..i + 4];
        [
            srgb_to_linear(p[0]),
            srgb_to_linear(p[1]),
            srgb_to_linear(p[2]),
            f32::from(p[3]) / 255.0,
        ]
    }
}

fn wrap(i: i64, n: usize, mode: Wrap) -> usize {
    let n = n as i64;
    let i = match mode {
        Wrap::Repeat => i.rem_euclid(n),
        Wrap::Clamp => i.clamp(0, n - 1),
        Wrap::Mirror => {
            let m = i.rem_euclid(2 * n);
            if m >= n { 2 * n - 1 - m } else { m }
        }
    };
    i as usize
}

/// The pictures the drawn primitives' materials use, decoded, indexed like
/// the file's textures. A picture that won't decode (KTX2, corrupt, too
/// big) leaves its material with the base colour factor.
pub(super) fn decode_used(g: &mut Gltf, materials: &[Material], prims: &[Prim]) -> Vec<Option<Texture>> {
    let count = g.json.get("textures").and_then(Value::as_array).map_or(0, Vec::len);
    let mut textures: Vec<Option<Texture>> = std::iter::repeat_with(|| None).take(count).collect();
    let mut tried = vec![false; count];
    let mut decoded = 0;
    for prim in prims.iter().filter(|p| !p.uvs.is_empty()) {
        let Some(t) = materials[prim.material].texture.as_ref() else {
            continue;
        };
        if t.texture >= count || tried[t.texture] || decoded >= MAX_TEXTURES {
            continue;
        }
        tried[t.texture] = true;
        textures[t.texture] = decode(g, t.texture);
        decoded += 1;
    }
    textures
}

fn decode(g: &mut Gltf, index: usize) -> Option<Texture> {
    let t = g.json.get("textures")?.get(index)?;
    // A WebP picture through EXT_texture_webp first; plain `source` is its
    // fallback (or the only one).
    let source = t
        .pointer("/extensions/EXT_texture_webp/source")
        .and_then(Value::as_u64)
        .map(|s| s as usize)
        .or_else(|| uint(t, "source"))?;
    let sampler = uint(t, "sampler").and_then(|s| g.json.get("samplers")?.get(s));
    let wrap_of = |key: &str| match sampler.and_then(|s| uint(s, key)) {
        Some(33071) => Wrap::Clamp,
        Some(33648) => Wrap::Mirror,
        _ => Wrap::Repeat,
    };
    let wrap = [wrap_of("wrapS"), wrap_of("wrapT")];
    let bytes = g.image(source)?;
    let reader = image::ImageReader::new(Cursor::new(bytes)).with_guessed_format().ok()?;
    // Textures are used as stored: an EXIF orientation doesn't turn them.
    let (img, _) = crate::thumbs::decode_unrotated(
        reader,
        crate::thumbs::MAX_DECODE_DIM,
        crate::thumbs::MAX_DECODE_BYTES,
    )?;
    // UVs are fractions of the picture, so any shrink keeps them right.
    let img = if img.width() > TEX_DIM || img.height() > TEX_DIM {
        img.thumbnail(TEX_DIM, TEX_DIM)
    } else {
        img
    };
    let rgba = img.to_rgba8();
    let (w, h) = (rgba.width() as usize, rgba.height() as usize);
    (w > 0 && h > 0).then(|| Texture { w, h, rgba: rgba.into_raw(), wrap })
}

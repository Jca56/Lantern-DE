//! Drawing the model: a camera from the front right, a little above (the
//! angle of Prism's own "iso" view), framing the model tight; triangles
//! rasterized with a depth buffer at several samples a pixel; soft studio
//! light; clear everywhere the model is not.

use image::RgbaImage;

use super::material::{Alpha, Material, Texture};
use super::math::{self, add, cross, dot, lerp, mul, normalize, scale, sub, Mat4, V3};
use super::scene::{Model, Prim};

/// The camera turns this far round from the front toward the right, and
/// rises this far above level (degrees), as Prism's "iso" view does.
const YAW: f32 = 35.0;
const PITCH: f32 = 25.0;
/// Field of view (degrees): narrow, a mild product-shot perspective.
const FOV: f32 = 30.0;
/// Clear pixels kept round the model, so its edges aren't cut.
const PAD: f32 = 2.0;

/// The light, toward each lamp in the camera's frame (x right, y up, z
/// toward the camera), and its colour: from upper left (the key), from the
/// right (a cool fill), from behind (a rim that lifts the silhouette off a
/// dark background); then a sky-over-ground ambient.
const KEY: (V3, V3) = ([-0.55, 0.75, 0.45], [1.25, 1.2, 1.12]);
const FILL: (V3, V3) = ([0.75, 0.1, 0.5], [0.32, 0.35, 0.4]);
const RIM: (V3, V3) = ([0.25, 0.55, -0.9], [0.55, 0.55, 0.6]);
const SKY: V3 = [0.62, 0.64, 0.7];
const GROUND: V3 = [0.2, 0.19, 0.18];

/// The model drawn to fit a `fit` px square.
pub(super) fn render(model: &Model, fit: u32) -> Option<RgbaImage> {
    let cam = Camera::frame(model, fit)?;
    let lights = Lights::new(&cam);
    let mut target = Target::new(cam.w * cam.ss, cam.h * cam.ss);
    // Everything solid first, so what is see-through blends over it.
    for blend in [false, true] {
        for prim in &model.prims {
            let mat = &model.materials[prim.material];
            if (mat.alpha == Alpha::Blend) != blend {
                continue;
            }
            let tex = mat
                .texture
                .as_ref()
                .and_then(|t| model.textures.get(t.texture)?.as_ref())
                .filter(|_| !prim.uvs.is_empty());
            draw(prim, mat, tex, &cam, &lights, &mut target, blend);
        }
    }
    Some(target.resolve(cam.w, cam.h, cam.ss))
}

struct Camera {
    eye: V3,
    right: V3,
    up: V3,
    fwd: V3,
    /// The model's left and top edges on the picture plane (x/z, y/z).
    left: f32,
    top: f32,
    /// Samples per unit of the picture plane.
    k: f32,
    /// Clear samples round the model.
    pad: f32,
    /// The picture's size in pixels, and samples per pixel each way.
    w: usize,
    h: usize,
    ss: usize,
}

impl Camera {
    /// Aim at the model from [`YAW`]/[`PITCH`], far enough that all of it
    /// is in front, and size the picture to what it covers there.
    fn frame(model: &Model, fit: u32) -> Option<Camera> {
        let drawn = || {
            model
                .prims
                .iter()
                .flat_map(|p| p.indices.iter().map(|&i| p.positions[i as usize]))
                .filter(|&p| math::is_finite(p))
        };
        let (mut lo, mut hi) = ([f32::INFINITY; 3], [f32::NEG_INFINITY; 3]);
        for p in drawn() {
            for a in 0..3 {
                lo[a] = lo[a].min(p[a]);
                hi[a] = hi[a].max(p[a]);
            }
        }
        let radius = math::length(sub(hi, lo)) / 2.0;
        if !(radius > 0.0 && radius.is_finite()) {
            return None;
        }
        let center = scale(add(lo, hi), 0.5);
        let (yaw, pitch) = (YAW.to_radians(), PITCH.to_radians());
        let back = [yaw.sin() * pitch.cos(), pitch.sin(), yaw.cos() * pitch.cos()];
        // The bounding sphere fits the view cone from here, so every vertex
        // is well in front of the camera.
        let dist = radius / (FOV.to_radians() / 2.0).sin() * 1.05;
        let eye = add(center, scale(back, dist));
        let fwd = scale(back, -1.0);
        let right = normalize(cross(fwd, [0.0, 1.0, 0.0]))?;
        let up = cross(right, fwd);

        let (mut x0, mut y0, mut x1, mut y1) = (f32::INFINITY, f32::INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY);
        for p in drawn() {
            let v = sub(p, eye);
            let z = dot(v, fwd);
            let (x, y) = (dot(v, right) / z, dot(v, up) / z);
            x0 = x0.min(x);
            x1 = x1.max(x);
            y0 = y0.min(y);
            y1 = y1.max(y);
        }
        let (bw, bh) = (x1 - x0, y1 - y0);
        let span = bw.max(bh);
        if !(span > 0.0 && span.is_finite()) {
            return None;
        }
        let k = (fit as f32 - 2.0 * PAD) / span;
        let size = |b: f32| ((b * k).ceil() + 2.0 * PAD).clamp(1.0, fit as f32) as usize;
        // Three samples a pixel each way at icon size; two for a big
        // picture, which keeps the buffers in tens of megabytes.
        let ss = if fit <= 256 { 3 } else { 2 };
        Some(Camera {
            eye,
            right,
            up,
            fwd,
            left: x0,
            top: y1,
            k: k * ss as f32,
            pad: PAD * ss as f32,
            w: size(bw),
            h: size(bh),
            ss,
        })
    }

    /// A world point in samples (x right, y down) and one over its depth.
    fn project(&self, p: V3) -> [f32; 3] {
        let v = sub(p, self.eye);
        let iz = 1.0 / dot(v, self.fwd);
        [
            (dot(v, self.right) * iz - self.left) * self.k + self.pad,
            (self.top - dot(v, self.up) * iz) * self.k + self.pad,
            iz,
        ]
    }
}

struct Light {
    /// Toward the light.
    dir: V3,
    color: V3,
}

struct Lights {
    eye: V3,
    lights: [Light; 3],
}

impl Lights {
    fn new(cam: &Camera) -> Lights {
        // The camera's frame as a matrix: right, up, and toward the camera.
        let mut frame = Mat4::IDENTITY;
        for (col, axis) in [cam.right, cam.up, scale(cam.fwd, -1.0)].into_iter().enumerate() {
            frame.0[col * 4..col * 4 + 3].copy_from_slice(&axis);
        }
        let light = |(dir, color): (V3, V3)| Light {
            dir: normalize(frame.point(dir)).unwrap_or(cam.up),
            color,
        };
        Lights { eye: cam.eye, lights: [light(KEY), light(FILL), light(RIM)] }
    }

    /// The light leaving a surface point toward the camera, linear.
    fn shade(&self, albedo: V3, mat: &Material, pos: V3, n: V3) -> V3 {
        let v = normalize(sub(self.eye, pos)).unwrap_or([0.0, 0.0, 1.0]);
        let mut n = normalize(n).unwrap_or(v);
        // Both sides of every face are drawn; light the one facing us.
        if dot(n, v) < 0.0 {
            n = scale(n, -1.0);
        }
        let metal = mat.metallic;
        let rough = mat.roughness.max(0.05);
        // Metal keeps some diffuse: with no scene around it to reflect, a
        // true metal is all highlight and reads as a dark blob at icon size.
        let diffuse_w = 1.0 - 0.6 * metal;
        let f0 = lerp([0.04; 3], albedo, metal);
        let shininess = (2.0 / rough.powi(4) - 2.0).clamp(4.0, 512.0);
        let norm = (shininess + 8.0) / (8.0 * std::f32::consts::PI);
        let hemi = |d: V3| lerp(GROUND, SKY, 0.5 + 0.5 * d[1]);

        let mut diffuse = hemi(n);
        let mut spec = [0.0; 3];
        for l in &self.lights {
            let ndl = dot(n, l.dir);
            if ndl <= 0.0 {
                continue;
            }
            diffuse = add(diffuse, scale(l.color, ndl));
            if let Some(h) = normalize(add(l.dir, v)) {
                let ndh = dot(n, h).max(0.0);
                spec = add(spec, scale(l.color, ndh.powf(shininess) * norm * ndl));
            }
        }
        // The studio reflected: what lets metal read as metal.
        let r = sub(scale(n, 2.0 * dot(n, v)), v);
        let env = scale(hemi(r), 1.0 - 0.7 * rough);
        add(scale(mul(albedo, diffuse), diffuse_w), mul(f0, add(spec, env)))
    }
}

/// The samples: depth (one over it, 0 where nothing is) and colour (linear,
/// premultiplied, already tone-mapped).
struct Target {
    w: usize,
    h: usize,
    depth: Vec<f32>,
    color: Vec<[f32; 4]>,
}

impl Target {
    fn new(w: usize, h: usize) -> Target {
        Target { w, h, depth: vec![0.0; w * h], color: vec![[0.0; 4]; w * h] }
    }

    /// Each pixel the average of its samples, as straight-alpha sRGB.
    fn resolve(&self, w: usize, h: usize, ss: usize) -> RgbaImage {
        let n = (ss * ss) as f32;
        RgbaImage::from_fn(w as u32, h as u32, |x, y| {
            let mut sum = [0.0f32; 4];
            for sy in 0..ss {
                let row = (y as usize * ss + sy) * self.w;
                for sx in 0..ss {
                    let c = self.color[row + x as usize * ss + sx];
                    for i in 0..4 {
                        sum[i] += c[i];
                    }
                }
            }
            let a = sum[3] / n;
            if a <= 0.0 {
                return image::Rgba([0, 0, 0, 0]);
            }
            let srgb = |c: f32| math::linear_to_srgb(c / n / a);
            image::Rgba([srgb(sum[0]), srgb(sum[1]), srgb(sum[2]), (a.min(1.0) * 255.0 + 0.5) as u8])
        })
    }
}

/// Twice the signed area of a, b, p (positive when counter-clockwise on
/// screen).
fn edge(a: [f32; 3], b: [f32; 3], px: f32, py: f32) -> f32 {
    (b[0] - a[0]) * (py - a[1]) - (b[1] - a[1]) * (px - a[0])
}

/// What a sample on a triangle needs to be shaded, perspective-correct.
struct Frag {
    pos: V3,
    normal: V3,
    uv: Option<[f32; 2]>,
    color: Option<[f32; 4]>,
}

fn draw(prim: &Prim, mat: &Material, tex: Option<&Texture>, cam: &Camera, lights: &Lights, t: &mut Target, blend: bool) {
    let proj: Vec<[f32; 3]> = prim.positions.iter().map(|&p| cam.project(p)).collect();
    let (wmax, hmax) = (t.w as f32 - 1.0, t.h as f32 - 1.0);
    for tri in prim.indices.chunks_exact(3) {
        let [i0, i1, i2] = [tri[0] as usize, tri[1] as usize, tri[2] as usize];
        let (a, b, c) = (proj[i0], proj[i1], proj[i2]);
        if ![a, b, c].iter().flatten().all(|v| v.is_finite()) || a[2] <= 0.0 || b[2] <= 0.0 || c[2] <= 0.0 {
            continue;
        }
        let area = edge(a, b, c[0], c[1]);
        if area.abs() < 1e-9 {
            continue;
        }
        let inv_area = 1.0 / area;
        let x0 = a[0].min(b[0]).min(c[0]).floor().max(0.0);
        let x1 = a[0].max(b[0]).max(c[0]).ceil().min(wmax);
        let y0 = a[1].min(b[1]).min(c[1]).floor().max(0.0);
        let y1 = a[1].max(b[1]).max(c[1]).ceil().min(hmax);
        if x0 > x1 || y0 > y1 {
            continue;
        }
        let (p0, p1, p2) = (prim.positions[i0], prim.positions[i1], prim.positions[i2]);
        let face = cross(sub(p1, p0), sub(p2, p0));
        for y in y0 as usize..=y1 as usize {
            let py = y as f32 + 0.5;
            for x in x0 as usize..=x1 as usize {
                let px = x as f32 + 0.5;
                let w0 = edge(b, c, px, py) * inv_area;
                let w1 = edge(c, a, px, py) * inv_area;
                let w2 = edge(a, b, px, py) * inv_area;
                // A hair of overlap, so float error never opens a crack
                // along an edge two triangles share.
                if w0 < -1e-5 || w1 < -1e-5 || w2 < -1e-5 {
                    continue;
                }
                let iz = w0 * a[2] + w1 * b[2] + w2 * c[2];
                let at = y * t.w + x;
                if iz <= t.depth[at] {
                    continue;
                }
                let q = [w0 * a[2] / iz, w1 * b[2] / iz, w2 * c[2] / iz];
                let frag = Frag {
                    pos: interp3(&prim.positions, [i0, i1, i2], q),
                    normal: if prim.normals.is_empty() {
                        face
                    } else {
                        interp3(&prim.normals, [i0, i1, i2], q)
                    },
                    uv: tex.map(|_| {
                        let uv = |i: usize, k: usize| prim.uvs[[i0, i1, i2][i]][k];
                        std::array::from_fn(|k| uv(0, k) * q[0] + uv(1, k) * q[1] + uv(2, k) * q[2])
                    }),
                    color: (!prim.colors.is_empty()).then(|| {
                        let col = |i: usize, k: usize| prim.colors[[i0, i1, i2][i]][k];
                        std::array::from_fn(|k| col(0, k) * q[0] + col(1, k) * q[1] + col(2, k) * q[2])
                    }),
                };
                let Some(rgba) = shade(&frag, mat, tex, lights) else {
                    continue;
                };
                let dst = &mut t.color[at];
                if blend {
                    let a = rgba[3];
                    *dst = [
                        rgba[0] * a + dst[0] * (1.0 - a),
                        rgba[1] * a + dst[1] * (1.0 - a),
                        rgba[2] * a + dst[2] * (1.0 - a),
                        a + dst[3] * (1.0 - a),
                    ];
                } else {
                    *dst = [rgba[0], rgba[1], rgba[2], 1.0];
                    t.depth[at] = iz;
                }
            }
        }
    }
}

fn interp3(v: &[V3], i: [usize; 3], q: [f32; 3]) -> V3 {
    add(add(scale(v[i[0]], q[0]), scale(v[i[1]], q[1])), scale(v[i[2]], q[2]))
}

/// A sample's colour, tone-mapped, and its alpha; `None` where a cut-out
/// material cuts it away.
fn shade(frag: &Frag, mat: &Material, tex: Option<&Texture>, lights: &Lights) -> Option<[f32; 4]> {
    let mut base = mat.base;
    if let (Some(t), Some(uv)) = (tex, frag.uv) {
        let s = t.sample(uv);
        base = std::array::from_fn(|i| base[i] * s[i]);
    }
    if let Some(c) = frag.color {
        base = std::array::from_fn(|i| base[i] * c[i]);
    }
    let alpha = match mat.alpha {
        Alpha::Opaque => 1.0,
        Alpha::Mask(cutoff) if base[3] < cutoff => return None,
        Alpha::Mask(_) => 1.0,
        Alpha::Blend if base[3] < 1.0 / 255.0 => return None,
        Alpha::Blend => base[3].min(1.0),
    };
    let albedo = [base[0], base[1], base[2]];
    let lit = if mat.unlit {
        albedo
    } else {
        lights.shade(albedo, mat, frag.pos, frag.normal)
    };
    let [r, g, b] = add(lit, mat.emissive).map(tonemap);
    Some([r, g, b, alpha])
}

/// Narkowicz's fit of the ACES filmic curve: highlights roll off instead
/// of clipping.
fn tonemap(x: f32) -> f32 {
    let x = x.max(0.0);
    ((x * (2.51 * x + 0.03)) / (x * (2.43 * x + 0.59) + 0.14)).clamp(0.0, 1.0)
}

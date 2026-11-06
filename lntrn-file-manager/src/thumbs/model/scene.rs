//! The scene as the thumbnail draws it: every triangle of the file's
//! default scene in world space, with its node transforms applied (skins
//! at rest, GPU instances copied out), and what shading needs beside it.

use std::path::Path;

use serde_json::Value;

use super::gltf::{numbers, uint, Gltf};
use super::material::{self, Material, Texture};
use super::math::{apply3, Mat4, V3};

/// Past this many triangles or vertices the model keeps its generic icon:
/// drawing it would take seconds, and its geometry hundreds of megabytes.
const MAX_TRIANGLES: usize = 4_000_000;
const MAX_VERTICES: usize = 6_000_000;
/// Most copies EXT_mesh_gpu_instancing may ask one node for.
const MAX_INSTANCES: usize = 65_536;

const TRIANGLES: usize = 4;
const TRIANGLE_STRIP: usize = 5;
const TRIANGLE_FAN: usize = 6;

/// One primitive, in world space.
pub(super) struct Prim {
    pub positions: Vec<V3>,
    /// As long as `positions`, or empty: then the faces are drawn flat.
    pub normals: Vec<V3>,
    /// The base colour picture's coordinates, its texture transform
    /// applied; as long as `positions`, or empty.
    pub uvs: Vec<[f32; 2]>,
    /// COLOR_0, linear RGBA; as long as `positions`, or empty.
    pub colors: Vec<[f32; 4]>,
    /// Triangles, three indices each.
    pub indices: Vec<u32>,
    /// Index into [`Model::materials`].
    pub material: usize,
}

pub(super) struct Model {
    pub prims: Vec<Prim>,
    /// The file's materials, then the one primitives without a material
    /// get.
    pub materials: Vec<Material>,
    /// The decoded base colour pictures, indexed like the file's textures.
    pub textures: Vec<Option<Texture>>,
}

/// How a node's vertices reach world space.
enum Xform {
    Rigid(Mat4),
    /// Each joint's world matrix times its inverse bind matrix; `node` for
    /// vertices without weights.
    Skin { joints: Vec<Mat4>, node: Mat4 },
}

/// Everything emitted so far, against the budget.
struct Out {
    prims: Vec<Prim>,
    vertices: usize,
    triangles: usize,
}

/// Read the model at `path` into what the rasterizer draws.
pub(super) fn load(path: &Path) -> Option<Model> {
    let mut g = super::gltf::open(path)?;
    if needs_unreadable_geometry(&g.json) {
        return None;
    }
    let mut materials = material::parse_all(&g.json);
    let fallback = materials.len();
    materials.push(Material::default());

    let worlds = world_matrices(&g.json);
    let mut out = Out { prims: Vec::new(), vertices: 0, triangles: 0 };
    for (node, world) in worlds.iter().enumerate() {
        let Some(world) = *world else { continue };
        let n = &g.json["nodes"][node];
        let (Some(mesh), skin) = (uint(n, "mesh"), uint(n, "skin")) else {
            continue;
        };
        // A skinned mesh is placed by its joints, never its own node (the
        // spec's rule); one whose skin can't be read stands where its node
        // puts it instead.
        let xforms = match skin.and_then(|s| skin_matrices(&mut g, s, &worlds)) {
            Some(joints) => vec![Xform::Skin { joints, node: world }],
            None => instances(&mut g, node, world),
        };
        let prims = g.json["meshes"][mesh]["primitives"].as_array().map_or(0, Vec::len);
        for prim in 0..prims {
            for xf in &xforms {
                emit(&mut g, mesh, prim, xf, &materials, fallback, &mut out)?;
            }
        }
    }
    if out.prims.is_empty() {
        return None;
    }
    let textures = material::decode_used(&mut g, &materials, &out.prims);
    Some(Model { prims: out.prims, materials, textures })
}

/// True when the file's geometry is compressed in a way this reader can't
/// undo. Other required extensions only change looks, and the picture is
/// drawn without them.
fn needs_unreadable_geometry(json: &Value) -> bool {
    json.get("extensionsRequired")
        .and_then(Value::as_array)
        .is_some_and(|exts| {
            exts.iter().any(|e| {
                matches!(
                    e.as_str(),
                    Some("KHR_draco_mesh_compression" | "EXT_meshopt_compression" | "KHR_meshopt_compression")
                )
            })
        })
}

/// Each node's world matrix, for the nodes in the default scene (`None`
/// for the rest). A file without scenes shows every root node.
fn world_matrices(json: &Value) -> Vec<Option<Mat4>> {
    let nodes = json.get("nodes").and_then(Value::as_array).map_or(&[][..], Vec::as_slice);
    let children = |n: &Value| -> Vec<usize> {
        n.get("children")
            .and_then(Value::as_array)
            .map(|c| c.iter().filter_map(|i| i.as_u64().map(|i| i as usize)).collect())
            .unwrap_or_default()
    };
    let scene = uint(json, "scene").unwrap_or(0);
    let roots: Vec<usize> = match json.get("scenes").and_then(|s| s.get(scene)) {
        Some(s) => s
            .get("nodes")
            .and_then(Value::as_array)
            .map(|r| r.iter().filter_map(|i| i.as_u64().map(|i| i as usize)).collect())
            .unwrap_or_default(),
        None => {
            let mut is_child = vec![false; nodes.len()];
            for c in nodes.iter().flat_map(children) {
                if let Some(slot) = is_child.get_mut(c) {
                    *slot = true;
                }
            }
            (0..nodes.len()).filter(|&i| !is_child[i]).collect()
        }
    };
    let mut world = vec![None; nodes.len()];
    let mut stack: Vec<(usize, Mat4)> = roots.into_iter().rev().map(|r| (r, Mat4::IDENTITY)).collect();
    while let Some((n, parent)) = stack.pop() {
        // A node already placed is a second parent or a cycle, both broken
        // files: it keeps its first place.
        if n >= nodes.len() || world[n].is_some() {
            continue;
        }
        let m = parent.mul(&local(&nodes[n]));
        world[n] = Some(m);
        stack.extend(children(&nodes[n]).into_iter().rev().map(|c| (c, m)));
    }
    world
}

/// A node's own transform: its matrix, or translation × rotation × scale.
fn local(node: &Value) -> Mat4 {
    if let Some(m) = numbers::<16>(node, "matrix") {
        return Mat4(m);
    }
    Mat4::from_trs(
        numbers::<3>(node, "translation").unwrap_or([0.0; 3]),
        numbers::<4>(node, "rotation").unwrap_or([0.0, 0.0, 0.0, 1.0]),
        numbers::<3>(node, "scale").unwrap_or([1.0; 3]),
    )
}

/// Skin `skin`'s joint matrices at rest (the pose its nodes are in).
fn skin_matrices(g: &mut Gltf, skin: usize, worlds: &[Option<Mat4>]) -> Option<Vec<Mat4>> {
    let s = &g.json["skins"][skin];
    let joints: Vec<usize> = s
        .get("joints")?
        .as_array()?
        .iter()
        .map(|j| j.as_u64().map(|j| j as usize))
        .collect::<Option<_>>()?;
    let ibm = uint(s, "inverseBindMatrices");
    if joints.is_empty() {
        return None;
    }
    let ibms = match ibm {
        Some(a) => match g.floats(a)? {
            (16, m) if m.len() >= joints.len() * 16 => Some(m),
            _ => return None,
        },
        None => None, // identity, as the spec says
    };
    joints
        .iter()
        .enumerate()
        .map(|(i, &j)| {
            let world = (*worlds.get(j)?)?;
            Some(match &ibms {
                Some(m) => world.mul(&Mat4::from_slice(&m[i * 16..])),
                None => world,
            })
        })
        .collect()
}

/// Where a node's mesh is drawn: once at its world matrix, or once per
/// EXT_mesh_gpu_instancing instance.
fn instances(g: &mut Gltf, node: usize, world: Mat4) -> Vec<Xform> {
    let attr = |name: &str| {
        g.json["nodes"][node]
            .pointer("/extensions/EXT_mesh_gpu_instancing/attributes")
            .and_then(|a| uint(a, name))
    };
    let (t, r, s) = (attr("TRANSLATION"), attr("ROTATION"), attr("SCALE"));
    if t.is_none() && r.is_none() && s.is_none() {
        return vec![Xform::Rigid(world)];
    }
    let mut read = |a: Option<usize>, comps: usize| match a.map(|a| g.floats(a)) {
        Some(Some((c, d))) if c == comps => Some(d),
        Some(_) => None, // there, but unreadable
        None => Some(Vec::new()),
    };
    let (Some(t), Some(r), Some(s)) = (read(t, 3), read(r, 4), read(s, 3)) else {
        return vec![Xform::Rigid(world)];
    };
    let count = (t.len() / 3).max(r.len() / 4).max(s.len() / 3).min(MAX_INSTANCES);
    (0..count)
        .map(|i| {
            let t = t.get(i * 3..i * 3 + 3).map_or([0.0; 3], |v| [v[0], v[1], v[2]]);
            let r = r.get(i * 4..i * 4 + 4).map_or([0.0, 0.0, 0.0, 1.0], |v| [v[0], v[1], v[2], v[3]]);
            let s = s.get(i * 3..i * 3 + 3).map_or([1.0; 3], |v| [v[0], v[1], v[2]]);
            Xform::Rigid(world.mul(&Mat4::from_trs(t, r, s)))
        })
        .collect()
}

/// Read primitive `prim` of mesh `mesh`, placed by `xf`, into `out`. A
/// primitive that can't be read (or isn't triangles) is skipped; `None`
/// means the model is over budget and the whole thumbnail is given up.
fn emit(
    g: &mut Gltf,
    mesh: usize,
    prim: usize,
    xf: &Xform,
    materials: &[Material],
    fallback: usize,
    out: &mut Out,
) -> Option<()> {
    let p = &g.json["meshes"][mesh]["primitives"][prim];
    let mode = uint(p, "mode").unwrap_or(TRIANGLES);
    let attrs = &p["attributes"];
    let material = uint(p, "material").filter(|&m| m < fallback).unwrap_or(fallback);
    let tex = materials[material].texture.as_ref();
    let uv_attr = tex.and_then(|t| uint(attrs, &format!("TEXCOORD_{}", t.set)));
    let uv_transform = tex.and_then(|t| t.transform);
    let (pos, nrm, col) = (uint(attrs, "POSITION"), uint(attrs, "NORMAL"), uint(attrs, "COLOR_0"));
    let (joints, weights) = (uint(attrs, "JOINTS_0"), uint(attrs, "WEIGHTS_0"));
    let indices = uint(p, "indices");

    if !matches!(mode, TRIANGLES | TRIANGLE_STRIP | TRIANGLE_FAN) {
        return Some(());
    }
    let Some((3, positions)) = pos.and_then(|a| g.floats(a)) else {
        return Some(());
    };
    let count = positions.len() / 3;
    let indices = match indices {
        Some(a) => match g.uints(a) {
            Some((1, i)) if i.iter().all(|&i| (i as usize) < count) => i,
            _ => return Some(()),
        },
        None => (0..count as u32).collect(),
    };
    let indices = triangulate(mode, indices);
    out.vertices += count;
    out.triangles += indices.len() / 3;
    if out.vertices > MAX_VERTICES || out.triangles > MAX_TRIANGLES {
        return None;
    }
    if indices.is_empty() {
        return Some(());
    }

    let mut read = |a: Option<usize>, comps: &[usize]| {
        a.and_then(|a| g.floats(a))
            .filter(|(c, d)| comps.contains(c) && d.len() == count * c)
    };
    let normals = read(nrm, &[3]).map(|(_, d)| d);
    let uvs = read(uv_attr, &[2]).map(|(_, d)| d);
    let colors = read(col, &[3, 4]);
    let weights = read(weights, &[4]).map(|(_, d)| d);
    let joints = joints
        .and_then(|a| g.uints(a))
        .filter(|(c, d)| *c == 4 && d.len() == count * 4)
        .map(|(_, d)| d);

    let to_v3 = |d: &[f32]| -> Vec<V3> { d.chunks_exact(3).map(|v| [v[0], v[1], v[2]]).collect() };
    let mut positions = to_v3(&positions);
    let mut normals = normals.map_or_else(Vec::new, |n| to_v3(&n));
    match xf {
        Xform::Skin { joints: mats, node } => match (&joints, &weights) {
            (Some(j), Some(w)) => skin(&mut positions, &mut normals, j, w, mats, node),
            _ => rigid(&mut positions, &mut normals, node),
        },
        Xform::Rigid(m) => rigid(&mut positions, &mut normals, m),
    }

    let uvs = uvs.map_or_else(Vec::new, |d| {
        d.chunks_exact(2)
            .map(|uv| match uv_transform {
                Some(t) => [t[0] * uv[0] + t[1] * uv[1] + t[2], t[3] * uv[0] + t[4] * uv[1] + t[5]],
                None => [uv[0], uv[1]],
            })
            .collect()
    });
    let colors = colors.map_or_else(Vec::new, |(c, d)| {
        d.chunks_exact(c)
            .map(|v| [v[0], v[1], v[2], if c == 4 { v[3] } else { 1.0 }])
            .collect()
    });
    out.prims.push(Prim { positions, normals, uvs, colors, indices, material });
    Some(())
}

fn rigid(positions: &mut [V3], normals: &mut [V3], m: &Mat4) {
    let nm = m.normal_matrix();
    for p in positions.iter_mut() {
        *p = m.point(*p);
    }
    for n in normals.iter_mut() {
        *n = apply3(&nm, *n);
    }
}

/// Linear blend skinning at rest: each vertex by its joints, weighted.
fn skin(positions: &mut [V3], normals: &mut [V3], joints: &[u32], weights: &[f32], mats: &[Mat4], node: &Mat4) {
    for v in 0..positions.len() {
        let mut m = Mat4([0.0; 16]);
        let mut total = 0.0;
        for k in v * 4..v * 4 + 4 {
            let w = weights[k];
            if let Some(jm) = mats.get(joints[k] as usize).filter(|_| w > 0.0) {
                m.add_scaled(jm, w);
                total += w;
            }
        }
        // Weights should add up to one; a vertex whose don't is shared out
        // in proportion, and one with none stays with its node.
        let m = if total > 1e-6 {
            Mat4(m.0.map(|c| c / total))
        } else {
            *node
        };
        positions[v] = m.point(positions[v]);
        if let Some(n) = normals.get_mut(v) {
            *n = apply3(&m.normal_matrix(), *n);
        }
    }
}

/// Strips and fans as plain triangles; a list cut to whole triangles.
fn triangulate(mode: usize, idx: Vec<u32>) -> Vec<u32> {
    match mode {
        TRIANGLE_STRIP => (0..idx.len().saturating_sub(2))
            .flat_map(|i| {
                if i % 2 == 0 {
                    [idx[i], idx[i + 1], idx[i + 2]]
                } else {
                    [idx[i + 1], idx[i], idx[i + 2]]
                }
            })
            .collect(),
        TRIANGLE_FAN => (1..idx.len().saturating_sub(1))
            .flat_map(|i| [idx[0], idx[i], idx[i + 1]])
            .collect(),
        _ => {
            let mut idx = idx;
            idx.truncate(idx.len() / 3 * 3);
            idx
        }
    }
}

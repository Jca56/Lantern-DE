use serde_json::{json, Value};

use super::*;

/// A scratch folder of its own for one test.
fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("lntrn-fm-model-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A unit cube (8 corners, 12 triangles, u16 indices) as a buffer: the
/// positions at 0, the indices at 96.
fn cube_bin() -> Vec<u8> {
    let mut bin = Vec::new();
    for i in 0..8u32 {
        for axis in 0..3 {
            let v: f32 = if i >> axis & 1 == 1 { 0.5 } else { -0.5 };
            bin.extend_from_slice(&v.to_le_bytes());
        }
    }
    let quads = [[0, 1, 3, 2], [4, 6, 7, 5], [0, 4, 5, 1], [2, 3, 7, 6], [0, 2, 6, 4], [1, 5, 7, 3]];
    for q in quads {
        for i in [q[0], q[1], q[2], q[0], q[2], q[3]] {
            bin.extend_from_slice(&(i as u16).to_le_bytes());
        }
    }
    bin
}

/// The JSON of a one-node scene showing the cube in `cube_bin`, coloured
/// `color`, the buffer at `uri` (none: a GLB's own chunk).
fn cube_json(uri: Option<String>, color: [f32; 4]) -> Value {
    let mut buffer = json!({ "byteLength": 168 });
    if let Some(uri) = uri {
        buffer["uri"] = json!(uri);
    }
    json!({
        "asset": { "version": "2.0" },
        "scene": 0,
        "scenes": [{ "nodes": [0] }],
        "nodes": [{ "mesh": 0 }],
        "meshes": [{ "primitives": [{ "attributes": { "POSITION": 0 }, "indices": 1, "material": 0 }] }],
        "materials": [{ "pbrMetallicRoughness": { "baseColorFactor": color, "metallicFactor": 0.0 } }],
        "accessors": [
            { "bufferView": 0, "componentType": 5126, "count": 8, "type": "VEC3",
              "min": [-0.5, -0.5, -0.5], "max": [0.5, 0.5, 0.5] },
            { "bufferView": 1, "componentType": 5123, "count": 36, "type": "SCALAR" }
        ],
        "bufferViews": [
            { "buffer": 0, "byteOffset": 0, "byteLength": 96 },
            { "buffer": 0, "byteOffset": 96, "byteLength": 72 }
        ],
        "buffers": [buffer]
    })
}

/// A GLB of `json` and `bin`, each chunk padded to four bytes.
fn glb(json: &Value, bin: &[u8]) -> Vec<u8> {
    let mut text = serde_json::to_vec(json).unwrap();
    while text.len() % 4 != 0 {
        text.push(b' ');
    }
    let mut bin = bin.to_vec();
    while bin.len() % 4 != 0 {
        bin.push(0);
    }
    let total = 12 + 8 + text.len() + 8 + bin.len();
    let mut out = b"glTF".to_vec();
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&(total as u32).to_le_bytes());
    out.extend_from_slice(&(text.len() as u32).to_le_bytes());
    out.extend_from_slice(b"JSON");
    out.extend(text);
    out.extend_from_slice(&(bin.len() as u32).to_le_bytes());
    out.extend_from_slice(b"BIN\0");
    out.extend(bin);
    out
}

fn base64_encode(data: &[u8]) -> String {
    const ABC: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in data.chunks(3) {
        let n = chunk.iter().enumerate().fold(0u32, |n, (i, &b)| n | u32::from(b) << (16 - 8 * i));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(ABC[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// The pixel at the middle of the picture.
fn middle(img: &image::RgbaImage) -> [u8; 4] {
    img.get_pixel(img.width() / 2, img.height() / 2).0
}

#[test]
fn a_glb_cube_draws_in_its_colour_with_clear_corners() {
    let dir = scratch("glb");
    let path = dir.join("cube.glb");
    std::fs::write(&path, glb(&cube_json(None, [0.8, 0.05, 0.05, 1.0]), &cube_bin())).unwrap();
    let img = thumbnail(&path, 192).expect("cube draws");
    assert!(img.width() <= 192 && img.height() <= 192);
    // Framed tight: it fills the box one way.
    assert!(img.width().max(img.height()) >= 190);
    let [r, g, b, a] = middle(&img);
    assert_eq!(a, 255);
    assert!(r > 150 && r > g * 2 && r > b * 2, "reddish, got {r} {g} {b}");
    // A cube seen from above at an angle leaves its corners clear.
    assert_eq!(img.get_pixel(0, 0).0[3], 0);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_gltf_reads_its_buffer_from_a_data_uri_or_a_file_beside_it() {
    let dir = scratch("gltf");
    let uri = format!("data:application/octet-stream;base64,{}", base64_encode(&cube_bin()));
    let embedded = dir.join("embedded.gltf");
    std::fs::write(&embedded, cube_json(Some(uri), [0.1, 0.7, 0.1, 1.0]).to_string()).unwrap();
    let [r, g, b, _] = middle(&thumbnail(&embedded, 96).expect("data uri"));
    assert!(g > r * 2 && g > b * 2, "greenish, got {r} {g} {b}");

    // A sibling file, its name percent-encoded in the URI.
    std::fs::write(dir.join("my cube.bin"), cube_bin()).unwrap();
    let beside = dir.join("beside.gltf");
    std::fs::write(&beside, cube_json(Some("my%20cube.bin".into()), [1.0; 4]).to_string()).unwrap();
    assert!(thumbnail(&beside, 96).is_some());

    // Only relative references are followed.
    for uri in ["/etc/passwd", "file:///tmp/x.bin", "https://example.com/x.bin", "missing.bin"] {
        let path = dir.join("bad.gltf");
        std::fs::write(&path, cube_json(Some(uri.into()), [1.0; 4]).to_string()).unwrap();
        assert!(thumbnail(&path, 96).is_none(), "{uri}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_texture_colours_the_surface() {
    let dir = scratch("texture");
    // A 2×2 blue PNG, after the geometry in the same buffer.
    let mut png = Vec::new();
    image::RgbaImage::from_pixel(2, 2, image::Rgba([20, 40, 230, 255]))
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();
    let mut bin = cube_bin();
    let uv_at = bin.len();
    for _ in 0..8 {
        bin.extend_from_slice(&0.5f32.to_le_bytes());
        bin.extend_from_slice(&0.5f32.to_le_bytes());
    }
    let png_at = bin.len();
    bin.extend_from_slice(&png);
    let mut j = cube_json(None, [1.0; 4]);
    j["buffers"][0]["byteLength"] = json!(bin.len());
    j["meshes"][0]["primitives"][0]["attributes"]["TEXCOORD_0"] = json!(2);
    j["accessors"].as_array_mut().unwrap().push(json!({ "bufferView": 2, "componentType": 5126, "count": 8, "type": "VEC2" }));
    let views = j["bufferViews"].as_array_mut().unwrap();
    views.push(json!({ "buffer": 0, "byteOffset": uv_at, "byteLength": 64 }));
    views.push(json!({ "buffer": 0, "byteOffset": png_at, "byteLength": png.len() }));
    j["images"] = json!([{ "bufferView": 3, "mimeType": "image/png" }]);
    j["textures"] = json!([{ "source": 0 }]);
    j["materials"][0]["pbrMetallicRoughness"]["baseColorTexture"] = json!({ "index": 0 });
    let path = dir.join("textured.glb");
    std::fs::write(&path, glb(&j, &bin)).unwrap();
    let [r, g, b, _] = middle(&thumbnail(&path, 96).expect("textured cube"));
    assert!(b > r * 3 && b > g * 2, "blue, got {r} {g} {b}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn nodes_skins_and_sparse_accessors_place_the_vertices() {
    let dir = scratch("scene");
    // The cube one node down, under a parent that moves it +10 on x; a
    // sparse accessor lifts corner 0 to y = 3.
    let mut j = cube_json(None, [1.0; 4]);
    j["nodes"] = json!([{ "translation": [10.0, 0.0, 0.0], "children": [1] }, { "mesh": 0 }]);
    let mut bin = cube_bin();
    let sparse_at = bin.len();
    bin.extend_from_slice(&0u16.to_le_bytes());
    bin.extend_from_slice(&[0, 0]);
    for v in [-0.5f32, 3.0, -0.5] {
        bin.extend_from_slice(&v.to_le_bytes());
    }
    j["buffers"][0]["byteLength"] = json!(bin.len());
    j["bufferViews"].as_array_mut().unwrap().extend([
        json!({ "buffer": 0, "byteOffset": sparse_at, "byteLength": 2 }),
        json!({ "buffer": 0, "byteOffset": sparse_at + 4, "byteLength": 12 }),
    ]);
    j["accessors"][0]["sparse"] = json!({
        "count": 1,
        "indices": { "bufferView": 2, "componentType": 5123 },
        "values": { "bufferView": 3 }
    });
    let path = dir.join("nested.glb");
    std::fs::write(&path, glb(&j, &bin)).unwrap();
    let model = scene::load(&path).unwrap();
    let p = &model.prims[0].positions;
    assert_eq!(p[0], [9.5, 3.0, -0.5]);
    assert_eq!(p[7], [10.5, 0.5, 0.5]);

    // Skinned to one joint that stands at y = 5: the mesh follows the joint
    // (its bind matrix the identity), and its own node's move is ignored.
    let mut j = cube_json(None, [1.0; 4]);
    j["nodes"] = json!([
        { "mesh": 0, "skin": 0, "translation": [100.0, 0.0, 0.0] },
        { "translation": [0.0, 5.0, 0.0] }
    ]);
    j["scenes"][0]["nodes"] = json!([0, 1]);
    j["skins"] = json!([{ "joints": [1] }]);
    let mut bin = cube_bin();
    let skin_at = bin.len();
    for _ in 0..8 {
        bin.extend_from_slice(&[0, 0, 0, 0]);
    }
    for _ in 0..8 {
        for w in [1.0f32, 0.0, 0.0, 0.0] {
            bin.extend_from_slice(&w.to_le_bytes());
        }
    }
    j["buffers"][0]["byteLength"] = json!(bin.len());
    j["bufferViews"].as_array_mut().unwrap().extend([
        json!({ "buffer": 0, "byteOffset": skin_at, "byteLength": 32 }),
        json!({ "buffer": 0, "byteOffset": skin_at + 32, "byteLength": 128 }),
    ]);
    j["accessors"].as_array_mut().unwrap().extend([
        json!({ "bufferView": 2, "componentType": 5121, "count": 8, "type": "VEC4" }),
        json!({ "bufferView": 3, "componentType": 5126, "count": 8, "type": "VEC4" }),
    ]);
    let attrs = &mut j["meshes"][0]["primitives"][0]["attributes"];
    attrs["JOINTS_0"] = json!(2);
    attrs["WEIGHTS_0"] = json!(3);
    let path = dir.join("skinned.glb");
    std::fs::write(&path, glb(&j, &bin)).unwrap();
    let model = scene::load(&path).unwrap();
    assert_eq!(model.prims[0].positions[0], [-0.5, 4.5, -0.5]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn what_cannot_be_drawn_gives_no_picture() {
    let dir = scratch("refused");
    let cases: Vec<(&str, Vec<u8>)> = vec![
        ("garbage.glb", b"glTF definitely not a model".to_vec()),
        ("empty.gltf", Vec::new()),
        ("other.gltf", br#"{"hello": "world"}"#.to_vec()),
        ("v1.gltf", br#"{"asset": {"version": "1.0"}}"#.to_vec()),
        ("cut.glb", glb(&cube_json(None, [1.0; 4]), &cube_bin())[..120].to_vec()),
        ("draco.glb", {
            let mut j = cube_json(None, [1.0; 4]);
            j["extensionsRequired"] = json!(["KHR_draco_mesh_compression"]);
            glb(&j, &cube_bin())
        }),
        ("lines.glb", {
            let mut j = cube_json(None, [1.0; 4]);
            j["meshes"][0]["primitives"][0]["mode"] = json!(1);
            glb(&j, &cube_bin())
        }),
        ("out-of-range.glb", {
            let mut j = cube_json(None, [1.0; 4]);
            j["accessors"][0]["count"] = json!(9);
            glb(&j, &cube_bin())
        }),
    ];
    for (name, bytes) in cases {
        let path = dir.join(name);
        std::fs::write(&path, bytes).unwrap();
        assert!(thumbnail(&path, 96).is_none(), "{name}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn base64_decodes_with_and_without_padding() {
    assert_eq!(gltf::base64("aGVsbG8=").unwrap(), b"hello");
    assert_eq!(gltf::base64("aGVsbG8").unwrap(), b"hello");
    assert_eq!(gltf::base64(&base64_encode(&[0, 255, 7, 9])).unwrap(), [0, 255, 7, 9]);
    assert!(gltf::base64("not*base64").is_none());
}

/// Draws the models named in `LNTRN_FM_MODEL_FILES` (separated by `:`)
/// into `LNTRN_FM_MODEL_OUT`, timing each: for judging the look on real
/// files. `cargo test --release -p lntrn-file-manager -- --ignored draw_models`
#[test]
#[ignore]
fn draw_models() {
    let (Ok(files), Ok(out)) = (std::env::var("LNTRN_FM_MODEL_FILES"), std::env::var("LNTRN_FM_MODEL_OUT")) else {
        return;
    };
    for file in files.split(':') {
        let path = std::path::Path::new(file);
        let start = std::time::Instant::now();
        let img = thumbnail(path, 192);
        println!("{file}: {:?} in {:?}", img.as_ref().map(|i| i.dimensions()), start.elapsed());
        if let Some(img) = img {
            let name = path.file_stem().unwrap().to_string_lossy();
            img.save(std::path::Path::new(&out).join(format!("{name}.png"))).unwrap();
        }
    }
}

//! The little linear algebra and colour arithmetic the model thumbnails
//! need: 3-vectors as arrays, column-major 4×4 matrices as glTF writes
//! them, and sRGB conversions.

use std::sync::OnceLock;

pub(super) type V3 = [f32; 3];

pub(super) fn add(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

pub(super) fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

pub(super) fn scale(a: V3, s: f32) -> V3 {
    [a[0] * s, a[1] * s, a[2] * s]
}

/// Component by component.
pub(super) fn mul(a: V3, b: V3) -> V3 {
    [a[0] * b[0], a[1] * b[1], a[2] * b[2]]
}

pub(super) fn dot(a: V3, b: V3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

pub(super) fn cross(a: V3, b: V3) -> V3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

pub(super) fn length(a: V3) -> f32 {
    dot(a, a).sqrt()
}

/// `a` made one long; `None` when it has no direction (zero, or not
/// finite).
pub(super) fn normalize(a: V3) -> Option<V3> {
    let l = length(a);
    (l > 0.0 && l.is_finite()).then(|| scale(a, 1.0 / l))
}

pub(super) fn lerp(a: V3, b: V3, t: f32) -> V3 {
    add(a, scale(sub(b, a), t))
}

pub(super) fn is_finite(a: V3) -> bool {
    a.iter().all(|v| v.is_finite())
}

/// A 4×4 matrix, column-major (`m[col * 4 + row]`), as glTF stores them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Mat4(pub [f32; 16]);

impl Mat4 {
    pub const IDENTITY: Mat4 = Mat4([
        1.0, 0.0, 0.0, 0.0, //
        0.0, 1.0, 0.0, 0.0, //
        0.0, 0.0, 1.0, 0.0, //
        0.0, 0.0, 0.0, 1.0,
    ]);

    /// The matrix the first 16 numbers of `m` write.
    pub fn from_slice(m: &[f32]) -> Mat4 {
        let mut out = [0.0; 16];
        out.copy_from_slice(&m[..16]);
        Mat4(out)
    }

    /// Translation × rotation (a quaternion, x y z w) × scale, the order a
    /// glTF node's TRS means.
    pub fn from_trs(t: V3, q: [f32; 4], s: V3) -> Mat4 {
        let n = (q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3]).sqrt();
        let [x, y, z, w] = if n > 0.0 && n.is_finite() {
            q.map(|c| c / n)
        } else {
            [0.0, 0.0, 0.0, 1.0]
        };
        let (xx, yy, zz) = (x * x, y * y, z * z);
        let (xy, xz, yz) = (x * y, x * z, y * z);
        let (wx, wy, wz) = (w * x, w * y, w * z);
        Mat4([
            (1.0 - 2.0 * (yy + zz)) * s[0],
            2.0 * (xy + wz) * s[0],
            2.0 * (xz - wy) * s[0],
            0.0,
            2.0 * (xy - wz) * s[1],
            (1.0 - 2.0 * (xx + zz)) * s[1],
            2.0 * (yz + wx) * s[1],
            0.0,
            2.0 * (xz + wy) * s[2],
            2.0 * (yz - wx) * s[2],
            (1.0 - 2.0 * (xx + yy)) * s[2],
            0.0,
            t[0],
            t[1],
            t[2],
            1.0,
        ])
    }

    pub fn mul(&self, o: &Mat4) -> Mat4 {
        let (a, b) = (&self.0, &o.0);
        let mut out = [0.0; 16];
        for c in 0..4 {
            for r in 0..4 {
                out[c * 4 + r] = (0..4).map(|k| a[k * 4 + r] * b[c * 4 + k]).sum();
            }
        }
        Mat4(out)
    }

    /// Where the matrix takes point `p` (an affine transform: the bottom
    /// row is taken as 0 0 0 1, as glTF requires).
    pub fn point(&self, p: V3) -> V3 {
        let m = &self.0;
        [
            m[0] * p[0] + m[4] * p[1] + m[8] * p[2] + m[12],
            m[1] * p[0] + m[5] * p[1] + m[9] * p[2] + m[13],
            m[2] * p[0] + m[6] * p[1] + m[10] * p[2] + m[14],
        ]
    }

    /// The matrix that carries normals: the inverse transpose of the upper
    /// 3×3, up to a positive scale (normals are made one long after),
    /// row-major.
    pub fn normal_matrix(&self) -> [f32; 9] {
        let m = &self.0;
        let a = |r: usize, c: usize| m[c * 4 + r];
        let cof = [
            a(1, 1) * a(2, 2) - a(1, 2) * a(2, 1),
            a(1, 2) * a(2, 0) - a(1, 0) * a(2, 2),
            a(1, 0) * a(2, 1) - a(1, 1) * a(2, 0),
            a(0, 2) * a(2, 1) - a(0, 1) * a(2, 2),
            a(0, 0) * a(2, 2) - a(0, 2) * a(2, 0),
            a(0, 1) * a(2, 0) - a(0, 0) * a(2, 1),
            a(0, 1) * a(1, 2) - a(0, 2) * a(1, 1),
            a(0, 2) * a(1, 0) - a(0, 0) * a(1, 2),
            a(0, 0) * a(1, 1) - a(0, 1) * a(1, 0),
        ];
        // The cofactors are the inverse transpose times the determinant: a
        // mirroring transform would turn every normal round without this.
        let det = a(0, 0) * cof[0] + a(0, 1) * cof[1] + a(0, 2) * cof[2];
        if det < 0.0 {
            cof.map(|c| -c)
        } else {
            cof
        }
    }

    /// `self += o × w`, for blending a skin's joints.
    pub fn add_scaled(&mut self, o: &Mat4, w: f32) {
        for (a, b) in self.0.iter_mut().zip(o.0) {
            *a += b * w;
        }
    }
}

/// A row-major 3×3 matrix (from [`Mat4::normal_matrix`]) applied to `v`.
pub(super) fn apply3(m: &[f32; 9], v: V3) -> V3 {
    [
        m[0] * v[0] + m[1] * v[1] + m[2] * v[2],
        m[3] * v[0] + m[4] * v[1] + m[5] * v[2],
        m[6] * v[0] + m[7] * v[1] + m[8] * v[2],
    ]
}

/// An 8-bit sRGB channel as linear light.
pub(super) fn srgb_to_linear(c: u8) -> f32 {
    static LUT: OnceLock<[f32; 256]> = OnceLock::new();
    LUT.get_or_init(|| {
        std::array::from_fn(|i| {
            let c = i as f32 / 255.0;
            if c <= 0.04045 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        })
    })[c as usize]
}

/// Linear light, 0..1, as an 8-bit sRGB channel.
pub(super) fn linear_to_srgb(l: f32) -> u8 {
    let l = if l.is_finite() { l.clamp(0.0, 1.0) } else { 0.0 };
    let s = if l <= 0.003_130_8 {
        l * 12.92
    } else {
        1.055 * l.powf(1.0 / 2.4) - 0.055
    };
    (s * 255.0 + 0.5) as u8
}

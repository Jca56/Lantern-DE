//! Font variations: `fvar` axes + named instances, `avar` axis remapping,
//! coordinate normalization, the ItemVariationStore (shared by `HVAR` and
//! friends), and `HVAR` advance-width deltas.
//!
//! Outline deltas (`gvar`) live in `gvar.rs`; this module owns everything
//! scalar-shaped.

use super::sfnt::{read_i16_at, read_u16_at, read_u32_at, read_u8_at};

#[derive(Clone, Copy, Debug)]
pub(crate) struct Axis {
    pub tag: [u8; 4],
    pub min: f32,
    pub default: f32,
    pub max: f32,
}

#[derive(Clone, Debug)]
pub(crate) struct Instance {
    /// User-space value per axis, fvar order.
    pub coords: Vec<f32>,
}

fn fixed(d: &[u8], pos: usize) -> Option<f32> {
    read_u32_at(d, pos).ok().map(|v| v as i32 as f32 / 65536.0)
}

fn f2dot14(d: &[u8], pos: usize) -> Option<f32> {
    read_i16_at(d, pos).ok().map(|v| v as f32 / 16384.0)
}

/// Parse `fvar`: the axis list and named instances. `t` is the table slice.
pub(crate) fn parse_fvar(t: &[u8]) -> Option<(Vec<Axis>, Vec<Instance>)> {
    let axes_off = read_u16_at(t, 4).ok()? as usize;
    let axis_count = read_u16_at(t, 8).ok()? as usize;
    let axis_size = read_u16_at(t, 10).ok()? as usize;
    let instance_count = read_u16_at(t, 12).ok()? as usize;
    let instance_size = read_u16_at(t, 14).ok()? as usize;

    let mut axes = Vec::with_capacity(axis_count);
    for i in 0..axis_count {
        let rec = axes_off + i * axis_size;
        axes.push(Axis {
            tag: read_u32_at(t, rec).ok()?.to_be_bytes(),
            min: fixed(t, rec + 4)?,
            default: fixed(t, rec + 8)?,
            max: fixed(t, rec + 12)?,
        });
    }

    let instances_off = axes_off + axis_count * axis_size;
    let mut instances = Vec::with_capacity(instance_count);
    for i in 0..instance_count {
        let rec = instances_off + i * instance_size;
        let mut coords = Vec::with_capacity(axis_count);
        for a in 0..axis_count {
            coords.push(fixed(t, rec + 4 + a * 4)?);
        }
        instances.push(Instance { coords });
    }
    Some((axes, instances))
}

/// User coords → normalized [-1, 1] per axis (fvar order), with `avar`
/// remapping when present. `user` pairs may cover any subset of axes.
pub(crate) fn normalize(axes: &[Axis], avar: Option<&[u8]>, user: &[([u8; 4], f32)]) -> Vec<f32> {
    let mut out = Vec::with_capacity(axes.len());
    for axis in axes {
        let v = user
            .iter()
            .find(|(t, _)| *t == axis.tag)
            .map_or(axis.default, |&(_, v)| v)
            .clamp(axis.min, axis.max);
        let n = if v > axis.default && axis.max > axis.default {
            (v - axis.default) / (axis.max - axis.default)
        } else if v < axis.default && axis.min < axis.default {
            (v - axis.default) / (axis.default - axis.min)
        } else {
            0.0
        };
        out.push(n.clamp(-1.0, 1.0));
    }
    if let Some(avar) = avar {
        apply_avar(avar, &mut out);
    }
    out
}

/// `avar`: per-axis piecewise-linear segment maps over normalized values.
fn apply_avar(t: &[u8], coords: &mut [f32]) {
    let Ok(axis_count) = read_u16_at(t, 4) else {
        return;
    };
    let mut pos = 8usize;
    for coord in coords.iter_mut().take(axis_count as usize) {
        let Ok(pairs) = read_u16_at(t, pos) else {
            return;
        };
        pos += 2;
        let map_base = pos;
        pos += pairs as usize * 4;
        if pairs < 2 {
            continue;
        }
        let pair = |i: usize| -> Option<(f32, f32)> {
            Some((
                f2dot14(t, map_base + i * 4)?,
                f2dot14(t, map_base + i * 4 + 2)?,
            ))
        };
        let v = *coord;
        let mut mapped = v;
        for i in 0..pairs as usize - 1 {
            let (Some((from_a, to_a)), Some((from_b, to_b))) = (pair(i), pair(i + 1)) else {
                break;
            };
            if v <= from_a {
                mapped = to_a;
                break;
            }
            if v <= from_b {
                mapped = if from_b > from_a {
                    to_a + (v - from_a) / (from_b - from_a) * (to_b - to_a)
                } else {
                    to_a
                };
                break;
            }
            mapped = to_b;
        }
        *coord = mapped;
    }
}

/// Scalar for one variation region axis: a ramp over start → peak → end.
pub(crate) fn axis_scalar(coord: f32, start: f32, peak: f32, end: f32) -> f32 {
    if peak == 0.0 {
        return 1.0;
    }
    if coord < start || coord > end {
        return 0.0;
    }
    if (coord - peak).abs() < f32::EPSILON {
        return 1.0;
    }
    if coord < peak {
        if peak > start {
            (coord - start) / (peak - start)
        } else {
            1.0
        }
    } else if end > peak {
        (end - coord) / (end - peak)
    } else {
        1.0
    }
}

// ── ItemVariationStore ──────────────────────────────────────────────────────

/// Delta store for metric-style variations (HVAR/MVAR/CFF2). `off` is the
/// store's absolute offset in the font data.
pub(crate) struct ItemVariationStore {
    off: usize,
}

impl ItemVariationStore {
    pub fn new(off: usize) -> ItemVariationStore {
        ItemVariationStore { off }
    }

    /// Interpolated delta for (outer, inner) at normalized `coords`.
    pub fn delta(&self, d: &[u8], outer: u16, inner: u16, coords: &[f32]) -> f32 {
        self.try_delta(d, outer, inner, coords).unwrap_or(0.0)
    }

    fn try_delta(&self, d: &[u8], outer: u16, inner: u16, coords: &[f32]) -> Option<f32> {
        let region_list = self.off + read_u32_at(d, self.off + 2).ok()? as usize;
        let data_count = read_u16_at(d, self.off + 6).ok()?;
        if outer >= data_count {
            return None;
        }
        let data = self.off + read_u32_at(d, self.off + 8 + outer as usize * 4).ok()? as usize;

        let axis_count = read_u16_at(d, region_list).ok()? as usize;
        let item_count = read_u16_at(d, data).ok()?;
        if inner >= item_count {
            return None;
        }
        let word_info = read_u16_at(d, data + 2).ok()?;
        let long_words = word_info & 0x8000 != 0;
        let word_count = (word_info & 0x7FFF) as usize;
        let region_count = read_u16_at(d, data + 4).ok()? as usize;
        let regions_idx = data + 6;
        let (word_size, small_size) = if long_words { (4, 2) } else { (2, 1) };
        let row_size =
            word_count * word_size + region_count.saturating_sub(word_count) * small_size;
        let row = regions_idx + region_count * 2 + inner as usize * row_size;

        let mut total = 0.0f32;
        let mut pos = row;
        for r in 0..region_count {
            let delta = if r < word_count {
                if long_words {
                    let v = read_u32_at(d, pos).ok()? as i32 as f32;
                    pos += 4;
                    v
                } else {
                    let v = read_i16_at(d, pos).ok()? as f32;
                    pos += 2;
                    v
                }
            } else if long_words {
                let v = read_i16_at(d, pos).ok()? as f32;
                pos += 2;
                v
            } else {
                let v = read_u8_at(d, pos).ok()? as i8 as f32;
                pos += 1;
                v
            };
            if delta == 0.0 {
                continue;
            }
            let region_index = read_u16_at(d, regions_idx + r * 2).ok()? as usize;
            let region = region_list + 4 + region_index * axis_count * 6;
            let mut scalar = 1.0f32;
            for (a, &coord) in coords.iter().enumerate().take(axis_count) {
                let base = region + a * 6;
                let start = f2dot14(d, base)?;
                let peak = f2dot14(d, base + 2)?;
                let end = f2dot14(d, base + 4)?;
                scalar *= axis_scalar(coord, start, peak, end);
                if scalar == 0.0 {
                    break;
                }
            }
            total += scalar * delta;
        }
        Some(total)
    }
}

// ── HVAR ────────────────────────────────────────────────────────────────────

pub(crate) struct Hvar {
    ivs: ItemVariationStore,
    /// Absolute offset of the advance-width DeltaSetIndexMap (None = glyph
    /// id used directly as the inner index with outer 0).
    advance_map: Option<usize>,
}

impl Hvar {
    /// `off` = absolute offset of the HVAR table.
    pub fn parse(d: &[u8], off: usize) -> Option<Hvar> {
        let ivs_off = read_u32_at(d, off + 4).ok()? as usize;
        if ivs_off == 0 {
            return None;
        }
        let adv_off = read_u32_at(d, off + 8).ok()? as usize;
        Some(Hvar {
            ivs: ItemVariationStore::new(off + ivs_off),
            advance_map: (adv_off != 0).then_some(off + adv_off),
        })
    }

    pub fn advance_delta(&self, d: &[u8], gid: u16, coords: &[f32]) -> f32 {
        let (outer, inner) = match self.advance_map {
            Some(map) => delta_set_index(d, map, gid).unwrap_or((0, gid)),
            None => (0, gid),
        };
        self.ivs.delta(d, outer, inner, coords)
    }
}

/// DeltaSetIndexMap (format 0, the HVAR flavor): entry → (outer, inner).
fn delta_set_index(d: &[u8], map: usize, gid: u16) -> Option<(u16, u16)> {
    let entry_format = read_u16_at(d, map).ok()?;
    let map_count = read_u16_at(d, map + 2).ok()?;
    if map_count == 0 {
        return None;
    }
    let index = gid.min(map_count - 1) as usize;
    let entry_size = (((entry_format >> 4) & 0x3) + 1) as usize;
    let inner_bits = ((entry_format & 0xF) + 1) as u32;
    let mut value = 0u32;
    for k in 0..entry_size {
        value = (value << 8) | read_u8_at(d, map + 4 + index * entry_size + k).ok()? as u32;
    }
    Some((
        (value >> inner_bits) as u16,
        (value & ((1 << inner_bits) - 1)) as u16,
    ))
}

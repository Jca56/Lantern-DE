//! Signed-area scanline coverage accumulator.
//!
//! The classic exact-coverage AA rasterizer: every line segment deposits
//! per-pixel signed area *deltas* into a float buffer; a per-row prefix sum
//! then yields final coverage. Direction (winding) is carried in the sign of
//! each deposit, and `|sum|` clamped to 1 resolves fills, so closed contours
//! of either winding rasterize correctly without an explicit edge list or
//! active-edge table.
//!
//! Rows are `width + 1` floats wide: the extra "spill" column absorbs the
//! fractional tail of spans that end exactly on the right edge, keeping every
//! row's deltas balanced without out-of-bounds writes.

pub(crate) struct Accumulator {
    w: usize,
    h: usize,
    a: Vec<f32>,
}

impl Accumulator {
    pub fn new(w: usize, h: usize) -> Self {
        Self {
            w,
            h,
            a: vec![0.0; (w + 1) * h],
        }
    }

    /// Accumulate one line segment given in pixel space (y down, origin at the
    /// bitmap's top-left). Horizontal segments contribute no winding and are
    /// skipped.
    pub fn line(&mut self, p0: [f32; 2], p1: [f32; 2]) {
        if (p0[1] - p1[1]).abs() < 1e-7 {
            return;
        }
        // Normalize to top→bottom, remembering original direction for winding.
        let (dir, top, bot) = if p0[1] < p1[1] {
            (1.0f32, p0, p1)
        } else {
            (-1.0f32, p1, p0)
        };
        let hf = self.h as f32;
        if top[1] >= hf || bot[1] <= 0.0 {
            return;
        }
        let dxdy = (bot[0] - top[0]) / (bot[1] - top[1]);
        let y_start = top[1].max(0.0) as usize;
        let y_end = (bot[1].min(hf).ceil() as usize).min(self.h);
        let wf = self.w as f32;
        let stride = self.w + 1;
        // x at the segment's entry into the first row (advanced if clipped).
        let mut x = top[0] + 0.0f32.max(-top[1]) * dxdy;

        for y in y_start..y_end {
            let row = y * stride;
            let dy = (((y + 1) as f32).min(bot[1]) - (y as f32).max(top[1])).max(0.0);
            let xnext = x + dxdy * dy;
            let d = dy * dir;
            let (lx, rx) = if x < xnext { (x, xnext) } else { (xnext, x) };
            let lx = lx.clamp(0.0, wf);
            let rx = rx.clamp(0.0, wf);
            let x0floor = lx.floor();
            let x0i = x0floor as usize;
            let x1ceil = rx.ceil();
            let x1i = x1ceil as usize;

            if x0i >= self.w {
                // Entire span at/beyond the right edge — spill column only.
                self.a[row + self.w] += d;
            } else if x1i <= x0i + 1 {
                // Span within a single pixel column: split the area between
                // this column and the next by the span's mean x.
                let xmf = 0.5 * (lx + rx) - x0floor;
                self.a[row + x0i] += d * (1.0 - xmf);
                self.a[row + x0i + 1] += d * xmf;
            } else {
                // Span crosses several columns: trapezoidal area per column.
                let s = (rx - lx).recip();
                let x0f = lx - x0floor;
                let a0 = 0.5 * s * (1.0 - x0f) * (1.0 - x0f);
                let x1f = rx - x1ceil + 1.0;
                let am = 0.5 * s * x1f * x1f;
                self.a[row + x0i] += d * a0;
                if x1i == x0i + 2 {
                    self.a[row + x0i + 1] += d * (1.0 - a0 - am);
                } else {
                    let a1 = s * (1.5 - x0f);
                    self.a[row + x0i + 1] += d * (a1 - a0);
                    for xi in x0i + 2..x1i - 1 {
                        self.a[row + xi] += d * s;
                    }
                    let a2 = a1 + (x1i - x0i - 3) as f32 * s;
                    self.a[row + x1i - 1] += d * (1.0 - a2 - am);
                }
                self.a[row + x1i] += d * am;
            }
            x = xnext;
        }
    }

    /// Resolve deltas into an 8-bit coverage bitmap (`w * h`, row-major).
    pub fn finish(self) -> Vec<u8> {
        let stride = self.w + 1;
        let mut out = vec![0u8; self.w * self.h];
        for y in 0..self.h {
            let mut acc = 0.0f32;
            for x in 0..self.w {
                acc += self.a[y * stride + x];
                out[y * self.w + x] = (acc.abs().min(1.0) * 255.0 + 0.5) as u8;
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::Accumulator;

    fn rect(acc: &mut Accumulator, x0: f32, y0: f32, x1: f32, y1: f32) {
        acc.line([x0, y0], [x0, y1]); // left, downward
        acc.line([x0, y1], [x1, y1]);
        acc.line([x1, y1], [x1, y0]); // right, upward
        acc.line([x1, y0], [x0, y0]);
    }

    #[test]
    fn full_square_is_opaque() {
        let mut acc = Accumulator::new(4, 4);
        rect(&mut acc, 0.0, 0.0, 4.0, 4.0);
        let bmp = acc.finish();
        assert!(
            bmp.iter().all(|&c| c == 255),
            "expected full coverage, got {bmp:?}"
        );
    }

    #[test]
    fn half_pixel_edges() {
        let mut acc = Accumulator::new(4, 4);
        rect(&mut acc, 0.5, 0.0, 3.5, 4.0);
        let bmp = acc.finish();
        for y in 0..4 {
            let row = &bmp[y * 4..y * 4 + 4];
            assert!((row[0] as i32 - 128).abs() <= 1, "left edge: {row:?}");
            assert_eq!(row[1], 255);
            assert_eq!(row[2], 255);
            assert!((row[3] as i32 - 128).abs() <= 1, "right edge: {row:?}");
        }
    }

    #[test]
    fn winding_direction_irrelevant() {
        let mut cw = Accumulator::new(3, 3);
        rect(&mut cw, 0.0, 0.0, 3.0, 3.0);
        let mut ccw = Accumulator::new(3, 3);
        // Reverse point order → opposite winding.
        ccw.line([0.0, 0.0], [3.0, 0.0]);
        ccw.line([3.0, 0.0], [3.0, 3.0]);
        ccw.line([3.0, 3.0], [0.0, 3.0]);
        ccw.line([0.0, 3.0], [0.0, 0.0]);
        assert_eq!(cw.finish(), ccw.finish());
    }
}

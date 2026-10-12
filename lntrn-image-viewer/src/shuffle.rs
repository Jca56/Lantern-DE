//! Shuffle playback order for Left/Right: a Fisher–Yates deal over the
//! folder's files that plays every image once per pass, then deals again.

/// Tiny PRNG (xorshift64) — shuffling doesn't need more.
struct XorShift64 {
    state: u64,
}

impl XorShift64 {
    fn from_time() -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0xa5a5_5a5a_a5a5_5a5a);
        Self {
            state: if nanos == 0 {
                0xdead_beef_cafe_babe
            } else {
                nanos
            },
        }
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.state = x;
        x
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next_u64() as usize) % n
    }
}

pub struct Shuffle {
    order: Vec<usize>,
    pos: usize,
    rng: XorShift64,
}

impl Shuffle {
    pub fn new() -> Self {
        Self {
            order: Vec::new(),
            pos: 0,
            rng: XorShift64::from_time(),
        }
    }

    /// Deal a fresh order over `n` files. The open one (`current`) leads it,
    /// so the pass that follows reaches every other file exactly once.
    pub fn deal(&mut self, n: usize, current: usize) {
        self.order = (0..n).collect();
        for i in (1..n).rev() {
            let j = self.rng.below(i + 1);
            self.order.swap(i, j);
        }
        if let Some(p) = self.order.iter().position(|&i| i == current) {
            self.order.swap(0, p);
        }
        self.pos = 0;
    }

    /// Keep the order in step with a direct jump to `current`. A different
    /// list (rescanned folder, `force`) gets a new deal.
    pub fn follow(&mut self, n: usize, current: usize, force: bool) {
        if force || self.order.len() != n {
            self.deal(n, current);
        } else if let Some(p) = self.order.iter().position(|&i| i == current) {
            self.pos = p;
        }
    }

    /// The next file in shuffled order; the end of a pass deals a new one.
    pub fn next(&mut self, n: usize, current: usize) -> Option<usize> {
        if self.order.is_empty() {
            return None;
        }
        self.pos += 1;
        if self.pos >= self.order.len() {
            // New pass: `current` leads the fresh deal, so step past it
            // rather than showing the same image twice in a row.
            self.deal(n, current);
            self.pos = usize::from(self.order.len() > 1);
        }
        self.order.get(self.pos).copied()
    }

    pub fn prev(&mut self) -> Option<usize> {
        if self.order.is_empty() {
            return None;
        }
        self.pos = self.pos.checked_sub(1).unwrap_or(self.order.len() - 1);
        self.order.get(self.pos).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_pass_plays_every_file_once() {
        let mut s = Shuffle::new();
        s.deal(7, 3);
        let mut seen = vec![3];
        let mut current = 3;
        for _ in 0..6 {
            current = s.next(7, current).unwrap();
            seen.push(current);
        }
        seen.sort_unstable();
        assert_eq!(seen, (0..7).collect::<Vec<_>>());
        // The end of a pass deals again without repeating the open image.
        let a = s.next(7, current).unwrap();
        assert_ne!(a, current);
        // prev undoes next.
        let b = s.next(7, a).unwrap();
        assert_eq!(s.prev(), Some(a));
        assert_ne!(a, b);
        assert_eq!(Shuffle::new().next(0, 0), None);
    }
}

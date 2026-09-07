//! Find in a terminal: the query, every place it appears in the
//! scrollback and on the screen (case-insensitive), which one is
//! current, and the view scrolled to it.

use super::Terminal;
use super::grid::Grid;

/// A hit: the absolute row, the first cell, cells long.
pub type Hit = (u64, usize, usize);

pub struct TermSearch {
    pub query: String,
    pub matches: Vec<Hit>,
    pub current: usize,
    /// The field takes focus on the next draw.
    pub focus: bool,
    /// The view scrolls to the current hit on the next draw.
    pub follow: bool,
    /// What the matches were computed for: output time, rows scrolled
    /// off, columns, the query.
    key: (f64, u64, usize, String),
}

impl TermSearch {
    pub fn new() -> Self {
        Self { query: String::new(), matches: Vec::new(), current: 0, focus: true, follow: false, key: (-1.0, 0, 0, String::new()) }
    }

    /// Matches for the terminal as it is now; recomputed when the output
    /// or the query changed. The current hit stays put when it can.
    pub fn refresh(&mut self, term: &Terminal) {
        let key = (term.last_output, term.grid.scrolled_off, term.grid.cols, self.query.clone());
        if key == self.key {
            return;
        }
        let query_changed = key.3 != self.key.3;
        self.key = key;
        let keep = self.matches.get(self.current).copied();
        self.matches = if self.query.is_empty() { Vec::new() } else { find(&term.grid, &self.query) };
        self.current = match keep {
            Some(h) if !query_changed => self.matches.iter().position(|m| *m == h).unwrap_or(0),
            _ => {
                // A new query: the hit nearest the bottom of what is shown.
                let bottom = term.grid.abs_row(term.grid.rows.saturating_sub(1));
                self.matches.iter().rposition(|m| m.0 <= bottom).unwrap_or(0)
            }
        };
        self.follow = query_changed && !self.matches.is_empty();
    }

    /// The next (or previous) hit, wrapping around.
    pub fn step(&mut self, by: isize) {
        if self.matches.is_empty() {
            return;
        }
        let n = self.matches.len() as isize;
        self.current = ((self.current as isize + by).rem_euclid(n)) as usize;
        self.follow = true;
    }

    pub fn current(&self) -> Option<Hit> {
        self.matches.get(self.current).copied()
    }

    /// Scroll `grid` so the current hit sits mid-screen (once per ask).
    pub fn scroll_to_current(&mut self, grid: &mut Grid) {
        if !std::mem::take(&mut self.follow) {
            return;
        }
        let Some((abs, _, _)) = self.current() else {
            return;
        };
        let Some(i) = abs.checked_sub(grid.scrolled_off) else {
            return;
        };
        let back = grid.scrollback.len();
        let want_start = (i as usize).saturating_sub(grid.rows / 2).min(back);
        grid.view_offset = back - want_start;
    }
}

/// Every place `query` appears, oldest first, as cells.
pub fn find(grid: &Grid, query: &str) -> Vec<Hit> {
    let q: Vec<char> = query.chars().flat_map(char::to_lowercase).collect();
    if q.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let total = grid.scrollback.len() + grid.rows;
    let mut cells: Vec<char> = Vec::new();
    for i in 0..total {
        let abs = grid.scrolled_off + i as u64;
        let Some(row) = grid.row_by_abs(abs) else { continue };
        cells.clear();
        cells.extend(row.iter().map(|c| c.ch.to_lowercase().next().unwrap_or(c.ch)));
        let mut from = 0;
        while from + q.len() <= cells.len() {
            if cells[from..from + q.len()] == q[..] {
                out.push((abs, from, q.len()));
                from += q.len();
            } else {
                from += 1;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::term::csi;
    use crate::term::parser::Parser;

    fn grid_with(text: &str, rows: usize) -> Grid {
        let mut g = Grid::new(20, rows, 100);
        let mut p = Parser::new();
        p.feed(text.as_bytes(), |a| csi::dispatch(&mut g, a));
        g
    }

    #[test]
    fn finds_across_scrollback_ignoring_case() {
        let g = grid_with("Error one\r\nfine\r\nerror two\r\nlast\r\n", 2);
        assert!(g.scrollback.len() >= 2, "the first rows scrolled off the screen");
        let hits = find(&g, "ERROR");
        assert_eq!(hits.len(), 2);
        assert_eq!((hits[0].1, hits[0].2), (0, 5));
        assert!(hits[0].0 < hits[1].0, "oldest first");
        assert!(find(&g, "").is_empty());
    }
}

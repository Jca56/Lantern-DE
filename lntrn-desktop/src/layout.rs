use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Grid cell dimensions in logical pixels. Big for the user's eyesight.
pub const CELL_W: f32 = 140.0;
pub const CELL_H: f32 = 160.0;
pub const ICON_PX: f32 = 96.0;
pub const LABEL_PX: f32 = 16.0;
pub const PAD_TOP: f32 = 24.0;
pub const PAD_LEFT: f32 = 24.0;

/// (column, row) in the grid.
pub type Cell = (i32, i32);

/// Persistent position map: filename (just the basename) -> cell.
#[derive(Default, Serialize, Deserialize)]
pub struct PositionMap {
    #[serde(default)]
    pub positions: HashMap<String, [i32; 2]>,
}

impl PositionMap {
    pub fn get(&self, name: &str) -> Option<Cell> {
        self.positions.get(name).map(|a| (a[0], a[1]))
    }
    pub fn set(&mut self, name: &str, cell: Cell) {
        self.positions.insert(name.to_string(), [cell.0, cell.1]);
    }
    pub fn remove(&mut self, name: &str) {
        self.positions.remove(name);
    }
    pub fn load(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }
    pub fn save(&self, path: &Path) {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(s) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(path, s);
        }
    }
}

pub fn position_map_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    PathBuf::from(home).join(".lantern/config/desktop-icons.json")
}

/// Convert a cell to the top-left pixel of its icon area in logical coords.
pub fn cell_origin(cell: Cell) -> (f32, f32) {
    (
        PAD_LEFT + cell.0 as f32 * CELL_W,
        PAD_TOP + cell.1 as f32 * CELL_H,
    )
}

/// Convert a logical pixel position to the nearest grid cell.
pub fn pixel_to_cell(x: f32, y: f32) -> Cell {
    let col = ((x - PAD_LEFT + CELL_W * 0.5) / CELL_W).floor() as i32;
    let row = ((y - PAD_TOP + CELL_H * 0.5) / CELL_H).floor() as i32;
    (col.max(0), row.max(0))
}

/// How many columns / rows fit in the surface.
pub fn grid_dims(surface_w: f32, surface_h: f32) -> (i32, i32) {
    let cols = ((surface_w - PAD_LEFT) / CELL_W).floor().max(1.0) as i32;
    let rows = ((surface_h - PAD_TOP) / CELL_H).floor().max(1.0) as i32;
    (cols, rows)
}

/// Find the first empty cell, scanning column-first (top→bottom, then right).
/// Skips cells already in `occupied`.
pub fn first_empty_cell(occupied: &std::collections::HashSet<Cell>, dims: (i32, i32)) -> Cell {
    let (cols, rows) = dims;
    for c in 0..cols {
        for r in 0..rows {
            let cell = (c, r);
            if !occupied.contains(&cell) {
                return cell;
            }
        }
    }
    // Overflow off-screen; just stack at the end
    (cols.max(1) - 1, rows.max(1))
}

/// Assign cells to items, respecting saved positions first, then filling empty cells.
/// Returns a Vec aligned with `items` indices: items[i] is at returned[i].
pub fn assign_cells(
    items: &[crate::icons::DesktopItem],
    positions: &mut PositionMap,
    dims: (i32, i32),
) -> Vec<Cell> {
    use std::collections::HashSet;
    let mut occupied: HashSet<Cell> = HashSet::new();
    let mut assigned: Vec<Option<Cell>> = vec![None; items.len()];

    // First pass: respect saved positions (skip collisions — first wins).
    for (i, item) in items.iter().enumerate() {
        if let Some(cell) = positions.get(&item.name) {
            if !occupied.contains(&cell) {
                occupied.insert(cell);
                assigned[i] = Some(cell);
            }
        }
    }
    // Second pass: fill empties.
    for (i, item) in items.iter().enumerate() {
        if assigned[i].is_none() {
            let cell = first_empty_cell(&occupied, dims);
            occupied.insert(cell);
            assigned[i] = Some(cell);
            positions.set(&item.name, cell);
        }
    }
    assigned.into_iter().map(|c| c.unwrap_or((0, 0))).collect()
}

/// Hit-test a point against the icon cells. Returns the item index, if any.
/// We hit the union of the icon image and its label.
pub fn hit_test(x: f32, y: f32, cells: &[Cell]) -> Option<usize> {
    for (i, cell) in cells.iter().enumerate() {
        let (ox, oy) = cell_origin(*cell);
        if x >= ox && x < ox + CELL_W && y >= oy && y < oy + CELL_H {
            return Some(i);
        }
    }
    None
}

/// Hit-test any items intersected by a rect (used for rubber-band selection).
pub fn rect_hits(rect: (f32, f32, f32, f32), cells: &[Cell]) -> Vec<usize> {
    let (rx, ry, rw, rh) = rect;
    let (rx2, ry2) = (rx + rw, ry + rh);
    let mut hits = Vec::new();
    for (i, cell) in cells.iter().enumerate() {
        let (ox, oy) = cell_origin(*cell);
        let (ox2, oy2) = (ox + CELL_W, oy + CELL_H);
        let overlap_x = rx < ox2 && rx2 > ox;
        let overlap_y = ry < oy2 && ry2 > oy;
        if overlap_x && overlap_y {
            hits.push(i);
        }
    }
    hits
}

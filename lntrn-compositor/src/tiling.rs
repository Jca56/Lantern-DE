/// BSP (binary space partition) tiling window layout.
///
/// Arena-based binary tree where each node is either a Split (H/V with ratio)
/// or a Leaf (holding a window surface). Layout is computed by recursively
/// subdividing a root rectangle with gaps between windows and at screen edges.

use smithay::{
    output::Output,
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::{Logical, Point, Rectangle, Size},
};

use crate::state::Lantern;
use crate::window_ext::WindowExt;

/// Inner gap between tiled windows — read from [window_manager].gap, default 8.
pub fn default_gap() -> i32 {
    crate::read_config("window_manager", "gap", "8")
        .parse::<i32>()
        .unwrap_or(8)
        .clamp(0, 32)
}

/// Outer gap (between tiled windows and the screen edge). Matches the inner
/// gap so users see one consistent value.
pub fn default_outer_gap() -> i32 {
    default_gap()
}

/// Larger outer gap used when the active tiling tree has exactly one window,
/// so a solo tiled window doesn't sit flush against the screen edges.
pub const SINGLE_WINDOW_OUTER_GAP: i32 = 40;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SplitDirection {
    Horizontal, // children side-by-side (left | right)
    Vertical,   // children stacked (top / bottom)
}

impl SplitDirection {
    pub fn opposite(self) -> Self {
        match self {
            Self::Horizontal => Self::Vertical,
            Self::Vertical => Self::Horizontal,
        }
    }
}

#[derive(Debug, Clone)]
pub enum TilingNode {
    Split {
        dir: SplitDirection,
        ratio: f32,
        left: usize,
        right: usize,
    },
    Leaf {
        surface: WlSurface,
    },
    Empty,
}

pub struct TilingState {
    nodes: Vec<TilingNode>,
    root: Option<usize>,
    pub active: bool,
    pub gap: i32,
    pub outer_gap: i32,
}

impl TilingState {
    pub fn new() -> Self {
        Self {
            nodes: Vec::new(),
            root: None,
            active: false,
            gap: default_gap(),
            outer_gap: default_outer_gap(),
        }
    }

    /// Toggle tiling on/off. Returns new active state.
    pub fn toggle(&mut self) -> bool {
        self.active = !self.active;
        if !self.active {
            self.clear();
        }
        self.active
    }

    pub fn clear(&mut self) {
        self.nodes.clear();
        self.root = None;
    }

    pub fn contains(&self, surface: &WlSurface) -> bool {
        self.nodes.iter().any(|n| matches!(n, TilingNode::Leaf { surface: s } if s == surface))
    }

    pub fn leaf_count(&self) -> usize {
        self.nodes.iter().filter(|n| matches!(n, TilingNode::Leaf { .. })).count()
    }

    fn alloc(&mut self, node: TilingNode) -> usize {
        // Reuse an Empty slot if available
        if let Some(idx) = self.nodes.iter().position(|n| matches!(n, TilingNode::Empty)) {
            self.nodes[idx] = node;
            idx
        } else {
            let idx = self.nodes.len();
            self.nodes.push(node);
            idx
        }
    }

    /// Insert a new surface into the tree using balanced BSP.
    /// Splits the shallowest leaf (or `near` if specified) to keep the tree
    /// balanced. Split direction is the opposite of the parent's, so the
    /// layout naturally progresses: full → left|right → master+stack → 2×2 grid.
    pub fn insert(&mut self, surface: WlSurface, near: Option<&WlSurface>) {
        let new_leaf = self.alloc(TilingNode::Leaf { surface });

        let Some(root) = self.root else {
            // First window: just set as root
            self.root = Some(new_leaf);
            return;
        };

        // Find the leaf to split: prefer `near`, otherwise pick the shallowest
        let target = near
            .and_then(|s| self.find_leaf(s))
            .unwrap_or_else(|| {
                self.shallowest_leaf(root)
                    .map(|(idx, _)| idx)
                    .unwrap_or(root)
            });

        // Direction: opposite of parent's split, or Horizontal for root-level
        let dir = self.parent_direction(target)
            .map(|d| d.opposite())
            .unwrap_or(SplitDirection::Horizontal);

        // Replace target leaf with a split, target becomes left child
        let old_node = self.nodes[target].clone();
        let moved = self.alloc(old_node);

        self.nodes[target] = TilingNode::Split {
            dir,
            ratio: 0.5,
            left: moved,
            right: new_leaf,
        };
    }

    /// Remove a surface from the tree. Its sibling takes the parent's slot.
    pub fn remove(&mut self, surface: &WlSurface) {
        let Some(leaf_idx) = self.find_leaf(surface) else { return };

        // If this is the root leaf, just clear
        if self.root == Some(leaf_idx) {
            self.nodes[leaf_idx] = TilingNode::Empty;
            self.root = None;
            return;
        }

        // Find the parent split that contains this leaf
        let Some(parent_idx) = self.find_parent(leaf_idx) else { return };

        let (sibling_idx, _) = match &self.nodes[parent_idx] {
            TilingNode::Split { left, right, .. } => {
                if *left == leaf_idx {
                    (*right, *left)
                } else {
                    (*left, *right)
                }
            }
            _ => return,
        };

        // Move sibling into parent's slot (parent inherits sibling's content)
        let sibling_node = self.nodes[sibling_idx].clone();
        // Update any children of the sibling to still point correctly
        // (they already do since we're moving content, not indices that reference parent)
        self.nodes[parent_idx] = sibling_node;
        self.nodes[sibling_idx] = TilingNode::Empty;
        self.nodes[leaf_idx] = TilingNode::Empty;

    }

    /// Swap two surfaces in the tree.
    pub fn swap(&mut self, a: &WlSurface, b: &WlSurface) {
        let Some(a_idx) = self.find_leaf(a) else { return };
        let Some(b_idx) = self.find_leaf(b) else { return };
        if a_idx == b_idx {
            return;
        }
        // Just swap the surface references
        let a_node = self.nodes[a_idx].clone();
        let b_node = self.nodes[b_idx].clone();
        self.nodes[a_idx] = b_node;
        self.nodes[b_idx] = a_node;
    }

    /// Resize the parent split of the leaf containing `surface` by `delta`.
    /// Positive delta grows the window, negative shrinks it.
    pub fn resize_split(&mut self, surface: &WlSurface, delta: f32) {
        let Some(leaf_idx) = self.find_leaf(surface) else { return };
        let Some(parent_idx) = self.find_parent(leaf_idx) else { return };

        // Read the left child index first to avoid borrow conflict
        let left_idx = match &self.nodes[parent_idx] {
            TilingNode::Split { left, .. } => *left,
            _ => return,
        };
        let is_left = self.subtree_contains(left_idx, surface);

        if let TilingNode::Split { ratio, .. } = &mut self.nodes[parent_idx] {
            let new_ratio = if is_left {
                *ratio + delta
            } else {
                *ratio - delta
            };
            *ratio = new_ratio.clamp(0.1, 0.9);
        }
    }

    /// Compute layout rectangles for all leaf windows.
    /// `area` should already be inset by outer_gap.
    pub fn compute_layout(
        &self,
        area: Rectangle<i32, Logical>,
    ) -> Vec<(WlSurface, Rectangle<i32, Logical>)> {
        let mut result = Vec::new();
        if let Some(root) = self.root {
            self.layout_node(root, area, &mut result);
        }
        result
    }

    fn layout_node(
        &self,
        idx: usize,
        area: Rectangle<i32, Logical>,
        out: &mut Vec<(WlSurface, Rectangle<i32, Logical>)>,
    ) {
        match &self.nodes[idx] {
            TilingNode::Leaf { surface } => {
                out.push((surface.clone(), area));
            }
            TilingNode::Split { dir, ratio, left, right } => {
                let half_gap = self.gap / 2;
                match dir {
                    SplitDirection::Horizontal => {
                        let left_w = ((area.size.w as f32) * ratio) as i32 - half_gap;
                        let right_w = area.size.w - left_w - self.gap;
                        let left_rect = Rectangle::new(
                            area.loc,
                            Size::from((left_w, area.size.h)),
                        );
                        let right_rect = Rectangle::new(
                            Point::from((area.loc.x + left_w + self.gap, area.loc.y)),
                            Size::from((right_w, area.size.h)),
                        );
                        self.layout_node(*left, left_rect, out);
                        self.layout_node(*right, right_rect, out);
                    }
                    SplitDirection::Vertical => {
                        let top_h = ((area.size.h as f32) * ratio) as i32 - half_gap;
                        let bottom_h = area.size.h - top_h - self.gap;
                        let top_rect = Rectangle::new(
                            area.loc,
                            Size::from((area.size.w, top_h)),
                        );
                        let bottom_rect = Rectangle::new(
                            Point::from((area.loc.x, area.loc.y + top_h + self.gap)),
                            Size::from((area.size.w, bottom_h)),
                        );
                        self.layout_node(*left, top_rect, out);
                        self.layout_node(*right, bottom_rect, out);
                    }
                }
            }
            TilingNode::Empty => {}
        }
    }

    /// Find the leaf index for a surface.
    fn find_leaf(&self, surface: &WlSurface) -> Option<usize> {
        self.nodes.iter().position(|n| {
            matches!(n, TilingNode::Leaf { surface: s } if s == surface)
        })
    }

    /// Find the parent split index of a given node.
    fn find_parent(&self, child_idx: usize) -> Option<usize> {
        self.nodes.iter().position(|n| {
            matches!(n, TilingNode::Split { left, right, .. } if *left == child_idx || *right == child_idx)
        })
    }

    /// Check if a subtree rooted at `idx` contains the given surface.
    fn subtree_contains(&self, idx: usize, surface: &WlSurface) -> bool {
        match &self.nodes[idx] {
            TilingNode::Leaf { surface: s } => s == surface,
            TilingNode::Split { left, right, .. } => {
                self.subtree_contains(*left, surface)
                    || self.subtree_contains(*right, surface)
            }
            TilingNode::Empty => false,
        }
    }

    /// Find the shallowest (minimum depth) leaf in a subtree.
    /// Picks the first one in left-to-right order on ties.
    fn shallowest_leaf(&self, idx: usize) -> Option<(usize, usize)> {
        self.shallowest_leaf_at(idx, 0)
    }

    fn shallowest_leaf_at(&self, idx: usize, depth: usize) -> Option<(usize, usize)> {
        match &self.nodes[idx] {
            TilingNode::Leaf { .. } => Some((idx, depth)),
            TilingNode::Split { left, right, .. } => {
                let l = self.shallowest_leaf_at(*left, depth + 1);
                let r = self.shallowest_leaf_at(*right, depth + 1);
                match (l, r) {
                    (Some((li, ld)), Some((ri, rd))) => {
                        if ld <= rd { Some((li, ld)) } else { Some((ri, rd)) }
                    }
                    (Some(v), None) | (None, Some(v)) => Some(v),
                    (None, None) => None,
                }
            }
            TilingNode::Empty => None,
        }
    }

    /// Get the split direction of a node's parent.
    fn parent_direction(&self, child_idx: usize) -> Option<SplitDirection> {
        self.find_parent(child_idx).and_then(|p| match &self.nodes[p] {
            TilingNode::Split { dir, .. } => Some(*dir),
            _ => None,
        })
    }

    /// Compute the rectangle a given node occupies, given the root's area.
    /// Returns None if the node is unreachable from the root.
    pub fn node_rect(
        &self,
        target_idx: usize,
        area: Rectangle<i32, Logical>,
    ) -> Option<Rectangle<i32, Logical>> {
        let root = self.root?;
        self.node_rect_walk(root, area, target_idx)
    }

    fn node_rect_walk(
        &self,
        idx: usize,
        area: Rectangle<i32, Logical>,
        target: usize,
    ) -> Option<Rectangle<i32, Logical>> {
        if idx == target {
            return Some(area);
        }
        let TilingNode::Split { dir, ratio, left, right } = &self.nodes[idx] else {
            return None;
        };
        let half_gap = self.gap / 2;
        let (left_rect, right_rect) = match dir {
            SplitDirection::Horizontal => {
                let lw = ((area.size.w as f32) * ratio) as i32 - half_gap;
                let rw = area.size.w - lw - self.gap;
                (
                    Rectangle::new(area.loc, Size::from((lw, area.size.h))),
                    Rectangle::new(
                        Point::from((area.loc.x + lw + self.gap, area.loc.y)),
                        Size::from((rw, area.size.h)),
                    ),
                )
            }
            SplitDirection::Vertical => {
                let th = ((area.size.h as f32) * ratio) as i32 - half_gap;
                let bh = area.size.h - th - self.gap;
                (
                    Rectangle::new(area.loc, Size::from((area.size.w, th))),
                    Rectangle::new(
                        Point::from((area.loc.x, area.loc.y + th + self.gap)),
                        Size::from((area.size.w, bh)),
                    ),
                )
            }
        };
        self.node_rect_walk(*left, left_rect, target)
            .or_else(|| self.node_rect_walk(*right, right_rect, target))
    }

    /// Find the nearest ancestor split of a leaf whose direction matches
    /// `want_dir` AND where the leaf sits in the requested subtree side
    /// (`want_left_side` true = leaf is in the left/top child).
    /// Used by edge-drag resize: dragging the right edge of a leaf wants
    /// the nearest H-split where leaf is left; dragging left edge wants
    /// leaf in right; analog for top/bottom + V-splits.
    pub fn find_ancestor_split(
        &self,
        surface: &WlSurface,
        want_dir: SplitDirection,
        want_left_side: bool,
    ) -> Option<usize> {
        let leaf = self.find_leaf(surface)?;
        let mut cur = leaf;
        loop {
            let parent = self.find_parent(cur)?;
            if let TilingNode::Split { dir, left, .. } = &self.nodes[parent] {
                if *dir == want_dir {
                    let is_left_side = self.subtree_contains(*left, surface);
                    if is_left_side == want_left_side {
                        return Some(parent);
                    }
                }
                cur = parent;
            } else {
                return None;
            }
        }
    }

    /// Read a split's direction.
    pub fn split_dir(&self, idx: usize) -> Option<SplitDirection> {
        match self.nodes.get(idx)? {
            TilingNode::Split { dir, .. } => Some(*dir),
            _ => None,
        }
    }

    /// Read a split's current ratio.
    pub fn split_ratio(&self, idx: usize) -> Option<f32> {
        match self.nodes.get(idx)? {
            TilingNode::Split { ratio, .. } => Some(*ratio),
            _ => None,
        }
    }

    /// Set a split's ratio with a safety clamp.
    pub fn set_split_ratio(&mut self, idx: usize, new_ratio: f32) {
        if let Some(TilingNode::Split { ratio, .. }) = self.nodes.get_mut(idx) {
            *ratio = new_ratio.clamp(0.05, 0.95);
        }
    }

    /// Locate a seam (the gap between two siblings of a Split) at a logical
    /// point. Returns the deepest matching split index plus the rect that
    /// split occupies. `grab_radius` widens the hit zone perpendicular to
    /// the seam axis. Recurses depth-first so child seams win over parents.
    pub fn seam_at(
        &self,
        point: Point<i32, Logical>,
        area: Rectangle<i32, Logical>,
        grab_radius: i32,
    ) -> Option<(usize, Rectangle<i32, Logical>)> {
        let root = self.root?;
        self.seam_at_walk(root, area, point, grab_radius)
    }

    fn seam_at_walk(
        &self,
        idx: usize,
        area: Rectangle<i32, Logical>,
        point: Point<i32, Logical>,
        grab_radius: i32,
    ) -> Option<(usize, Rectangle<i32, Logical>)> {
        let TilingNode::Split { dir, ratio, left, right } = &self.nodes[idx] else {
            return None;
        };
        let half_gap = self.gap / 2;
        let (left_rect, right_rect, seam_min, seam_max, in_perp) = match dir {
            SplitDirection::Horizontal => {
                let lw = ((area.size.w as f32) * ratio) as i32 - half_gap;
                let seam_x = area.loc.x + lw + half_gap;
                let lr = Rectangle::new(area.loc, Size::from((lw, area.size.h)));
                let rr = Rectangle::new(
                    Point::from((area.loc.x + lw + self.gap, area.loc.y)),
                    Size::from((area.size.w - lw - self.gap, area.size.h)),
                );
                let perp = point.y >= area.loc.y && point.y <= area.loc.y + area.size.h;
                (lr, rr, seam_x - grab_radius, seam_x + grab_radius, perp)
            }
            SplitDirection::Vertical => {
                let th = ((area.size.h as f32) * ratio) as i32 - half_gap;
                let seam_y = area.loc.y + th + half_gap;
                let lr = Rectangle::new(area.loc, Size::from((area.size.w, th)));
                let rr = Rectangle::new(
                    Point::from((area.loc.x, area.loc.y + th + self.gap)),
                    Size::from((area.size.w, area.size.h - th - self.gap)),
                );
                let perp = point.x >= area.loc.x && point.x <= area.loc.x + area.size.w;
                (lr, rr, seam_y - grab_radius, seam_y + grab_radius, perp)
            }
        };
        if let Some(child) = self.seam_at_walk(*left, left_rect, point, grab_radius)
            .or_else(|| self.seam_at_walk(*right, right_rect, point, grab_radius))
        {
            return Some(child);
        }
        let along = match dir {
            SplitDirection::Horizontal => point.x >= seam_min && point.x <= seam_max,
            SplitDirection::Vertical => point.y >= seam_min && point.y <= seam_max,
        };
        if along && in_perp {
            Some((idx, area))
        } else {
            None
        }
    }

    /// Find a neighbor window in a given direction from the focused surface.
    pub fn find_adjacent(
        &self,
        surface: &WlSurface,
        area: Rectangle<i32, Logical>,
        dir: AdjacentDir,
    ) -> Option<WlSurface> {
        let layout = self.compute_layout(area);
        let current = layout.iter().find(|(s, _)| s == surface)?;
        let current_rect = current.1;

        // Find the center of the current window
        let cx = current_rect.loc.x + current_rect.size.w / 2;
        let cy = current_rect.loc.y + current_rect.size.h / 2;

        layout
            .iter()
            .filter(|(s, _)| s != surface)
            .filter(|(_, rect)| {
                let nx = rect.loc.x + rect.size.w / 2;
                let ny = rect.loc.y + rect.size.h / 2;
                match dir {
                    AdjacentDir::Left => nx < cx,
                    AdjacentDir::Right => nx > cx,
                    AdjacentDir::Up => ny < cy,
                    AdjacentDir::Down => ny > cy,
                }
            })
            .min_by_key(|(_, rect)| {
                let nx = rect.loc.x + rect.size.w / 2;
                let ny = rect.loc.y + rect.size.h / 2;
                let dx = (nx - cx).abs();
                let dy = (ny - cy).abs();
                dx * dx + dy * dy // distance squared
            })
            .map(|(s, _)| s.clone())
    }

}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdjacentDir {
    Left,
    Right,
    Up,
    Down,
}

/// Methods on Lantern for applying tiling layout.
impl Lantern {
    /// Get the tiling area for a specific output.
    pub fn tiling_area_for_output(&self, output: &Output) -> Option<Rectangle<i32, Logical>> {
        let geo = self.workspaces.output_geometry(output)?;
        let mut area = Rectangle::new(geo.loc.into(), geo.size);
        let (top_excl, bottom_excl, left_excl, right_excl) = self.exclusive_zone_offsets_for_output(output);
        area.loc.x += left_excl;
        area.loc.y += top_excl;
        area.size.w -= left_excl + right_excl;
        area.size.h -= top_excl + bottom_excl;
        let single_window = self
            .workspaces
            .active_tiling_tree(&output.name())
            .map(|t| t.leaf_count() == 1)
            .unwrap_or(false);
        let gap = if single_window {
            SINGLE_WINDOW_OUTER_GAP
        } else {
            self.workspaces.outer_gap
        };
        area.loc.x += gap;
        area.loc.y += gap;
        area.size.w -= gap * 2;
        area.size.h -= gap * 2;
        Some(area)
    }

    /// Get the tiling area for the output a surface's window is on.
    pub fn tiling_area_for_surface(&self, surface: &WlSurface) -> Option<Rectangle<i32, Logical>> {
        let window = self.find_mapped_window(surface)?;
        let output = self.output_for_window(&window)
            .or_else(|| self.workspaces.outputs_iter().next().cloned())?;
        self.tiling_area_for_output(&output)
    }

    /// Recompute tiling layout and animate all tiled windows to their targets.
    pub fn apply_tiling_layout(&mut self) {
        if !self.workspaces.tiling_active {
            return;
        }

        // Collect per-output layouts from each output's active workspace
        let outputs: Vec<Output> = self.workspaces.outputs_iter().cloned().collect();
        let mut all_layout = Vec::new();

        for output in &outputs {
            let name = output.name();
            let Some(area) = self.tiling_area_for_output(output) else { continue };
            if let Some(tree) = self.workspaces.active_tiling_tree(&name) {
                all_layout.extend(tree.compute_layout(area));
            }
        }

        for (surface, target_rect) in all_layout {
            // Skip windows the user has manually snapped via keyboard cycling —
            // their rect is owned by the snap system, not BSP.
            if self.snapped_windows.iter().any(|s| s.surface == surface) {
                continue;
            }
            let Some(window) = self.find_mapped_window(&surface) else { continue };

            let current_loc = self.workspaces.element_location(&window).unwrap_or_default();
            let current_size = window.geometry().size;
            let current_rect = Rectangle::new(current_loc, current_size);

            if current_rect != target_rect {
                self.tiling_anim.animate_to(&surface, current_rect, target_rect);
            }

            window.set_tiled(true);
            window.configure_size(target_rect.size);
            self.remap_tracked_window(window, target_rect.loc, false);
        }

        self.schedule_render();
    }

    /// Same as `apply_tiling_layout` but skips the per-window animation.
    /// Used during interactive resize so the layout follows the cursor in
    /// real time without the rubbery feel of an animator catching up.
    pub fn apply_tiling_layout_immediate(&mut self) {
        if !self.workspaces.tiling_active {
            return;
        }
        let outputs: Vec<Output> = self.workspaces.outputs_iter().cloned().collect();
        let mut all_layout = Vec::new();
        for output in &outputs {
            let name = output.name();
            let Some(area) = self.tiling_area_for_output(output) else { continue };
            if let Some(tree) = self.workspaces.active_tiling_tree(&name) {
                all_layout.extend(tree.compute_layout(area));
            }
        }
        for (surface, target_rect) in all_layout {
            if self.snapped_windows.iter().any(|s| s.surface == surface) {
                continue;
            }
            let Some(window) = self.find_mapped_window(&surface) else { continue };
            // Cancel any in-flight tile animation; we are tracking the cursor.
            self.tiling_anim.remove(&surface);
            window.set_tiled(true);
            window.configure_size(target_rect.size);
            self.remap_tracked_window(window, target_rect.loc, false);
        }
        self.schedule_render();
    }

    /// Toggle tiling and tile/untile all current windows.
    pub fn toggle_tiling(&mut self) {
        let now_active = self.workspaces.toggle();

        if now_active {
            // Group windows by their output
            let windows: Vec<_> = self.space.elements()
                .filter_map(|w| {
                    let s = w.get_wl_surface()?;
                    if self.fullscreen_windows.iter().any(|f| f.surface == s)
                        || self.scratchpad_surface.as_ref().is_some_and(|sc| sc == &s)
                    {
                        return None;
                    }
                    let output = self.output_for_window(w)
                        .or_else(|| self.workspaces.outputs_iter().next().cloned())?;
                    Some((s, output.name()))
                })
                .collect();

            for (surface, output_name) in windows {
                if !self.workspaces.contains(&surface) {
                    self.workspaces.insert(&output_name, surface, None);
                }
            }
            self.apply_tiling_layout();
            tracing::info!("Tiling enabled");
        } else {
            let surfaces: Vec<WlSurface> = self.space.elements()
                .filter_map(|w| w.get_wl_surface())
                .collect();
            for surface in surfaces {
                if let Some(window) = self.find_mapped_window(&surface) {
                    window.set_tiled(false);
                    window.send_pending_configure();
                }
            }
            tracing::info!("Tiling disabled — windows stay at current positions");
        }
    }
}

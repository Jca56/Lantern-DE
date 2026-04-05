/// BSP (binary space partition) tiling window layout.
///
/// Arena-based binary tree where each node is either a Split (H/V with ratio)
/// or a Leaf (holding a window surface). Layout is computed by recursively
/// subdividing a root rectangle with gaps between windows and at screen edges.

use smithay::{
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::{Logical, Point, Rectangle, Size},
};

use crate::state::Lantern;
use crate::window_ext::WindowExt;

const DEFAULT_GAP: i32 = 30;
const DEFAULT_OUTER_GAP: i32 = 30;

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
    next_direction: SplitDirection,
}

impl TilingState {
    pub fn new() -> Self {
        Self {
            nodes: Vec::new(),
            root: None,
            active: false,
            gap: DEFAULT_GAP,
            outer_gap: DEFAULT_OUTER_GAP,
            next_direction: SplitDirection::Horizontal,
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
        self.next_direction = SplitDirection::Horizontal;
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

    /// Insert a new surface into the tree. Splits the leaf containing `near`
    /// (or the last leaf, or creates the root if tree is empty).
    pub fn insert(&mut self, surface: WlSurface, near: Option<&WlSurface>) {
        let new_leaf = self.alloc(TilingNode::Leaf { surface });

        let Some(root) = self.root else {
            // First window: just set as root
            self.root = Some(new_leaf);
            return;
        };

        // Find the leaf to split next to
        let target = near
            .and_then(|s| self.find_leaf(s))
            .unwrap_or_else(|| self.last_leaf(root).unwrap_or(root));

        let dir = self.next_direction;
        self.next_direction = dir.opposite(); // alternate for next insert

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
            self.next_direction = SplitDirection::Horizontal;
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

        // Reset split direction when down to one window
        if self.leaf_count() <= 1 {
            self.next_direction = SplitDirection::Horizontal;
        }
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

    /// Find the last (rightmost/bottommost) leaf in a subtree.
    fn last_leaf(&self, idx: usize) -> Option<usize> {
        match &self.nodes[idx] {
            TilingNode::Leaf { .. } => Some(idx),
            TilingNode::Split { right, .. } => self.last_leaf(*right),
            TilingNode::Empty => None,
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

    /// Get the tiling area: output geometry minus exclusive zones, inset by outer gap.
    pub fn tiling_area(lantern: &Lantern) -> Option<Rectangle<i32, Logical>> {
        // Reuse the exclusive-zone-aware geometry used by maximize
        let dummy_window = lantern.space.elements().next()?;
        let mut area = lantern.window_output_geometry(dummy_window)?;
        let gap = lantern.tiling.outer_gap;
        area.loc.x += gap;
        area.loc.y += gap;
        area.size.w -= gap * 2;
        area.size.h -= gap * 2;
        Some(area)
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
    /// Recompute tiling layout and animate all tiled windows to their targets.
    pub fn apply_tiling_layout(&mut self) {
        if !self.tiling.active {
            return;
        }

        let Some(area) = TilingState::tiling_area(self) else { return };
        let layout = self.tiling.compute_layout(area);

        for (surface, target_rect) in layout {
            let Some(window) = self.find_mapped_window(&surface) else { continue };

            // Get current position for animation start
            let current_loc = self.space.element_location(&window).unwrap_or_default();
            let current_size = window.geometry().size;
            let current_rect = Rectangle::new(current_loc, current_size);

            // Start animation from current to target
            if current_rect != target_rect {
                self.tiling_anim.animate_to(&surface, current_rect, target_rect);
            }

            // Set tiled state so the client respects the configured size,
            // then configure window to final size and position.
            window.set_tiled(true);
            window.configure_size(target_rect.size);
            self.space.map_element(window, target_rect.loc, false);
        }

        self.schedule_render();
    }

    /// Toggle tiling and tile/untile all current windows.
    pub fn toggle_tiling(&mut self) {
        let now_active = self.tiling.toggle();

        if now_active {
            // Collect all mapped window surfaces (excluding minimized/fullscreen)
            let surfaces: Vec<WlSurface> = self.space.elements()
                .filter_map(|w| w.get_wl_surface())
                .filter(|s| {
                    !self.fullscreen_windows.iter().any(|f| f.surface == *s)
                        && !self.scratchpad_surface.as_ref().is_some_and(|sc| sc == s)
                })
                .collect();

            for surface in surfaces {
                if !self.tiling.contains(&surface) {
                    self.tiling.insert(surface, None);
                }
            }
            self.apply_tiling_layout();
            tracing::info!("Tiling enabled");
        } else {
            // Clear tiled state on all windows so they can freely resize again
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

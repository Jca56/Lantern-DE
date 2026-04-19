/// Server-side decorations (SSD) for windows that don't draw their own.
///
/// Integrated style: semi-transparent header overlay on the window's top region,
/// rounded corners via corner-mask shader elements, no gold accent line.

use smithay::{
    backend::renderer::{
        element::solid::{SolidColorBuffer, SolidColorRenderElement},
        gles::{element::PixelShaderElement, GlesPixelProgram, Uniform},
    },
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::{Logical, Physical, Point, Rectangle, Size},
};
use std::collections::HashMap;

use crate::snap::SnapZone;

// ── Constants ───────────────────────────────────────────────────────────────

/// Titlebar height in logical pixels (overlay on window top).
const BAR_HEIGHT: i32 = 34;
/// Window control button width.
const BTN_W: i32 = 46;
/// Corner radius for floating (non-tiled) windows.
pub const CORNER_RADIUS: f32 = 18.0;

fn phys_pt(x: i32, y: i32) -> Point<i32, Physical> {
    Point::from((x, y))
}

// ── Types ───────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SsdButton {
    Close,
    Maximize,
    Minimize,
}

/// Which corners of a window should be rounded.
#[derive(Clone, Copy, Debug)]
pub struct RoundedCorners {
    pub tl: bool,
    pub tr: bool,
    pub bl: bool,
    pub br: bool,
}

impl RoundedCorners {
    pub fn all() -> Self {
        Self { tl: true, tr: true, bl: true, br: true }
    }
    pub fn none() -> Self {
        Self { tl: false, tr: false, bl: false, br: false }
    }

    /// Snapped windows always sit inside tiling gaps (outer + inner), so every
    /// corner is floating in free space and should be rounded.
    pub fn for_snap(_zone: SnapZone) -> Self {
        Self::all()
    }
}

/// Per-window SSD state with pre-allocated color buffers.
pub struct SsdState {
    pub hovered_button: Option<SsdButton>,
    close_hover_buf: SolidColorBuffer,
    btn_hover_buf: SolidColorBuffer,
}

impl SsdState {
    fn new() -> Self {
        let close_hover: [f32; 4] = [0.91, 0.18, 0.18, 0.70]; // semi-transparent red
        let btn_hover: [f32; 4] = [1.0, 1.0, 1.0, 0.08];

        Self {
            hovered_button: None,
            close_hover_buf: SolidColorBuffer::new((1, 1), close_hover),
            btn_hover_buf: SolidColorBuffer::new((1, 1), btn_hover),
        }
    }
}

/// Manages SSD for all windows.
pub struct SsdManager {
    pub windows: HashMap<WlSurface, SsdState>,
}

impl SsdManager {
    pub fn new() -> Self {
        Self { windows: HashMap::new() }
    }

    pub fn add(&mut self, surface: WlSurface) {
        self.windows.entry(surface).or_insert_with(SsdState::new);
    }

    pub fn remove(&mut self, surface: &WlSurface) {
        self.windows.remove(surface);
    }

    pub fn has_ssd(&self, surface: &WlSurface) -> bool {
        self.windows.contains_key(surface)
    }

    pub fn get_mut(&mut self, surface: &WlSurface) -> Option<&mut SsdState> {
        self.windows.get_mut(surface)
    }

    pub fn bar_height() -> i32 {
        BAR_HEIGHT
    }
}

// ── Geometry helpers ────────────────────────────────────────────────────────

/// Titlebar rect — sits above the window (not overlapping content).
pub fn titlebar_rect(
    win_loc: Point<i32, Logical>,
    win_size: Size<i32, Logical>,
) -> Rectangle<i32, Logical> {
    Rectangle::new(
        Point::from((win_loc.x, win_loc.y - BAR_HEIGHT)),
        Size::from((win_size.w, BAR_HEIGHT)),
    )
}

pub fn button_rects(
    win_loc: Point<i32, Logical>,
    win_size: Size<i32, Logical>,
) -> (Rectangle<i32, Logical>, Rectangle<i32, Logical>, Rectangle<i32, Logical>) {
    let bar = titlebar_rect(win_loc, win_size);
    let mk = |idx: i32| Rectangle::new(
        Point::from((bar.loc.x + bar.size.w - BTN_W * (idx + 1), bar.loc.y)),
        Size::from((BTN_W, BAR_HEIGHT)),
    );
    (mk(0), mk(1), mk(2))
}

pub fn hit_test(
    point: Point<f64, Logical>,
    win_loc: Point<i32, Logical>,
    win_size: Size<i32, Logical>,
) -> Result<Option<SsdButton>, ()> {
    let bar = titlebar_rect(win_loc, win_size);
    let p = Point::from((point.x as i32, point.y as i32));
    if !bar.contains(p) {
        return Err(());
    }
    let (close, maximize, minimize) = button_rects(win_loc, win_size);
    if close.contains(p) {
        Ok(Some(SsdButton::Close))
    } else if maximize.contains(p) {
        Ok(Some(SsdButton::Maximize))
    } else if minimize.contains(p) {
        Ok(Some(SsdButton::Minimize))
    } else {
        Ok(None)
    }
}

// ── Rendering ───────────────────────────────────────────────────────────────

/// Render the SSD header overlay + button highlights.
/// Returns (solid_elements, shader_elements).
pub fn render_decoration(
    state: &mut SsdState,
    win_loc_phys: Point<i32, Physical>,
    win_loc_logical: Point<i32, Logical>,
    win_size: Size<i32, Logical>,
    scale: f64,
    icon_shader: Option<&GlesPixelProgram>,
    header_shader: Option<&GlesPixelProgram>,
    corners: RoundedCorners,
) -> (Vec<SolidColorRenderElement>, Vec<PixelShaderElement>) {
    let mut solids = Vec::with_capacity(2);
    let mut shaders = Vec::with_capacity(4);
    let kind = smithay::backend::renderer::element::Kind::Unspecified;

    let bar_w = win_size.w;
    let bar_h = BAR_HEIGHT;
    let phys_bar_h = (bar_h as f64 * scale).round() as i32;
    let bar_px = win_loc_phys.x;
    let bar_py = win_loc_phys.y - phys_bar_h; // bar sits above the window

    // Logical origin of the bar (above the window)
    let bar_lx = win_loc_logical.x;
    let bar_ly = win_loc_logical.y - bar_h;

    let p = |lx: i32, ly: i32| -> Point<i32, Physical> {
        phys_pt(
            bar_px + (lx as f64 * scale).round() as i32,
            bar_py + (ly as f64 * scale).round() as i32,
        )
    };

    // Header background via shader (semi-transparent with rounded top corners)
    if let Some(shader) = header_shader {
        let corner_r = if corners.tl || corners.tr {
            CORNER_RADIUS * scale as f32
        } else {
            0.0
        };

        let header_area = Rectangle::<i32, Logical>::new(
            Point::from((bar_lx, bar_ly)),
            Size::from((bar_w, bar_h)),
        );

        shaders.push(PixelShaderElement::new(
            shader.clone(),
            header_area,
            None,
            1.0,
            vec![
                Uniform::new("corner_radius", corner_r),
                Uniform::new("bar_color", [0.18f32, 0.18, 0.18, 0.75]),
            ],
            kind,
        ));
    }

    // Button hover highlight
    if let Some(btn) = state.hovered_button {
        let idx: i32 = match btn {
            SsdButton::Close => 0,
            SsdButton::Maximize => 1,
            SsdButton::Minimize => 2,
        };
        if btn == SsdButton::Close {
            // Close hover uses header shader so it respects the rounded top-right corner
            if let Some(shader) = header_shader {
                let btn_x = bar_lx + bar_w - BTN_W;
                let hover_r = if corners.tr { CORNER_RADIUS * scale as f32 } else { 0.0 };
                let hover_area = Rectangle::<i32, Logical>::new(
                    Point::from((btn_x, bar_ly)),
                    Size::from((BTN_W, bar_h)),
                );
                shaders.push(PixelShaderElement::new(
                    shader.clone(), hover_area, None, 1.0,
                    vec![
                        Uniform::new("corner_radius", hover_r),
                        Uniform::new("bar_color", [0.91f32, 0.18, 0.18, 0.70]),
                    ],
                    kind,
                ));
            }
        } else {
            state.btn_hover_buf.resize((BTN_W, bar_h));
            solids.push(SolidColorRenderElement::from_buffer(
                &state.btn_hover_buf, p(bar_w - BTN_W * (idx + 1), 0), scale, 1.0, kind,
            ));
        }
    }

    // Icons via pixel shader
    if let Some(shader) = icon_shader {
        let icon_rest: [f32; 4] = [0.86, 0.86, 0.86, 0.78];
        let icon_white: [f32; 4] = [1.0, 1.0, 1.0, 1.0];
        let is_close_hovered = state.hovered_button == Some(SsdButton::Close);

        for (idx, icon_type) in [(0, 0.0f32), (1, 1.0f32), (2, 2.0f32)] {
            let btn_x = bar_w - BTN_W * (idx + 1);
            let color = if idx == 0 && is_close_hovered { icon_white } else { icon_rest };

            let screen_area: Rectangle<i32, Logical> = Rectangle::new(
                Point::from((bar_lx + btn_x, bar_ly)),
                Size::from((BTN_W, bar_h)),
            );

            shaders.push(PixelShaderElement::new(
                shader.clone(),
                screen_area,
                None,
                1.0,
                vec![
                    Uniform::new("icon_type", icon_type),
                    Uniform::new("icon_color", color),
                ],
                kind,
            ));
        }
    }

    (solids, shaders)
}

/// Generate corner mask elements that clip the window to rounded corners.
/// Each mask is a `radius x radius` square at the relevant corner.
pub fn render_corner_masks(
    corner_shader: &GlesPixelProgram,
    win_loc_logical: Point<i32, Logical>,
    win_size: Size<i32, Logical>,
    scale: f64,
    alpha: f32,
    corners: RoundedCorners,
) -> Vec<PixelShaderElement> {
    let r = CORNER_RADIUS.ceil() as i32;
    let kind = smithay::backend::renderer::element::Kind::Unspecified;
    let phys_r = CORNER_RADIUS * scale as f32;
    let x = win_loc_logical.x;
    let y = win_loc_logical.y;
    let w = win_size.w;
    let h = win_size.h;

    let mut masks = Vec::with_capacity(4);

    let corner_cases: [(bool, i32, i32, f32, f32); 4] = [
        (corners.tl, x,         y,         0.0, 0.0), // top-left
        (corners.tr, x + w - r, y,         1.0, 0.0), // top-right
        (corners.bl, x,         y + h - r, 0.0, 1.0), // bottom-left
        (corners.br, x + w - r, y + h - r, 1.0, 1.0), // bottom-right
    ];

    for (enabled, cx, cy, corner_x, corner_y) in corner_cases {
        if !enabled { continue; }
        let area = Rectangle::<i32, Logical>::new(
            Point::from((cx, cy)),
            Size::from((r, r)),
        );
        masks.push(PixelShaderElement::new(
            corner_shader.clone(),
            area,
            None,
            alpha,
            vec![
                Uniform::new("corner_radius", phys_r),
                Uniform::new("corner", [corner_x, corner_y]),
            ],
            kind,
        ));
    }

    masks
}

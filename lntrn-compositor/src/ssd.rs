/// Server-side decorations (SSD) for windows that don't draw their own.
///
/// Minimal style: flat titlebar with gold accent line and proper vector icons
/// rendered via a GLSL pixel shader for crisp X/square/dash at any scale.

use smithay::{
    backend::renderer::{
        element::solid::{SolidColorBuffer, SolidColorRenderElement},
        gles::{element::PixelShaderElement, GlesPixelProgram, Uniform},
    },
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::{Logical, Physical, Point, Rectangle, Size},
};
use std::collections::HashMap;

// ── Constants ───────────────────────────────────────────────────────────────

/// Titlebar height in logical pixels.
const BAR_HEIGHT: i32 = 34;
/// Gold accent line thickness at the top.
const ACCENT_HEIGHT: i32 = 2;
/// Window control button width.
const BTN_W: i32 = 46;

fn color_srgb8(r: u8, g: u8, b: u8, a: f32) -> [f32; 4] {
    [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, a]
}

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

/// Per-window SSD state with pre-allocated color buffers.
pub struct SsdState {
    pub hovered_button: Option<SsdButton>,
    bar_buf: SolidColorBuffer,
    accent_buf: SolidColorBuffer,
    close_hover_buf: SolidColorBuffer,
    btn_hover_buf: SolidColorBuffer,
}

impl SsdState {
    fn new() -> Self {
        let bar_color = color_srgb8(39, 39, 39, 1.0);
        let accent_color = color_srgb8(200, 134, 10, 1.0);
        let close_hover = color_srgb8(232, 45, 45, 1.0);
        let btn_hover: [f32; 4] = [1.0, 1.0, 1.0, 0.06];

        Self {
            hovered_button: None,
            bar_buf: SolidColorBuffer::new((1, 1), bar_color),
            accent_buf: SolidColorBuffer::new((1, 1), accent_color),
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

/// Render elements for the SSD titlebar. Returns (solid_elements, shader_elements).
pub fn render_decoration(
    state: &mut SsdState,
    win_loc_phys: Point<i32, Physical>,
    win_loc_logical: Point<i32, Logical>,
    win_size: Size<i32, Logical>,
    scale: f64,
    icon_shader: Option<&GlesPixelProgram>,
) -> (Vec<SolidColorRenderElement>, Vec<PixelShaderElement>) {
    let mut solids = Vec::with_capacity(4);
    let mut shaders = Vec::with_capacity(3);
    let kind = smithay::backend::renderer::element::Kind::Unspecified;

    let bar_w = win_size.w;
    let bar_h = BAR_HEIGHT;
    // Compute physical bar height and position so bar bottom == window top (no gap)
    let phys_bar_h = (bar_h as f64 * scale).round() as i32;
    let bar_px = win_loc_phys.x;
    let bar_py = win_loc_phys.y - phys_bar_h;

    // Logical origin of the bar (for PixelShaderElement which uses logical coords)
    let bar_lx = win_loc_logical.x;
    let bar_ly = win_loc_logical.y - bar_h;

    let p = |lx: i32, ly: i32| -> Point<i32, Physical> {
        phys_pt(
            bar_px + (lx as f64 * scale).round() as i32,
            bar_py + (ly as f64 * scale).round() as i32,
        )
    };

    // Gold accent line at bottom of titlebar (highest Z)
    state.accent_buf.resize((bar_w, ACCENT_HEIGHT));
    let accent_py = bar_py + phys_bar_h - (ACCENT_HEIGHT as f64 * scale).round() as i32;
    solids.push(SolidColorRenderElement::from_buffer(
        &state.accent_buf, phys_pt(bar_px, accent_py), scale, 1.0, kind,
    ));

    // Button hover highlight
    if let Some(btn) = state.hovered_button {
        let idx: i32 = match btn {
            SsdButton::Close => 0,
            SsdButton::Maximize => 1,
            SsdButton::Minimize => 2,
        };
        let buf = if btn == SsdButton::Close {
            state.close_hover_buf.resize((BTN_W, bar_h));
            &state.close_hover_buf
        } else {
            state.btn_hover_buf.resize((BTN_W, bar_h));
            &state.btn_hover_buf
        };
        solids.push(SolidColorRenderElement::from_buffer(
            buf, p(bar_w - BTN_W * (idx + 1), 0), scale, 1.0, kind,
        ));
    }

    // Icons via pixel shader
    if let Some(shader) = icon_shader {
        let icon_rest = color_srgb8(220, 220, 220, 0.78);
        let icon_white: [f32; 4] = [1.0, 1.0, 1.0, 1.0];
        let is_close_hovered = state.hovered_button == Some(SsdButton::Close);

        for (idx, icon_type) in [(0, 0.0f32), (1, 1.0f32), (2, 2.0f32)] {
            let btn_x = bar_w - BTN_W * (idx + 1);
            let color = if idx == 0 && is_close_hovered { icon_white } else { icon_rest };

            // PixelShaderElement uses logical coordinates directly
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

    // Bar background (lowest Z)
    state.bar_buf.resize((bar_w, bar_h));
    solids.push(SolidColorRenderElement::from_buffer(
        &state.bar_buf, phys_pt(bar_px, bar_py), scale, 1.0, kind,
    ));

    (solids, shaders)
}

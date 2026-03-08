use lntrn_render::Color;

/// Fox Dark palette expressed as linear-space `Color` values.
pub struct FoxPalette {
    pub bg: Color,
    pub surface: Color,
    pub surface_2: Color,
    pub sidebar: Color,
    pub text: Color,
    pub text_secondary: Color,
    pub muted: Color,
    pub accent: Color,
    pub danger: Color,
    pub success: Color,
}

impl FoxPalette {
    pub fn dark() -> Self {
        Self {
            bg: Color::from_rgb8(24, 24, 24),
            surface: Color::from_rgb8(39, 39, 39),
            surface_2: Color::from_rgb8(51, 51, 51),
            sidebar: Color::from_rgb8(52, 52, 58),
            text: Color::from_rgb8(236, 236, 236),
            text_secondary: Color::from_rgb8(200, 200, 200),
            muted: Color::from_rgb8(144, 144, 144),
            accent: Color::from_rgb8(200, 134, 10),
            danger: Color::from_rgb8(239, 68, 68),
            success: Color::from_rgb8(34, 197, 94),
        }
    }

    pub fn light() -> Self {
        Self {
            bg: Color::from_rgb8(245, 245, 245),
            surface: Color::from_rgb8(234, 234, 234),
            surface_2: Color::from_rgb8(218, 218, 218),
            sidebar: Color::from_rgb8(238, 238, 242),
            text: Color::from_rgb8(30, 30, 30),
            text_secondary: Color::from_rgb8(80, 80, 80),
            muted: Color::from_rgb8(110, 110, 110),
            accent: Color::from_rgb8(200, 134, 10),
            danger: Color::from_rgb8(239, 68, 68),
            success: Color::from_rgb8(34, 197, 94),
        }
    }

    pub fn gradient_border_colors(&self) -> [Color; 4] {
        [
            Color::from_rgb8(170, 110, 8),
            Color::from_rgb8(200, 134, 10),
            Color::from_rgb8(220, 150, 15),
            Color::from_rgb8(250, 204, 21),
        ]
    }

    pub fn file_manager_gradient_stops(&self) -> [Color; 5] {
        [
            Color::from_rgb8(255, 105, 180),
            Color::from_rgb8(59, 130, 246),
            Color::from_rgb8(34, 197, 94),
            Color::from_rgb8(250, 204, 21),
            Color::from_rgb8(239, 68, 68),
        ]
    }
}

use lntrn_render::Color;

pub struct Theme {
    pub blob_colors: Vec<Color>,
    pub glass_tint: Color,
    pub glass_border: Color,
    pub base_color: Color,
    pub cap_color: Color,
    pub heat_glow: Color,
}

impl Theme {
    pub fn by_name(name: &str) -> Self {
        match name {
            "cosmic" => Self::cosmic(),
            "neon" => Self::neon(),
            "lofi" => Self::lofi(),
            _ => Self::classic(),
        }
    }

    pub fn classic() -> Self {
        Self {
            blob_colors: vec![
                Color::from_rgb8(220, 40, 40),   // ruby red
                Color::from_rgb8(240, 120, 20),  // sunset orange
                Color::from_rgb8(240, 200, 30),  // golden yellow
                Color::from_rgb8(200, 70, 30),   // warm amber
            ],
            glass_tint: Color::from_rgba8(10, 8, 20, 40),
            glass_border: Color::from_rgba8(255, 255, 255, 25),
            base_color: Color::from_rgb8(45, 40, 38),
            cap_color: Color::from_rgb8(50, 45, 42),
            heat_glow: Color::from_rgba8(255, 80, 20, 25),
        }
    }

    pub fn cosmic() -> Self {
        Self {
            blob_colors: vec![
                Color::from_rgb8(120, 40, 200),  // deep violet
                Color::from_rgb8(40, 100, 240),  // electric blue
                Color::from_rgb8(225, 175, 35),  // cosmic gold
                Color::from_rgb8(30, 180, 170),  // nebula teal
            ],
            glass_tint: Color::from_rgba8(12, 6, 30, 80),
            glass_border: Color::from_rgba8(140, 100, 255, 25),
            base_color: Color::from_rgb8(30, 20, 50),
            cap_color: Color::from_rgb8(35, 25, 55),
            heat_glow: Color::from_rgba8(100, 50, 255, 20),
        }
    }

    pub fn neon() -> Self {
        Self {
            blob_colors: vec![
                Color::from_rgb8(255, 20, 147),  // hot pink
                Color::from_rgb8(0, 255, 65),    // electric green
                Color::from_rgb8(0, 191, 255),   // cyber blue
                Color::from_rgb8(255, 255, 0),   // neon yellow
            ],
            glass_tint: Color::from_rgba8(5, 5, 15, 80),
            glass_border: Color::from_rgba8(100, 200, 255, 30),
            base_color: Color::from_rgb8(20, 20, 25),
            cap_color: Color::from_rgb8(25, 25, 30),
            heat_glow: Color::from_rgba8(0, 200, 255, 20),
        }
    }

    pub fn lofi() -> Self {
        Self {
            blob_colors: vec![
                Color::from_rgb8(200, 130, 140), // dusty rose
                Color::from_rgb8(140, 180, 140), // sage green
                Color::from_rgb8(160, 140, 190), // lavender
                Color::from_rgb8(220, 170, 130), // peach
            ],
            glass_tint: Color::from_rgba8(30, 28, 25, 80),
            glass_border: Color::from_rgba8(180, 170, 160, 25),
            base_color: Color::from_rgb8(60, 50, 42),
            cap_color: Color::from_rgb8(65, 55, 47),
            heat_glow: Color::from_rgba8(200, 150, 100, 18),
        }
    }
}

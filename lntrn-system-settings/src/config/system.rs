//! `[input]`, `[power]`, `[notifications]`, `[animations]` and what we
//! show of `[terminal]`. Choices
//! are the strings the compositor and the notification daemon read, not
//! enums, so the file keeps the words they already know.

use lntrn_props::props;

props! {
    /// Pointer, scrolling, clicking and the cursor.
    pub struct Input {
        /// libinput speed, -1 to 1.
        pub mouse_speed: f64 = 0.0 => { id: 1, hard: -1.0..=1.0, step: 0.05 },
        /// Adaptive (true) or flat (false) acceleration profile.
        pub pointer_acceleration: bool = true => { id: 2 },
        pub scroll_speed: f64 = 1.0 => { id: 3, hard: 0.25..=3.0, step: 0.05 },
        /// The file manager opens on a double click instead of one.
        pub double_click_to_open: bool = false => { id: 4 },
        pub cursor_size: i64 = 24 => { id: 5, hard: 16..=128 },
        /// `"default"` for the bundled cursor, else the stem of a file in
        /// `~/.lantern/config/cursors/`.
        pub cursor_theme: String = "default".to_owned() => { id: 6 },
        /// Recolour stops for the bundled cursor, hex.
        pub cursor_body_light: String = "#ffffff".to_owned() => { id: 7 },
        pub cursor_body_dark: String = "#ababab".to_owned() => { id: 8 },
        pub cursor_accent_light: String = "#fab414".to_owned() => { id: 9 },
        pub cursor_accent_dark: String = "#9a6300".to_owned() => { id: 10 },
        pub cursor_outline_color: String = "#0a0a0a".to_owned() => { id: 11 },
        /// Multiplier on the cursor's stroke widths; 0 hides outlines.
        pub cursor_outline_scale: f64 = 1.0 => { id: 12, hard: 0.0..=3.0, step: 0.1 },
        /// 0 = sharp tip, 1 = rounded pebble.
        pub cursor_corner_radius: f64 = 0.0 => { id: 13, hard: 0.0..=1.0, step: 0.05 },
        pub click_anim_enabled: bool = true => { id: 14 },
        pub click_anim_size: f64 = 1.0 => { id: 15, hard: 0.25..=3.0, step: 0.05 },
        /// Ripple colour, hex; empty follows the cursor outline.
        pub click_anim_color: String = String::new() => { id: 16 },
        /// `"rings"`; reserved for other styles.
        pub click_anim_style: String = "rings".to_owned() => { id: 17 },
    }
}

impl Input {
    pub fn clamp(&mut self) {
        self.mouse_speed = self.mouse_speed.clamp(-1.0, 1.0);
        self.scroll_speed = self.scroll_speed.clamp(0.25, 3.0);
        self.cursor_size = self.cursor_size.clamp(16, 128);
        self.cursor_outline_scale = self.cursor_outline_scale.clamp(0.0, 3.0);
        self.cursor_corner_radius = self.cursor_corner_radius.clamp(0.0, 1.0);
        self.click_anim_size = self.click_anim_size.clamp(0.25, 3.0);
        if self.cursor_theme.trim().is_empty() {
            self.cursor_theme = "default".to_owned();
        }
    }
}

props! {
    /// Lid, idle and battery behaviour.
    pub struct Power {
        /// `suspend`, `hibernate`, `lock` or `nothing`.
        pub lid_close_action: String = "suspend".to_owned() => { id: 1 },
        pub lid_close_on_ac: String = "lock".to_owned() => { id: 2 },
        /// Seconds before the screen dims; 0 never.
        pub dim_after: i64 = 120 => { id: 3, hard: 0..=1800 },
        /// Seconds before the idle action.
        pub idle_timeout: i64 = 300 => { id: 4, hard: 60..=3600 },
        /// `suspend`, `lock` or `nothing`.
        pub idle_action: String = "suspend".to_owned() => { id: 5 },
        pub low_battery_threshold: i64 = 15 => { id: 6, hard: 5..=50 },
        pub critical_battery_threshold: i64 = 5 => { id: 7, hard: 1..=20 },
        /// `suspend`, `hibernate`, `shutdown` or `nothing`.
        pub critical_battery_action: String = "hibernate".to_owned() => { id: 8 },
    }
}

impl Power {
    pub fn clamp(&mut self) {
        self.dim_after = self.dim_after.clamp(0, 1800);
        self.idle_timeout = self.idle_timeout.clamp(60, 3600);
        self.low_battery_threshold = self.low_battery_threshold.clamp(5, 50);
        self.critical_battery_threshold = self.critical_battery_threshold.clamp(1, 20);
    }
}

props! {
    /// Toasts and their sound.
    pub struct Notifications {
        pub do_not_disturb: bool = false => { id: 1 },
        pub show_toasts: bool = true => { id: 2 },
        pub play_sound: bool = true => { id: 3 },
        pub volume: f64 = 0.8 => { id: 4, hard: 0.0..=1.0 },
        pub default_duration_secs: f64 = 5.0 => { id: 5, hard: 1.0..=30.0 },
        /// `top-right`, `top-left`, `bottom-right` or `bottom-left`.
        pub position: String = "top-right".to_owned() => { id: 6 },
    }
}

pub const NOTIFICATION_POSITIONS: [&str; 4] = ["top-right", "top-left", "bottom-right", "bottom-left"];

impl Notifications {
    pub fn clamp(&mut self) {
        self.volume = self.volume.clamp(0.0, 1.0);
        self.default_duration_secs = self.default_duration_secs.clamp(1.0, 30.0);
        if !NOTIFICATION_POSITIONS.contains(&self.position.as_str()) {
            self.position = "top-right".to_owned();
        }
    }
}

props! {
    /// Window animations.
    pub struct Animations {
        /// Off, every animation completes at once.
        pub enabled: bool = true => { id: 1 },
        /// Speed multiplier; 1 is the stock pace.
        pub speed: f64 = 1.0 => { id: 2, hard: 0.25..=3.0, step: 0.05 },
        /// `cinematic`, `snappy`, `springy` or `linear`.
        pub preset: String = "cinematic".to_owned() => { id: 3 },
        pub open_close: bool = true => { id: 4 },
        pub state: bool = true => { id: 5 },
        pub minimize: bool = true => { id: 6 },
        pub tiling: bool = true => { id: 7 },
        pub workspace: bool = true => { id: 8 },
    }
}

pub const ANIMATION_PRESETS: [&str; 4] = ["cinematic", "snappy", "springy", "linear"];

impl Animations {
    pub fn clamp(&mut self) {
        self.speed = self.speed.clamp(0.25, 3.0);
        if !ANIMATION_PRESETS.contains(&self.preset.as_str()) {
            self.preset = "cinematic".to_owned();
        }
    }
}

props! {
    /// The terminal, as far as this app goes. The rest of `[terminal]`
    /// (the tabs it has pinned) is the terminal's own and is left alone.
    pub struct Terminal {
        /// Text size, logical pixels.
        pub font_size: f64 = 20.0 => { id: 1, hard: 8.0..=40.0, step: 0.5 },
        /// `block`, `underline` or `beam`: the cursor until a program
        /// asks for another.
        pub cursor_style: String = "block".to_owned() => { id: 2 },
        /// Open with the title bar and the tab strips hidden.
        pub open_bar_hidden: bool = false => { id: 3 },
    }
}

pub const TERMINAL_FONT_SIZES: (f64, f64) = (8.0, 40.0);
pub const CURSOR_STYLES: [&str; 3] = ["block", "underline", "beam"];

impl Terminal {
    pub fn clamp(&mut self) {
        self.font_size = self.font_size.clamp(TERMINAL_FONT_SIZES.0, TERMINAL_FONT_SIZES.1);
        if !CURSOR_STYLES.contains(&self.cursor_style.as_str()) {
            self.cursor_style = "block".to_owned();
        }
    }

    /// What the terminal's old file of its own (`terminal.toml`) said:
    /// what it is set to until it has a section in `lantern.toml`, which
    /// the terminal writes the first time it runs. Without this, saving
    /// here before then would put the defaults in its way.
    pub fn from_old_file(doc: &lntrn_data::Doc) -> Terminal {
        use lntrn_data::Doc;
        let d = Terminal::default();
        let mut t = Terminal {
            font_size: doc.path("font.size").and_then(Doc::as_f64).unwrap_or(d.font_size),
            cursor_style: doc.path("general.cursor_style").and_then(Doc::as_str).map_or(d.cursor_style, str::to_owned),
            open_bar_hidden: doc.path("general.open_chrome_hidden").and_then(Doc::as_bool).unwrap_or(false),
        };
        t.clamp();
        t
    }
}

//! New repository view — create a local repo, optionally on GitHub too.

use std::path::PathBuf;

use lntrn_render::{Painter, Rect, TextRenderer};
use lntrn_ui::gpu::{Checkbox, FoxPalette, InteractionContext, TextInput};

use crate::keys;

// Zone IDs (must not collide with app.rs / clone.rs zones)
const ZONE_BACK_BTN: u32 = 4000;
const ZONE_NAME_INPUT: u32 = 4001;
const ZONE_PATH_INPUT: u32 = 4002;
const ZONE_GITHUB_CB: u32 = 4003;
const ZONE_PRIVATE_CB: u32 = 4004;
const ZONE_CREATE_BTN: u32 = 4005;

#[derive(PartialEq, Clone, Copy)]
enum Field {
    None,
    Name,
    Path,
}

pub struct NewRepoView {
    pub name: String,
    pub parent_path: String,
    pub create_on_github: bool,
    pub private: bool,
    pub creating: bool,
    pub error: Option<String>,
    focused: Field,
    name_cursor: usize,
    path_cursor: usize,
}

/// Result of a new-repo view action — tells the parent App what happened.
pub enum NewRepoAction {
    None,
    GoBack,
    Create {
        name: String,
        parent: PathBuf,
        github: bool,
        private: bool,
    },
}

impl NewRepoView {
    pub fn new() -> Self {
        Self {
            name: String::new(),
            parent_path: default_parent_path(),
            create_on_github: false,
            private: true,
            creating: false,
            error: None,
            focused: Field::Name,
            name_cursor: 0,
            path_cursor: 0,
        }
    }

    /// Clear the form for the next visit (after a successful create).
    pub fn reset(&mut self) {
        *self = Self::new();
    }

    pub fn wants_keyboard(&self) -> bool {
        self.focused != Field::None
    }

    fn create_action(&mut self) -> NewRepoAction {
        if self.creating {
            return NewRepoAction::None;
        }
        let name = self.name.trim().to_string();
        if name.is_empty() {
            self.error = Some("Enter a repository name".into());
            return NewRepoAction::None;
        }
        self.error = None;
        self.creating = true;
        NewRepoAction::Create {
            name,
            parent: PathBuf::from(self.parent_path.trim()),
            github: self.create_on_github,
            private: self.private,
        }
    }

    pub fn on_click(&mut self, ix: &InteractionContext, px: f32, py: f32) -> NewRepoAction {
        let Some(zone) = ix.zone_at(px, py) else {
            self.focused = Field::None;
            return NewRepoAction::None;
        };

        match zone {
            ZONE_BACK_BTN => return NewRepoAction::GoBack,
            ZONE_NAME_INPUT => {
                self.focused = Field::Name;
                self.name_cursor = self.name.len();
            }
            ZONE_PATH_INPUT => {
                self.focused = Field::Path;
                self.path_cursor = self.parent_path.len();
            }
            ZONE_GITHUB_CB => {
                self.create_on_github = !self.create_on_github;
            }
            ZONE_PRIVATE_CB => {
                self.private = !self.private;
            }
            ZONE_CREATE_BTN => return self.create_action(),
            _ => {
                self.focused = Field::None;
            }
        }
        NewRepoAction::None
    }

    pub fn on_key(&mut self, key: u32, shift: bool) -> NewRepoAction {
        let (buf, cursor) = match self.focused {
            Field::Name => (&mut self.name, &mut self.name_cursor),
            Field::Path => (&mut self.parent_path, &mut self.path_cursor),
            Field::None => return NewRepoAction::None,
        };

        match key {
            keys::KEY_ESC => {
                self.focused = Field::None;
            }
            keys::KEY_TAB => {
                self.focused = if self.focused == Field::Name {
                    Field::Path
                } else {
                    Field::Name
                };
                self.name_cursor = self.name.len();
                self.path_cursor = self.parent_path.len();
            }
            keys::KEY_ENTER => return self.create_action(),
            keys::KEY_BACKSPACE => {
                if *cursor > 0 {
                    *cursor -= 1;
                    buf.remove(*cursor);
                }
            }
            keys::KEY_LEFT => {
                if *cursor > 0 {
                    *cursor -= 1;
                }
            }
            keys::KEY_RIGHT => {
                if *cursor < buf.len() {
                    *cursor += 1;
                }
            }
            _ => {
                if let Some(ch) = keys::keycode_to_char(key, shift) {
                    // Repo names can't contain path separators.
                    if self.focused == Field::Name && ch == '/' {
                        return NewRepoAction::None;
                    }
                    buf.insert(*cursor, ch);
                    *cursor += 1;
                }
            }
        }
        NewRepoAction::None
    }

    pub fn draw(
        &mut self,
        painter: &mut Painter,
        text: &mut TextRenderer,
        ix: &mut InteractionContext,
        palette: &FoxPalette,
        cx: f32,
        cy: f32,
        cw: f32,
        _ch: f32,
        s: f32,
        sw: u32,
        sh: u32,
    ) {
        let title_font = 28.0 * s;
        let body_font = 22.0 * s;
        let small_font = 18.0 * s;
        let pad = 20.0 * s;
        let input_h = 44.0 * s;
        let btn_h = 44.0 * s;

        let mut y = cy;

        // Back button
        let back_w = 40.0 * s;
        let back_rect = Rect::new(cx + pad, y, back_w, title_font + 8.0 * s);
        let back_state = ix.add_zone(ZONE_BACK_BTN, back_rect);
        if back_state.is_hovered() {
            painter.rect_filled(back_rect, 6.0 * s, palette.muted.with_alpha(0.2));
        }
        text.queue(
            "←",
            title_font,
            cx + pad + 6.0 * s,
            y,
            palette.accent,
            back_w,
            sw,
            sh,
        );

        // Title
        text.queue(
            "New Repository",
            title_font,
            cx + pad + back_w + 8.0 * s,
            y,
            palette.text,
            cw,
            sw,
            sh,
        );
        y += title_font + 16.0 * s;

        // Error
        if let Some(ref err) = self.error {
            text.queue(
                err,
                small_font,
                cx + pad,
                y,
                palette.danger,
                cw - pad * 2.0,
                sw,
                sh,
            );
            y += small_font + 8.0 * s;
        }

        let input_w = (cw - pad * 2.0).min(640.0 * s);

        // Name
        text.queue("Name:", body_font, cx + pad, y, palette.text, cw, sw, sh);
        y += body_font + 8.0 * s;
        let name_rect = Rect::new(cx + pad, y, input_w, input_h);
        ix.add_zone(ZONE_NAME_INPUT, name_rect);
        TextInput::new(name_rect)
            .text(&self.name)
            .placeholder("my-cool-project")
            .focused(self.focused == Field::Name)
            .scale(s)
            .cursor_pos(self.name_cursor)
            .draw(painter, text, palette, sw, sh);
        y += input_h + 16.0 * s;

        // Parent directory
        text.queue(
            "Create in:",
            body_font,
            cx + pad,
            y,
            palette.text,
            cw,
            sw,
            sh,
        );
        y += body_font + 8.0 * s;
        let path_rect = Rect::new(cx + pad, y, input_w, input_h);
        ix.add_zone(ZONE_PATH_INPUT, path_rect);
        TextInput::new(path_rect)
            .text(&self.parent_path)
            .placeholder("~/Projects")
            .focused(self.focused == Field::Path)
            .scale(s)
            .cursor_pos(self.path_cursor)
            .draw(painter, text, palette, sw, sh);
        y += input_h + 10.0 * s;

        // Full path preview
        if !self.name.trim().is_empty() {
            let preview = format!(
                "Will create: {}/{}",
                self.parent_path.trim_end_matches('/'),
                self.name.trim()
            );
            text.queue(
                &preview,
                small_font,
                cx + pad,
                y,
                palette.muted,
                cw - pad * 2.0,
                sw,
                sh,
            );
        }
        y += small_font + 20.0 * s;

        // GitHub checkbox
        let cb_h = 36.0 * s;
        let gh_rect = Rect::new(cx + pad, y, 420.0 * s, cb_h);
        let gh_state = ix.add_zone(ZONE_GITHUB_CB, gh_rect);
        Checkbox::new(gh_rect, self.create_on_github)
            .label("Also create on GitHub")
            .hovered(gh_state.is_hovered())
            .scale(s)
            .draw(painter, text, palette, sw, sh);
        y += cb_h + 12.0 * s;

        // Private/public sub-option (only meaningful with GitHub)
        if self.create_on_github {
            let pv_rect = Rect::new(cx + pad + 40.0 * s, y, 380.0 * s, cb_h);
            let pv_state = ix.add_zone(ZONE_PRIVATE_CB, pv_rect);
            Checkbox::new(pv_rect, self.private)
                .label("Private repository")
                .hovered(pv_state.is_hovered())
                .scale(s)
                .draw(painter, text, palette, sw, sh);
            y += cb_h + 12.0 * s;
        }

        y += 12.0 * s;

        // Create button
        let btn_w = 160.0 * s;
        let create_rect = Rect::new(cx + pad, y, btn_w, btn_h);
        let create_state = ix.add_zone(ZONE_CREATE_BTN, create_rect);
        let btn_color = if self.creating || self.name.trim().is_empty() {
            palette.muted.with_alpha(0.3)
        } else if create_state.is_hovered() {
            palette.accent
        } else {
            palette.accent.with_alpha(0.8)
        };
        painter.rect_filled(create_rect, 8.0 * s, btn_color);
        let label = if self.creating {
            "Creating…"
        } else {
            "Create"
        };
        let tw = text.measure_width(label, body_font);
        text.queue(
            label,
            body_font,
            create_rect.x + (btn_w - tw) / 2.0,
            create_rect.y + (btn_h - body_font) / 2.0,
            palette.text,
            btn_w,
            sw,
            sh,
        );
    }
}

fn default_parent_path() -> String {
    let home = std::env::var("HOME").unwrap_or_default();
    format!("{home}/Projects")
}

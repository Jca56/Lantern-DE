//! The app's own preferences (beside the shell's): the code font, tabs,
//! and the syntax colors. A `props!` struct, so the Preferences editor
//! draws it, and it saves as tagged bytes next to the shell's file.

use lntrn_math::Color;
use lntrn_props::props;
use lntrn_ui::persist;

use crate::syntax::TokenKind;

props! {
    /// Colors of the code by what a token is.
    pub struct SyntaxColors {
        /// Code that is no token in particular; the editor's ink, apart from the theme's.
        pub text: Color = Color::hex(0xF2F2F4) => { id: 13, label: "Plain Text" },
        pub keyword: Color = Color::hex(0xC792EA) => { id: 1 },
        pub types: Color = Color::hex(0xFFCB6B) => { id: 2, label: "Types" },
        pub function: Color = Color::hex(0x82AAFF) => { id: 3 },
        pub string: Color = Color::hex(0xC3E88D) => { id: 4 },
        pub number: Color = Color::hex(0xF78C6C) => { id: 5 },
        pub comment: Color = Color::hex(0x6B7A8F) => { id: 6 },
        pub punct: Color = Color::hex(0x9AA7B5) => { id: 7, label: "Punctuation" },
        pub attribute: Color = Color::hex(0x89DDFF) => { id: 8 },
        pub constant: Color = Color::hex(0xFF8A80) => { id: 9 },
        pub heading: Color = Color::hex(0xFFB733) => { id: 10 },
        pub emphasis: Color = Color::hex(0xE6C08A) => { id: 11 },
        pub link: Color = Color::hex(0x6EA6FF) => { id: 12 },
    }
}

impl SyntaxColors {
    /// The color of a token kind.
    pub fn of(&self, kind: TokenKind) -> Color {
        match kind {
            TokenKind::Text => self.text,
            TokenKind::Keyword => self.keyword,
            TokenKind::Type => self.types,
            TokenKind::Function => self.function,
            TokenKind::String | TokenKind::Code => self.string,
            TokenKind::Number => self.number,
            TokenKind::Comment => self.comment,
            TokenKind::Punct | TokenKind::Operator => self.punct,
            TokenKind::Attribute => self.attribute,
            TokenKind::Constant => self.constant,
            TokenKind::Heading => self.heading,
            TokenKind::Emphasis => self.emphasis,
            TokenKind::Link => self.link,
        }
    }
}

props! {
    /// Colors of the gutter marks and file-tree dots for what git says.
    pub struct GitColors {
        pub added: Color = Color::hex(0x8BD17C) => { id: 1 },
        pub modified: Color = Color::hex(0x6EA6FF) => { id: 2 },
        pub deleted: Color = Color::hex(0xE0473A) => { id: 3 },
    }
}

props! {
    /// Editor preferences.
    pub struct Settings {
        /// Monospace family for code and the terminal; empty for the default.
        pub font_family: String = "JetBrains Mono".to_owned() => { id: 1, label: "Code Font" },
        /// Code text size in logical pixels.
        pub font_size: f64 = 20.0 => { id: 2, hard: 8.0..=64.0, step: 1.0, subtype: Pixels },
        /// Cells per tab stop.
        pub tab_width: i64 = 4 => { id: 3, hard: 1..=16 },
        /// Tab inserts spaces up to the next stop.
        pub insert_spaces: bool = true => { id: 4 },
        /// Tint the line the caret is on.
        pub highlight_line: bool = true => { id: 5 },
        /// Strip trailing spaces from every line when saving.
        pub trim_on_save: bool = false => { id: 6 },
        /// Lines of terminal output kept above the screen.
        pub scrollback: i64 = 5000 => { id: 7, hard: 100..=100000, step: 100.0 },
        /// Have the language server format the file on every save.
        pub format_on_save: bool = false => { id: 8 },
        pub colors: SyntaxColors = SyntaxColors::default() => { id: 10, label: "Syntax Colors" },
        pub git: GitColors = GitColors::default() => { id: 11, label: "Git Colors" },
        pub terminal: TerminalColors = TerminalColors::default() => { id: 14, label: "Terminal" },
        /// How big the side panels (Files, Git, Search, Problems) draw,
        /// relative to the UI scale: smaller keeps them thin beside big code.
        pub panel_scale: f64 = 0.8 => { id: 15, hard: 0.5..=1.5, step: 0.05, subtype: Factor },
        /// Soft-wrap Markdown and plain text at the view's width.
        pub wrap_prose: bool = true => { id: 16, label: "Wrap Prose" },
    }
}

props! {
    /// The terminal's own well and ink, apart from the theme's fields.
    pub struct TerminalColors {
        pub background: Color = Color::hex(0x131315) => { id: 1 },
        pub text: Color = Color::hex(0xF2F2F4) => { id: 2 },
    }
}

const FILE: &str = "settings.bin";

impl Settings {
    pub fn tab(&self) -> usize {
        self.tab_width.clamp(1, 16) as usize
    }

    pub fn load(app_id: &str) -> Self {
        let mut s = Self::default();
        if let Some(dir) = persist::config_dir(app_id) {
            persist::load(&dir.join(FILE), &mut s);
        }
        s
    }

    pub fn save(&self, app_id: &str) {
        if let Some(dir) = persist::config_dir(app_id)
            && let Err(e) = persist::save(&dir.join(FILE), self)
        {
            lntrn_core::log_error!("saving settings: {e}");
        }
    }
}

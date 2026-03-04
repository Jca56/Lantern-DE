use eframe::egui;

use crate::theme::MenuTheme;

// ── Menu item types ──────────────────────────────────────────────────────────

/// Style hint for a menu item — controls text color.
#[derive(Clone, Copy, PartialEq)]
pub enum ItemStyle {
    Normal,
    Accent,
    Danger,
}

/// A single item in a context menu.
pub struct MenuItem {
    pub label: String,
    pub style: ItemStyle,
    pub enabled: bool,
}

impl MenuItem {
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            style: ItemStyle::Normal,
            enabled: true,
        }
    }

    pub fn accent(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            style: ItemStyle::Accent,
            enabled: true,
        }
    }

    pub fn danger(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            style: ItemStyle::Danger,
            enabled: true,
        }
    }

    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }
}

// ── Context menu rendering ───────────────────────────────────────────────────

/// Show a themed context menu on a response (right-click popup).
///
/// The closure receives a `ContextMenuUi` which provides methods to add
/// items, separators, and submenus with consistent Lantern styling.
///
/// Returns `true` if any item was clicked (menu will auto-close).
///
/// # Example
/// ```ignore
/// let response = ui.allocate_response(size, egui::Sense::click());
/// lantern_context_menu::show(&response, &theme, |menu| {
///     if menu.item("Copy").clicked() { /* ... */ }
///     if menu.item("Cut").clicked() { /* ... */ }
///     menu.separator();
///     if menu.danger_item("Delete").clicked() { /* ... */ }
/// });
/// ```
pub fn show(
    response: &egui::Response,
    theme: &MenuTheme,
    add_contents: impl FnOnce(&mut ContextMenuUi),
) {
    response.context_menu(|ui| {
        let mut menu_ui = ContextMenuUi::new(ui, theme);
        add_contents(&mut menu_ui);
    });
}

/// Styled context menu builder. Wraps an egui `Ui` and applies Lantern theming
/// to every item automatically.
pub struct ContextMenuUi<'a> {
    ui: &'a mut egui::Ui,
    theme: &'a MenuTheme,
}

impl<'a> ContextMenuUi<'a> {
    pub fn new(ui: &'a mut egui::Ui, theme: &'a MenuTheme) -> Self {
        ui.set_min_width(theme.min_width);
        ui.spacing_mut().item_spacing.y = theme.item_spacing;
        Self { ui, theme }
    }

    /// Add a normal menu item. Returns an `ItemResponse` you can check `.clicked()` on.
    pub fn item(&mut self, label: impl Into<String>) -> ItemResponse {
        self.styled_item(MenuItem::new(label))
    }

    /// Add an accent-colored menu item (gold highlight).
    pub fn accent_item(&mut self, label: impl Into<String>) -> ItemResponse {
        self.styled_item(MenuItem::accent(label))
    }

    /// Add a danger-colored menu item (red, for destructive actions).
    pub fn danger_item(&mut self, label: impl Into<String>) -> ItemResponse {
        self.styled_item(MenuItem::danger(label))
    }

    /// Add a menu item with a custom `MenuItem` config.
    pub fn styled_item(&mut self, item: MenuItem) -> ItemResponse {
        let color = match item.style {
            ItemStyle::Normal => self.theme.text,
            ItemStyle::Accent => self.theme.accent,
            ItemStyle::Danger => self.theme.danger,
        };
        let text = egui::RichText::new(&item.label)
            .size(self.theme.font_size)
            .color(color);

        let response = if item.enabled {
            self.ui.button(text)
        } else {
            self.ui.add_enabled(false, egui::Button::new(text))
        };

        let clicked = response.clicked();
        if clicked {
            self.ui.close();
        }

        ItemResponse { clicked }
    }

    /// Add a visual separator line between groups.
    pub fn separator(&mut self) {
        self.ui.separator();
    }

    /// Add a non-interactive label (e.g. a group heading).
    pub fn label(&mut self, text: impl Into<String>) {
        self.ui.label(
            egui::RichText::new(text.into())
                .size(self.theme.font_size)
                .color(self.theme.text_secondary),
        );
    }

    /// Add a submenu that opens on hover. The closure receives a nested
    /// `ContextMenuUi` with the same theme.
    pub fn submenu(
        &mut self,
        label: impl Into<String>,
        add_contents: impl FnOnce(&mut ContextMenuUi),
    ) {
        let text = egui::RichText::new(label.into())
            .size(self.theme.font_size)
            .color(self.theme.text);
        let theme = self.theme;
        self.ui.menu_button(text, |ui| {
            let mut sub = ContextMenuUi::new(ui, theme);
            add_contents(&mut sub);
        });
    }

    /// Access the raw egui `Ui` for custom rendering (e.g. inline color swatches).
    pub fn raw_ui(&mut self) -> &mut egui::Ui {
        self.ui
    }
}

/// Result of rendering a menu item.
pub struct ItemResponse {
    clicked: bool,
}

impl ItemResponse {
    pub fn clicked(&self) -> bool {
        self.clicked
    }
}

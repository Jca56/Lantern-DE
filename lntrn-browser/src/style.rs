use gtk::prelude::*;

/// Apply custom CSS matching Lantern DE Fox Dark palette.
pub fn apply_css() {
    let css = gtk::CssProvider::new();
    let _ = css.load_from_data(CSS.as_bytes());
    gtk::StyleContext::add_provider_for_screen(
        &gdk::Screen::default().expect("No screen"),
        &css,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
}

const CSS: &str = r#"
window {
    background-color: rgb(24, 24, 24);
}

.bars-container {
    background: transparent;
}

.toolbar {
    background-color: rgba(30, 30, 30, 0.95);
    padding: 4px 8px;
    border-bottom: 1px solid rgb(20, 20, 20);
}

.nav-btn {
    background: none;
    border: none;
    border-radius: 6px;
    color: rgb(200, 200, 200);
    font-size: 18px;
    min-width: 32px;
    min-height: 32px;
    padding: 0;
}

.nav-btn:hover {
    background-color: rgb(51, 51, 51);
}

.tab-btn {
    background: none;
    border: none;
    border-radius: 6px;
    color: rgb(144, 144, 144);
    font-size: 13px;
    padding: 4px 12px;
    margin: 0 1px;
    min-height: 26px;
}

.tab-btn:hover {
    background-color: rgb(51, 51, 51);
    color: rgb(236, 236, 236);
}

.tab-btn.active {
    background-color: rgb(51, 51, 51);
    color: rgb(236, 236, 236);
}

.tab-close {
    background: none;
    border: none;
    color: rgb(100, 100, 100);
    font-size: 11px;
    min-width: 16px;
    min-height: 16px;
    padding: 0;
    margin-left: 2px;
    border-radius: 6px;
}

.tab-close:hover {
    background-color: rgb(255, 100, 100);
    color: white;
}

.new-tab-btn {
    background: none;
    border: none;
    color: rgb(144, 144, 144);
    font-size: 16px;
    min-width: 28px;
    min-height: 28px;
    padding: 0;
    border-radius: 6px;
}

.new-tab-btn:hover {
    background-color: rgb(51, 51, 51);
    color: rgb(34, 197, 94);
}

.url-entry {
    background-color: rgb(24, 24, 24);
    border: 1px solid rgb(51, 51, 51);
    border-radius: 8px;
    color: rgb(236, 236, 236);
    font-size: 14px;
    padding: 4px 12px;
    min-height: 28px;
}

.url-entry:focus {
    border-color: rgb(34, 197, 94);
}

button.window-control,
button.window-control:hover,
button.window-control:active,
button.window-control:checked,
button.window-control:backdrop {
    background-image: none;
    background-color: transparent;
    border: none;
    border-radius: 0;
    box-shadow: none;
    outline: none;
    padding: 0;
    margin: 0;
    min-width: 36px;
    min-height: 36px;
    color: rgb(160, 160, 160);
    font-size: 13px;
}

button.window-control:hover {
    background-color: rgb(51, 51, 51);
    color: rgb(236, 236, 236);
}

button.window-control.close:hover {
    background-color: rgb(255, 100, 100);
    color: white;
}
"#;

mod passwords;
mod style;
mod tabs;

use gtk::prelude::*;
use webkit2gtk::{
    WebContext, WebView, WebViewExt, WebContextExt,
    CookiePersistentStorage, LoadEvent,
    HardwareAccelerationPolicy, SettingsExt,
};
use webkit2gtk::CookieManagerExt;

use passwords::PasswordStore;
use tabs::TabBar;
use std::cell::RefCell;
use std::rc::Rc;

const HOME_URL: &str = "https://www.google.com";
const BAR_H: i32 = 40;

fn main() {
    // Disable WebKitGTK GL compositing — causes tearing on Intel Arc
    std::env::set_var("WEBKIT_DISABLE_COMPOSITING_MODE", "1");

    gtk::init().expect("Failed to init GTK");
    style::apply_css();

    let window = gtk::Window::new(gtk::WindowType::Toplevel);
    window.set_title("Lantern Browser");
    window.set_default_size(1200, 800);
    window.set_decorated(false);
    window.set_app_paintable(true);

    if let Some(screen) = gtk::prelude::WidgetExt::screen(&window) {
        if let Some(visual) = screen.rgba_visual() {
            window.set_visual(Some(&visual));
        }
    }

    // ── Overlay: bars float over the webview ────────────────────────────────
    let overlay = gtk::Overlay::new();

    // ── WebView (base layer) ────────────────────────────────────────────────
    let web_context = WebContext::default().unwrap();
    let data_dir = dirs_data();
    std::fs::create_dir_all(&data_dir).ok();
    let cookie_path = format!("{}/cookies.sqlite", data_dir);
    if let Some(cookie_mgr) = web_context.cookie_manager() {
        cookie_mgr.set_persistent_storage(&cookie_path, CookiePersistentStorage::Sqlite);
    }

    let webview = WebView::with_context(&web_context);
    webview.set_vexpand(true);
    webview.set_hexpand(true);

    // Enable hardware acceleration for smooth video playback
    if let Some(settings) = WebViewExt::settings(&webview) {
        settings.set_hardware_acceleration_policy(HardwareAccelerationPolicy::Never);
        settings.set_enable_smooth_scrolling(true);
    }

    overlay.add(&webview);
    webview.load_uri(HOME_URL);

    // ── Bars container (overlay, top-aligned) ───────────────────────────────
    let bars = gtk::Box::new(gtk::Orientation::Vertical, 0);
    bars.set_valign(gtk::Align::Start);
    bars.set_hexpand(true);
    bars.style_context().add_class("bars-container");

    // ── Nav bar: [← →] [URL] [─ □ ✕] ───────────────────────────────────────
    let nav_bar = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    nav_bar.set_size_request(-1, BAR_H);
    nav_bar.style_context().add_class("toolbar");

    let btn_back = gtk::Button::with_label("\u{2190}");
    btn_back.style_context().add_class("nav-btn");
    let btn_fwd = gtk::Button::with_label("\u{2192}");
    btn_fwd.style_context().add_class("nav-btn");

    let url_entry = gtk::Entry::new();
    url_entry.set_hexpand(true);
    url_entry.set_placeholder_text(Some("Search or enter URL"));
    url_entry.style_context().add_class("url-entry");
    url_entry.set_text(HOME_URL);

    let btn_min = gtk::Button::with_label("\u{2500}");
    btn_min.style_context().add_class("window-control");
    let btn_max = gtk::Button::with_label("\u{25a1}");
    btn_max.style_context().add_class("window-control");
    let btn_close = gtk::Button::with_label("\u{2715}");
    btn_close.style_context().add_class("window-control");
    btn_close.style_context().add_class("close");

    nav_bar.pack_start(&btn_back, false, false, 0);
    nav_bar.pack_start(&btn_fwd, false, false, 0);
    nav_bar.pack_start(&url_entry, true, true, 4);
    nav_bar.pack_end(&btn_close, false, false, 0);
    nav_bar.pack_end(&btn_max, false, false, 0);
    nav_bar.pack_end(&btn_min, false, false, 0);

    bars.pack_start(&nav_bar, false, false, 0);

    // ── Tab bar ─────────────────────────────────────────────────────────────
    let tab_bar = TabBar::new();
    tab_bar.container.style_context().add_class("toolbar");
    tab_bar.container.set_size_request(-1, 34);
    bars.pack_start(&tab_bar.container, false, false, 0);

    // ── Hit zone for showing bars (transparent strip at top) ────────────────
    let hit_zone = gtk::EventBox::new();
    hit_zone.set_valign(gtk::Align::Start);
    hit_zone.set_hexpand(true);
    hit_zone.set_size_request(-1, 6);
    hit_zone.set_above_child(true);
    hit_zone.set_visible_window(false);

    overlay.add_overlay(&bars);
    overlay.add_overlay(&hit_zone);

    // ── Auto-hide logic ─────────────────────────────────────────────────────
    bars.set_visible(false);
    window.add_events(gdk::EventMask::POINTER_MOTION_MASK);

    let b = bars.clone();
    let ue_ref = url_entry.clone();
    let bar_height = (BAR_H * 2 + 10) as f64; // both bars + padding
    window.connect_motion_notify_event(move |_, event| {
        let (_, y) = event.position();
        if y < 8.0 {
            // Mouse near top edge — show bars
            b.set_visible(true);
        } else if y > bar_height && b.is_visible() && !ue_ref.has_focus() {
            // Mouse below bars and URL not focused — hide
            b.set_visible(false);
        }
        glib::Propagation::Proceed
    });

    // Drag empty toolbar area to move window
    let w = window.clone();
    nav_bar.connect_button_press_event(move |_, event| {
        if event.button() == 1 {
            w.begin_move_drag(
                event.button() as i32,
                event.root().0 as i32,
                event.root().1 as i32,
                event.time(),
            );
        }
        glib::Propagation::Proceed
    });

    window.add(&overlay);

    // ── Wire everything ─────────────────────────────────────────────────────
    let w = window.clone();
    btn_min.connect_clicked(move |_| w.iconify());
    let w = window.clone();
    btn_max.connect_clicked(move |_| {
        if w.is_maximized() { w.unmaximize(); } else { w.maximize(); }
    });
    btn_close.connect_clicked(|_| gtk::main_quit());

    let wv = webview.clone();
    btn_back.connect_clicked(move |_| { wv.go_back(); });
    let wv = webview.clone();
    btn_fwd.connect_clicked(move |_| { wv.go_forward(); });

    let wv = webview.clone();
    let tb = tab_bar.clone();
    url_entry.connect_activate(move |entry| {
        let text = entry.text().to_string();
        let url = normalize_url(&text);
        wv.load_uri(&url);
        tb.update_current_url(&url);
    });

    let pw_store = Rc::new(RefCell::new(PasswordStore::load()));

    // On page load: update UI, inject password detection JS, autofill saved creds
    let ue = url_entry.clone();
    let tb = tab_bar.clone();
    let ps = pw_store.clone();
    webview.connect_load_changed(move |wv, event| {
        if event == LoadEvent::Committed || event == LoadEvent::Finished {
            if let Some(uri) = wv.uri() {
                ue.set_text(&uri);
                tb.update_current_url(&uri.to_string());
            }
            if let Some(title) = wv.title() {
                tb.update_current_title(&title.to_string());
            }
        }
        if event == LoadEvent::Finished {
            // Inject form submit detector
            wv.run_javascript(passwords::FORM_DETECT_JS, None::<&gio::Cancellable>, |_| {});

            // Autofill if we have saved credentials for this origin
            if let Some(uri) = wv.uri() {
                if let Ok(url) = url::Url::parse(&uri) {
                    let origin = url.origin().ascii_serialization();
                    let store = ps.borrow();
                    if let Some(creds) = store.lookup(&origin) {
                        if let Some(cred) = creds.first() {
                            let js = passwords::autofill_js(&cred.username, &cred.password());
                            wv.run_javascript(&js, None::<&gio::Cancellable>, |_| {});
                        }
                    }
                }
            }
        }
    });

    // Detect password save signals via title change hack
    let ps = pw_store.clone();
    let win = window.clone();
    webview.connect_title_notify(move |wv| {
        if let Some(title) = wv.title() {
            let title = title.to_string();
            if let Some(json) = title.strip_prefix("__LNTRN_PW__") {
                if let Ok(msg) = serde_json::from_str::<serde_json::Value>(json) {
                    let origin = msg["origin"].as_str().unwrap_or_default();
                    let username = msg["username"].as_str().unwrap_or_default();
                    let password = msg["password"].as_str().unwrap_or_default();
                    if !origin.is_empty() && !username.is_empty() && !password.is_empty() {
                        // Show save prompt
                        let dialog = gtk::MessageDialog::new(
                            Some(&win),
                            gtk::DialogFlags::MODAL,
                            gtk::MessageType::Question,
                            gtk::ButtonsType::YesNo,
                            &format!("Save password for {}?", username),
                        );
                        dialog.set_title("Save Password");
                        let response = dialog.run();
                        dialog.close();
                        if response == gtk::ResponseType::Yes {
                            ps.borrow_mut().store(origin, username, password);
                        }
                    }
                }
            }
        }
    });

    tab_bar.add_tab("New Tab", HOME_URL);

    let wv = webview.clone();
    let tb = tab_bar.clone();
    let ue = url_entry.clone();
    tab_bar.on_new_tab(move || {
        tb.add_tab("New Tab", HOME_URL);
        wv.load_uri(HOME_URL);
        ue.set_text(HOME_URL);
    });

    let wv = webview.clone();
    let ue = url_entry.clone();
    tab_bar.on_switch(move |url| {
        wv.load_uri(url);
        ue.set_text(url);
    });

    window.show_all();
    bars.set_visible(false); // ensure hidden after show_all

    window.connect_delete_event(|_, _| {
        gtk::main_quit();
        glib::Propagation::Stop
    });

    gtk::main();
}

fn normalize_url(text: &str) -> String {
    if text.starts_with("http://") || text.starts_with("https://") {
        text.to_string()
    } else if text.contains('.') && !text.contains(' ') {
        format!("https://{}", text)
    } else {
        format!("https://www.google.com/search?q={}", text.replace(' ', "+"))
    }
}

fn dirs_data() -> String {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    format!("{}/.local/share/lntrn-browser", home)
}

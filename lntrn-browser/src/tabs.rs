use gtk::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;

#[derive(Clone)]
struct Tab {
    url: String,
    button: gtk::Box,
    label: gtk::Label,
}

struct TabBarInner {
    tabs: Vec<Tab>,
    active: usize,
    on_switch: Option<Rc<dyn Fn(&str)>>,
    on_new: Option<Rc<dyn Fn()>>,
}

#[derive(Clone)]
pub struct TabBar {
    inner: Rc<RefCell<TabBarInner>>,
    pub container: gtk::Box,
    tabs_box: gtk::Box,
}

impl TabBar {
    pub fn new() -> Self {
        let container = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        container.style_context().add_class("tab-bar");

        let tabs_box = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        container.pack_start(&tabs_box, true, true, 0);

        let new_btn = gtk::Button::with_label("+");
        new_btn.style_context().add_class("new-tab-btn");
        container.pack_end(&new_btn, false, false, 0);

        let bar = Self {
            inner: Rc::new(RefCell::new(TabBarInner {
                tabs: Vec::new(),
                active: 0,
                on_switch: None,
                on_new: None,
            })),
            container,
            tabs_box,
        };

        let b = bar.clone();
        new_btn.connect_clicked(move |_| {
            let cb = b.inner.borrow().on_new.clone();
            if let Some(f) = cb {
                f();
            }
        });

        bar
    }

    pub fn add_tab(&self, title: &str, url: &str) {
        let idx = self.inner.borrow().tabs.len();

        let tab_box = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        tab_box.style_context().add_class("tab-btn");

        let label = gtk::Label::new(Some(title));
        label.set_max_width_chars(20);
        label.set_ellipsize(pango::EllipsizeMode::End);
        tab_box.pack_start(&label, false, false, 0);

        let close_btn = gtk::Button::with_label("\u{2715}");
        close_btn.style_context().add_class("tab-close");
        tab_box.pack_start(&close_btn, false, false, 0);

        let bar = self.clone();
        let event_box = gtk::EventBox::new();
        event_box.add(&tab_box);
        event_box.connect_button_press_event(move |_, _| {
            bar.switch_to(idx);
            glib::Propagation::Stop
        });

        let bar = self.clone();
        close_btn.connect_clicked(move |_| {
            bar.close_tab(idx);
        });

        self.tabs_box.pack_start(&event_box, false, false, 0);
        event_box.show_all();

        let tab = Tab {
            url: url.to_string(),
            button: tab_box,
            label,
        };

        self.inner.borrow_mut().tabs.push(tab);
        self.switch_to(idx);
    }

    fn switch_to(&self, idx: usize) {
        let mut inner = self.inner.borrow_mut();
        if idx >= inner.tabs.len() { return; }

        if inner.active < inner.tabs.len() {
            inner.tabs[inner.active].button.style_context().remove_class("active");
        }

        inner.active = idx;
        inner.tabs[idx].button.style_context().add_class("active");

        let url = inner.tabs[idx].url.clone();
        let cb = inner.on_switch.clone();
        drop(inner);
        if let Some(f) = cb { f(&url); }
    }

    fn close_tab(&self, idx: usize) {
        let tab_count = self.inner.borrow().tabs.len();
        if tab_count <= 1 { return; }

        let children = self.tabs_box.children();
        if idx < children.len() {
            self.tabs_box.remove(&children[idx]);
        }

        let mut inner = self.inner.borrow_mut();
        inner.tabs.remove(idx);

        if inner.active >= inner.tabs.len() {
            inner.active = inner.tabs.len() - 1;
        }
        let new_active = inner.active;
        inner.tabs[new_active].button.style_context().add_class("active");

        let url = inner.tabs[new_active].url.clone();
        let cb = inner.on_switch.clone();
        drop(inner);
        if let Some(f) = cb { f(&url); }
    }

    pub fn update_current_url(&self, url: &str) {
        let mut inner = self.inner.borrow_mut();
        let idx = inner.active;
        if idx < inner.tabs.len() {
            inner.tabs[idx].url = url.to_string();
        }
    }

    pub fn update_current_title(&self, title: &str) {
        let inner = self.inner.borrow();
        let idx = inner.active;
        if idx < inner.tabs.len() {
            inner.tabs[idx].label.set_text(title);
        }
    }

    pub fn on_new_tab<F: Fn() + 'static>(&self, f: F) {
        self.inner.borrow_mut().on_new = Some(Rc::new(f));
    }

    pub fn on_switch<F: Fn(&str) + 'static>(&self, f: F) {
        self.inner.borrow_mut().on_switch = Some(Rc::new(f));
    }
}

mod app;
mod branch_panel;
mod branch_view;
mod clone;
mod git;
mod graph_view;
mod main_view;
mod merge_modal;
mod keys;
mod popup_backend;
mod wayland;
mod worker;

fn main() {
    if let Err(e) = wayland::run() {
        eprintln!("[lntrn-git] fatal: {e}");
        std::process::exit(1);
    }
}

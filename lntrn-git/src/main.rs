mod app;
mod clone;
mod git;
mod popup_backend;
mod wayland;

fn main() {
    if let Err(e) = wayland::run() {
        eprintln!("[lntrn-git] fatal: {e}");
        std::process::exit(1);
    }
}

mod popup_backend;
mod wayland;

fn main() {
    if let Err(e) = wayland::run() {
        eprintln!("[ui-test] fatal: {e}");
        std::process::exit(1);
    }
}

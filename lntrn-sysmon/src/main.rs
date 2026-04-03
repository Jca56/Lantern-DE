mod dispatch;
mod performance;
mod popup_backend;
mod processes;
mod sysinfo;
mod tabs;
mod wayland;

fn main() {
    if let Err(e) = wayland::run() {
        eprintln!("[sysmon] fatal: {e}");
        std::process::exit(1);
    }
}

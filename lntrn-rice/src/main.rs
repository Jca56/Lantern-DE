mod app;
mod dispatch;
mod scenes;

fn main() {
    if let Err(e) = app::run(scenes::all()) {
        eprintln!("[lntrn-rice] fatal: {e}");
        std::process::exit(1);
    }
}

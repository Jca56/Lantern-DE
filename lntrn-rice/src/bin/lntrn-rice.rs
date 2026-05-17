use lntrn_rice::{draw_theme_background, run, AppConfig};

fn main() {
    let cfg = AppConfig {
        app_id: "lntrn-rice",
        title: "Rice",
        initial_width: 720,
        initial_height: 480,
    };
    if let Err(e) = run(cfg, |painter, ctx| {
        draw_theme_background(painter, ctx);
    }) {
        eprintln!("[lntrn-rice] fatal: {e}");
        std::process::exit(1);
    }
}

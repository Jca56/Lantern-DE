mod dispatch;
mod simulation;
mod theme;
mod wayland;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mut blob_count: usize = 3;
    let mut theme_name = String::from("classic");

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--blobs" | "-b" => {
                if let Some(n) = args.get(i + 1).and_then(|s| s.parse().ok()) {
                    blob_count = n;
                }
                i += 2;
            }
            "--theme" | "-t" => {
                if let Some(t) = args.get(i + 1) {
                    theme_name = t.clone();
                }
                i += 2;
            }
            "--help" | "-h" => {
                println!("lntrn-lava-lamp [OPTIONS]");
                println!("  -b, --blobs N     Number of blobs (default: 7)");
                println!("  -t, --theme NAME  Theme: classic, cosmic, neon, lofi (default: classic)");
                std::process::exit(0);
            }
            _ => { i += 1; }
        }
    }

    if let Err(e) = wayland::run(blob_count, &theme_name) {
        eprintln!("[lava-lamp] fatal: {e}");
        std::process::exit(1);
    }
}

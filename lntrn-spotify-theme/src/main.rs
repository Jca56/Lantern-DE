//! lntrn-spotify-theme — rices Spotify to match the Lantern DE.
//!
//! Patches Spotify's `xpui.spa` (a ZIP of its web UI) with a generated
//! stylesheet derived from the live `lantern.toml` theme: FoxPalette colors,
//! accent recolor, and translucent chrome so the compositor blur shows
//! through. Also installs a transparent-visuals launcher.
//!
//! Usage:
//!   lntrn-spotify-theme [apply]            # patch + install launcher
//!   lntrn-spotify-theme restore            # revert to pristine + remove launcher
//!   lntrn-spotify-theme status             # show current state
//!
//! Options (apply): --spa <path>  --opacity <0..1>  --surface-opacity <0..1>
//!                  --no-font  --no-launcher

mod css;
mod launcher;
mod patch;
mod zip;

use css::ThemeInputs;
use std::process::exit;

struct Opts {
    spa: Option<String>,
    base_alpha: Option<f32>,
    surface_alpha: Option<f32>,
    no_font: bool,
    no_launcher: bool,
    loud: bool,
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cmd = args.first().map(String::as_str).unwrap_or("apply");

    match cmd {
        "apply" => run_apply(parse_opts(&args[1..])),
        "restore" => run_restore(parse_opts(&args[1..])),
        "status" => run_status(parse_opts(&args[1..])),
        "-h" | "--help" | "help" => print_help(),
        other => {
            eprintln!("❌ unknown command: {other}\n");
            print_help();
            exit(2);
        }
    }
}

fn parse_opts(args: &[String]) -> Opts {
    let mut o = Opts {
        spa: None,
        base_alpha: None,
        surface_alpha: None,
        no_font: false,
        no_launcher: false,
        loud: false,
    };
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--spa" => {
                o.spa = args.get(i + 1).cloned();
                i += 1;
            }
            "--opacity" => {
                o.base_alpha = args.get(i + 1).and_then(|s| s.parse().ok());
                i += 1;
            }
            "--surface-opacity" => {
                o.surface_alpha = args.get(i + 1).and_then(|s| s.parse().ok());
                i += 1;
            }
            "--no-font" => o.no_font = true,
            "--no-launcher" => o.no_launcher = true,
            "--loud" => o.loud = true,
            other => {
                eprintln!("⚠️  ignoring unknown option: {other}");
            }
        }
        i += 1;
    }
    o
}

fn run_apply(o: Opts) {
    let spa = ok_or_die(patch::locate(o.spa.as_deref()));
    ok_or_die(patch::ensure_writable(&spa));

    let mut inp = ThemeInputs::from_config(o.base_alpha, o.surface_alpha);
    inp.override_font = !o.no_font;

    let css = if o.loud {
        css::generate_loud()
    } else {
        css::generate(&inp)
    };
    ok_or_die(patch::apply(&spa, &css));

    if o.loud {
        println!(
            "🔊 LOUD DIAGNOSTIC applied to {}. Expect hue-shifted colors + a magenta border.",
            spa.display()
        );
    } else {
        println!(
            "🎨 Themed {} → {} (accent #{:02X}{:02X}{:02X}, base α {:.2}, surface α {:.2})",
            spa.display(),
            inp.variant.name(),
            inp.accent.r,
            inp.accent.g,
            inp.accent.b,
            inp.base_alpha,
            inp.surface_alpha,
        );
    }

    if !o.no_launcher {
        let path = ok_or_die(launcher::install());
        println!("🚀 Transparent-visuals launcher → {}", path.display());
    }

    println!(
        "\n✨ Done! Fully quit Spotify (so it relaunches via the new launcher), then reopen it.\n   \
         If Spotify ever updates, just run `lntrn-spotify-theme` again to re-apply."
    );
}

fn run_restore(o: Opts) {
    let spa = ok_or_die(patch::locate(o.spa.as_deref()));
    ok_or_die(patch::restore(&spa));
    ok_or_die(launcher::uninstall());
    println!("↩️  Restored pristine Spotify + removed the transparent launcher.");
}

fn run_status(o: Opts) {
    let spa = ok_or_die(patch::locate(o.spa.as_deref()));
    let (patched, has_backup) = ok_or_die(patch::status(&spa));
    let inp = ThemeInputs::from_config(None, None);
    println!("📂 xpui.spa:   {}", spa.display());
    println!("🎨 patched:    {}", yn(patched));
    println!("💾 backup:     {}", yn(has_backup));
    println!("🚀 launcher:   {}", yn(launcher::installed()));
    println!("🦊 theme:      {}", inp.variant.name());
    println!(
        "🎯 accent:     #{:02X}{:02X}{:02X}",
        inp.accent.r, inp.accent.g, inp.accent.b
    );
}

fn print_help() {
    println!(
        "lntrn-spotify-theme — rice Spotify to match Lantern DE\n\n\
         USAGE:\n  \
         lntrn-spotify-theme [apply]      patch xpui.spa + install transparent launcher\n  \
         lntrn-spotify-theme restore      revert to pristine Spotify\n  \
         lntrn-spotify-theme status       show current state\n\n\
         APPLY OPTIONS:\n  \
         --spa <path>              override xpui.spa location\n  \
         --opacity <0..1>          base background alpha (lower = more blur shows)\n  \
         --surface-opacity <0..1>  panel/card alpha (higher = more readable)\n  \
         --no-font                 keep Spotify's bundled font\n  \
         --no-launcher             don't install the transparent-visuals launcher"
    );
}

fn yn(b: bool) -> &'static str {
    if b {
        "yes"
    } else {
        "no"
    }
}

fn ok_or_die<T>(r: Result<T, String>) -> T {
    match r {
        Ok(v) => v,
        Err(e) => {
            eprintln!("❌ {e}");
            exit(1);
        }
    }
}

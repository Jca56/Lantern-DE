use lntrn_snapshot::config::Config;
use lntrn_snapshot::manager::{SnapshotKind, SnapshotManager};
use std::env;
use std::process;

fn main() {
    let args: Vec<String> = env::args().collect();

    if args.len() < 2 {
        print_usage();
        process::exit(1);
    }

    // Root check — rsync needs it to preserve ownership/permissions
    if !is_root() {
        eprintln!("error: lntrn-snapshot requires root privileges");
        eprintln!("  try: sudo lntrn-snapshot {}", args[1..].join(" "));
        process::exit(1);
    }

    let config = Config::load();

    match args[1].as_str() {
        "init" => cmd_init(&config),
        "create" => {
            let kind = parse_kind_flag(&args);
            cmd_create(&config, kind);
        }
        "list" | "ls" => cmd_list(&config),
        "delete" | "rm" => {
            if args.len() < 3 {
                eprintln!("error: delete requires a snapshot name");
                eprintln!("  usage: lntrn-snapshot delete <name>");
                process::exit(1);
            }
            cmd_delete(&config, &args[2]);
        }
        "rename" | "mv" => {
            if args.len() < 4 {
                eprintln!("error: rename requires old and new name");
                eprintln!("  usage: lntrn-snapshot rename <old> <new>");
                process::exit(1);
            }
            cmd_rename(&config, &args[2], &args[3]);
        }
        "prune" => cmd_prune(&config),
        "rollback" => {
            if args.len() < 3 {
                eprintln!("error: rollback requires a snapshot name");
                eprintln!("  usage: lntrn-snapshot rollback <name>");
                process::exit(1);
            }
            cmd_rollback(&config, &args[2]);
        }
        "config" => cmd_config(),
        "help" | "--help" | "-h" => print_usage(),
        other => {
            eprintln!("error: unknown command '{}'", other);
            print_usage();
            process::exit(1);
        }
    }
}

fn print_usage() {
    eprintln!(
        "\
lntrn-snapshot — rsync snapshot manager

usage:
  lntrn-snapshot init                    Set up snapshot directories + default config
  lntrn-snapshot create [--kind <type>]  Create a snapshot (manual/boot/hourly/daily/weekly)
  lntrn-snapshot list                    List all snapshots
  lntrn-snapshot delete <name>           Delete a snapshot by name
  lntrn-snapshot rename <old> <new>      Rename a snapshot
  lntrn-snapshot prune                   Apply retention policy, remove old snapshots
  lntrn-snapshot rollback <name>         Create writable snapshot for rollback
  lntrn-snapshot config                  Show config file location and contents

aliases: list=ls, delete=rm"
    );
}

fn is_root() -> bool {
    unsafe { libc::geteuid() == 0 }
}

fn parse_kind_flag(args: &[String]) -> SnapshotKind {
    for i in 2..args.len() {
        if args[i] == "--kind" {
            if let Some(val) = args.get(i + 1) {
                return match val.as_str() {
                    "boot" => SnapshotKind::Boot,
                    "hourly" => SnapshotKind::Hourly,
                    "daily" => SnapshotKind::Daily,
                    "weekly" => SnapshotKind::Weekly,
                    "manual" => SnapshotKind::Manual,
                    other => {
                        eprintln!("warning: unknown kind '{}', using manual", other);
                        SnapshotKind::Manual
                    }
                };
            }
        }
    }
    SnapshotKind::Manual
}

fn make_manager(config: &Config, target: &lntrn_snapshot::config::SnapshotTarget) -> SnapshotManager {
    let mut mgr = SnapshotManager::new(
        target.source.clone(),
        target.snapshot_dir.clone(),
    );
    mgr.excludes = config.excludes.clone();
    // Always exclude the snapshot dir itself
    let snap_dir_str = target.snapshot_dir.to_string_lossy().to_string();
    if !mgr.excludes.contains(&snap_dir_str) {
        mgr.excludes.push(snap_dir_str);
    }
    mgr
}

// ── Commands ───────────────────────────────────────────────────────

fn cmd_init(config: &Config) {
    // Write default config if it doesn't exist
    let config_path = Config::config_path();
    if !config_path.exists() {
        match Config::write_default() {
            Ok(()) => println!("wrote default config to {}", config_path.display()),
            Err(e) => eprintln!("warning: couldn't write config: {}", e),
        }
    } else {
        println!("config already exists at {}", config_path.display());
    }

    // Create snapshot directories
    for target in &config.targets {
        let mgr = make_manager(config, target);
        match mgr.init() {
            Ok(()) => println!(
                "snapshot dir ready: {} -> {}",
                target.source.display(),
                target.snapshot_dir.display()
            ),
            Err(e) => eprintln!(
                "error creating {}: {}",
                target.snapshot_dir.display(),
                e
            ),
        }
    }
}

fn cmd_create(config: &Config, kind: SnapshotKind) {
    for target in &config.targets {
        let mgr = make_manager(config, target);

        match mgr.create(kind) {
            Ok(snap) => println!(
                "created {} -> {}",
                snap.name,
                snap.path.display()
            ),
            Err(e) => eprintln!(
                "error snapshotting {}: {}",
                target.source.display(),
                e
            ),
        }
    }

    // Auto-prune after create
    cmd_prune_quiet(config);
}

fn cmd_list(config: &Config) {
    for target in &config.targets {
        let mgr = make_manager(config, target);

        println!("snapshots for {}:", target.source.display());

        match mgr.list() {
            Ok(snaps) => {
                if snaps.is_empty() {
                    println!("  (none)");
                } else {
                    // Column widths
                    let max_name = snaps.iter().map(|s| s.name.len()).max().unwrap_or(20);

                    for snap in &snaps {
                        let kind_str = format!("{:?}", snap.kind);
                        let time_str = format_timestamp(snap.timestamp);
                        println!(
                            "  {:<width$}  {:8}  {}",
                            snap.name,
                            kind_str,
                            time_str,
                            width = max_name
                        );
                    }
                }
            }
            Err(e) => eprintln!("  error: {}", e),
        }

        println!();
    }
}

fn cmd_rename(config: &Config, old_name: &str, new_name: &str) {
    for target in &config.targets {
        let snap_path = target.snapshot_dir.join(old_name);
        if !snap_path.exists() {
            continue;
        }

        let mgr = make_manager(config, target);

        match mgr.rename(old_name, new_name) {
            Ok(()) => println!("renamed {} -> {}", old_name, new_name),
            Err(e) => eprintln!("error renaming {}: {}", old_name, e),
        }
        return;
    }

    eprintln!("snapshot '{}' not found in any target", old_name);
}

fn cmd_delete(config: &Config, name: &str) {
    for target in &config.targets {
        let snap_path = target.snapshot_dir.join(name);
        if !snap_path.exists() {
            continue;
        }

        let mgr = make_manager(config, target);

        match mgr.delete(name) {
            Ok(()) => println!("deleted {}", name),
            Err(e) => eprintln!("error deleting {}: {}", name, e),
        }
        return;
    }

    eprintln!("snapshot '{}' not found in any target", name);
}

fn cmd_prune(config: &Config) {
    for target in &config.targets {
        let mut mgr = SnapshotManager::new(
            target.source.clone(),
            target.snapshot_dir.clone(),
        );
        mgr.retention_mut(&config.retention);

        match mgr.prune() {
            Ok(deleted) => {
                if deleted.is_empty() {
                    println!("nothing to prune for {}", target.source.display());
                } else {
                    for name in &deleted {
                        println!("pruned {}", name);
                    }
                }
            }
            Err(e) => eprintln!("error pruning {}: {}", target.source.display(), e),
        }
    }
}

fn cmd_prune_quiet(config: &Config) {
    for target in &config.targets {
        let mut mgr = make_manager(config, target);
        mgr.retention_mut(&config.retention);
        let _ = mgr.prune();
    }
}

fn cmd_rollback(config: &Config, name: &str) {
    for target in &config.targets {
        let snap_path = target.snapshot_dir.join(name);
        if !snap_path.exists() {
            continue;
        }

        let mgr = make_manager(config, target);

        match mgr.rollback(name) {
            Ok(backup_path) => {
                println!("rollback complete! restored from: {}", name);
                println!("pre-rollback backup saved to: {}", backup_path.display());
                println!();
                println!("reboot recommended to pick up all changes.");
            }
            Err(e) => eprintln!("error rolling back: {}", e),
        }
        return;
    }

    eprintln!("snapshot '{}' not found in any target", name);
}

fn cmd_config() {
    let path = Config::config_path();
    println!("config: {}", path.display());
    println!();

    if path.exists() {
        match std::fs::read_to_string(&path) {
            Ok(content) => print!("{}", content),
            Err(e) => eprintln!("error reading config: {}", e),
        }
    } else {
        println!("(no config file — using defaults)");
        println!("run 'lntrn-snapshot init' to create one");
    }
}

fn format_timestamp(ts: i64) -> String {
    if ts == 0 {
        return "unknown".to_string();
    }

    let secs = ts as libc::time_t;
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe { libc::localtime_r(&secs, &mut tm) };

    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        tm.tm_year + 1900,
        tm.tm_mon + 1,
        tm.tm_mday,
        tm.tm_hour,
        tm.tm_min,
        tm.tm_sec
    )
}

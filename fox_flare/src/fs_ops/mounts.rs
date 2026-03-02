use std::fs;

// ── Mount entry ──────────────────────────────────────────────────────────────

pub struct MountEntry {
    pub label: String,
    pub path: String,
}

// ── Read mounts from /proc/mounts ────────────────────────────────────────────

pub fn get_mounts() -> Vec<MountEntry> {
    let mut mounts = vec![MountEntry {
        label: "Root".to_string(),
        path: "/".to_string(),
    }];

    if let Ok(content) = fs::read_to_string("/proc/mounts") {
        for line in content.lines() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() < 2 {
                continue;
            }
            let mount_point = parts[1];
            if mount_point.starts_with("/media/") || mount_point.starts_with("/mnt/") {
                let label = mount_point
                    .split('/')
                    .last()
                    .unwrap_or(mount_point)
                    .to_string();
                mounts.push(MountEntry {
                    label: if label.is_empty() {
                        mount_point.to_string()
                    } else {
                        label
                    },
                    path: mount_point.to_string(),
                });
            }
        }
    }

    mounts
}

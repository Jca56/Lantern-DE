/// Claude menu: session management and project navigation.
use crate::help_menu::{HelpCategory, HelpCommand};

pub const CATEGORIES: &[HelpCategory] = &[
    HelpCategory {
        name: "Session",
        commands: &[
            HelpCommand {
                cmd: "claude",
                desc: "New session",
            },
            HelpCommand {
                cmd: "claude --dangerously-skip-permissions",
                desc: "YOLO mode",
            },
            HelpCommand {
                cmd: "claude --resume",
                desc: "Resume last session",
            },
            HelpCommand {
                cmd: "claude sessions list",
                desc: "List all sessions",
            },
            HelpCommand {
                cmd: "claude sessions rename ",
                desc: "Rename a session",
            },
            HelpCommand {
                cmd: "claude sessions delete ",
                desc: "Delete a session",
            },
        ],
    },
    HelpCategory {
        name: "Projects",
        commands: &[
            HelpCommand {
                cmd: "cd ~/Documents/Projects/Lantern-DE",
                desc: "Project root",
            },
            HelpCommand {
                cmd: "cd ~/Documents/Projects/Lantern-DE/lntrn-compositor",
                desc: "Compositor",
            },
            HelpCommand {
                cmd: "cd ~/Documents/Projects/Lantern-DE/lntrn-bar",
                desc: "Status bar",
            },
            HelpCommand {
                cmd: "cd ~/Documents/Projects/Lantern-DE/lntrn-terminal",
                desc: "Terminal",
            },
            HelpCommand {
                cmd: "cd ~/Documents/Projects/Lantern-DE/lntrn-fox-file-manager",
                desc: "File manager",
            },
            HelpCommand {
                cmd: "cd ~/Documents/Projects/Lantern-DE/lntrn-session-manager",
                desc: "Session manager",
            },
            HelpCommand {
                cmd: "cd ~/Documents/Projects/Lantern-DE/lntrn-screenshot",
                desc: "Screenshot",
            },
            HelpCommand {
                cmd: "cd ~/Documents/Projects/Lantern-DE/lntrn-render",
                desc: "Render lib",
            },
            HelpCommand {
                cmd: "cd ~/Documents/Projects/Lantern-DE/lntrn-ui",
                desc: "UI widgets",
            },
            HelpCommand {
                cmd: "cd ~/Documents/Projects/Lantern-DE/lntrn-theme",
                desc: "Theme",
            },
            HelpCommand {
                cmd: "cd ~/Documents/Projects/Lantern-DE/lntrn",
                desc: "CLI tool",
            },
            HelpCommand {
                cmd: "cd ~/Documents/Projects/Lantern-DE/lntrn-image-viewer",
                desc: "Image viewer",
            },
            HelpCommand {
                cmd: "cd ~/Documents/Projects/Lantern-DE/lntrn-music-player",
                desc: "Music player",
            },
            HelpCommand {
                cmd: "cd ~/Documents/Projects/Lantern-DE/lntrn-video-player",
                desc: "Video player",
            },
            HelpCommand {
                cmd: "cd ~/Documents/Projects/Lantern-DE/lntrn-system-monitor",
                desc: "System monitor",
            },
            HelpCommand {
                cmd: "cd ~/Documents/Projects/Lantern-DE/lntrn-system-settings",
                desc: "Settings",
            },
            HelpCommand {
                cmd: "cd ~/Documents/Projects/Lantern-DE/lantern-aur",
                desc: "AUR package",
            },
            HelpCommand {
                cmd: "cd ~/Documents/Projects/Lantern-DE/docs",
                desc: "Documentation",
            },
        ],
    },
];

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
];

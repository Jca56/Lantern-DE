/// Help menu: categorized Linux/Arch commands for quick reference and insertion.

pub struct HelpCategory {
    pub name: &'static str,
    pub commands: &'static [HelpCommand],
}

pub struct HelpCommand {
    pub cmd: &'static str,
    pub desc: &'static str,
}

pub const CATEGORIES: &[HelpCategory] = &[
    HelpCategory {
        name: "System Info",
        commands: &[
            HelpCommand {
                cmd: "lntrn",
                desc: "Lantern system info",
            },
            HelpCommand {
                cmd: "uname -a",
                desc: "Kernel version",
            },
            HelpCommand {
                cmd: "cat /proc/cpuinfo | head -20",
                desc: "CPU info",
            },
            HelpCommand {
                cmd: "free -h",
                desc: "Memory usage",
            },
            HelpCommand {
                cmd: "df -h",
                desc: "Disk usage",
            },
            HelpCommand {
                cmd: "lsblk",
                desc: "Block devices",
            },
            HelpCommand {
                cmd: "uptime",
                desc: "System uptime",
            },
            HelpCommand {
                cmd: "hostnamectl",
                desc: "Hostname & OS",
            },
        ],
    },
    HelpCategory {
        name: "Pacman & AUR",
        commands: &[
            HelpCommand {
                cmd: "pacman -Syu",
                desc: "Full system upgrade",
            },
            HelpCommand {
                cmd: "pacman -Ss ",
                desc: "Search packages",
            },
            HelpCommand {
                cmd: "pacman -Si ",
                desc: "Package info",
            },
            HelpCommand {
                cmd: "pacman -Ql ",
                desc: "List package files",
            },
            HelpCommand {
                cmd: "pacman -Qe",
                desc: "Explicitly installed",
            },
            HelpCommand {
                cmd: "pacman -Qdt",
                desc: "Orphaned packages",
            },
            HelpCommand {
                cmd: "pacman -Rs ",
                desc: "Remove with deps",
            },
            HelpCommand {
                cmd: "pacman -Sc",
                desc: "Clean package cache",
            },
            HelpCommand {
                cmd: "paru",
                desc: "AUR helper upgrade",
            },
            HelpCommand {
                cmd: "paru -Ss ",
                desc: "Search AUR",
            },
        ],
    },
    HelpCategory {
        name: "Process Mgmt",
        commands: &[
            HelpCommand {
                cmd: "htop",
                desc: "Interactive processes",
            },
            HelpCommand {
                cmd: "btop",
                desc: "Fancy resource monitor",
            },
            HelpCommand {
                cmd: "ps aux | grep ",
                desc: "Find process",
            },
            HelpCommand {
                cmd: "kill -9 ",
                desc: "Force kill PID",
            },
            HelpCommand {
                cmd: "killall ",
                desc: "Kill by name",
            },
            HelpCommand {
                cmd: "systemctl status",
                desc: "Systemd overview",
            },
            HelpCommand {
                cmd: "systemctl --failed",
                desc: "Failed services",
            },
            HelpCommand {
                cmd: "journalctl -xe",
                desc: "Recent logs",
            },
        ],
    },
    HelpCategory {
        name: "Network",
        commands: &[
            HelpCommand {
                cmd: "ip addr",
                desc: "Network interfaces",
            },
            HelpCommand {
                cmd: "ping -c 4 google.com",
                desc: "Test connectivity",
            },
            HelpCommand {
                cmd: "ss -tulnp",
                desc: "Open ports",
            },
            HelpCommand {
                cmd: "curl ifconfig.me",
                desc: "Public IP",
            },
            HelpCommand {
                cmd: "nmcli device status",
                desc: "WiFi status",
            },
            HelpCommand {
                cmd: "nmcli device wifi list",
                desc: "Scan WiFi",
            },
            HelpCommand {
                cmd: "dig ",
                desc: "DNS lookup",
            },
            HelpCommand {
                cmd: "traceroute ",
                desc: "Trace route",
            },
        ],
    },
    HelpCategory {
        name: "Files & Disk",
        commands: &[
            HelpCommand {
                cmd: "ls -la",
                desc: "List all files",
            },
            HelpCommand {
                cmd: "du -sh *",
                desc: "Directory sizes",
            },
            HelpCommand {
                cmd: "ncdu",
                desc: "Interactive disk usage",
            },
            HelpCommand {
                cmd: "find . -name '*.rs'",
                desc: "Find by name",
            },
            HelpCommand {
                cmd: "tree -L 2",
                desc: "Directory tree",
            },
            HelpCommand {
                cmd: "fd ",
                desc: "Fast find (fd-find)",
            },
            HelpCommand {
                cmd: "rg ",
                desc: "Fast grep (ripgrep)",
            },
            HelpCommand {
                cmd: "bat ",
                desc: "Cat with syntax hl",
            },
        ],
    },
    HelpCategory {
        name: "Troubleshoot",
        commands: &[
            HelpCommand {
                cmd: "dmesg | tail -30",
                desc: "Kernel messages",
            },
            HelpCommand {
                cmd: "journalctl -b -p err",
                desc: "Boot errors",
            },
            HelpCommand {
                cmd: "pacman -Qkk",
                desc: "Verify pkg files",
            },
            HelpCommand {
                cmd: "mkinitcpio -P",
                desc: "Rebuild initramfs",
            },
            HelpCommand {
                cmd: "systemctl restart ",
                desc: "Restart service",
            },
            HelpCommand {
                cmd: "lsof -i :",
                desc: "What's on port",
            },
            HelpCommand {
                cmd: "strace -p ",
                desc: "Trace syscalls",
            },
            HelpCommand {
                cmd: "coredumpctl list",
                desc: "Recent crashes",
            },
        ],
    },
    HelpCategory {
        name: "Fun & Silly",
        commands: &[
            HelpCommand {
                cmd: "cowsay 'I use Arch btw'",
                desc: "Moo wisdom",
            },
            HelpCommand {
                cmd: "fortune | cowsay",
                desc: "Wise cow",
            },
            HelpCommand {
                cmd: "cmatrix",
                desc: "Matrix rain",
            },
            HelpCommand {
                cmd: "sl",
                desc: "Choo choo (typo ls)",
            },
            HelpCommand {
                cmd: "nyancat",
                desc: "Nyan!",
            },
            HelpCommand {
                cmd: "figlet Lantern",
                desc: "ASCII banner",
            },
            HelpCommand {
                cmd: "lolcat",
                desc: "Rainbow pipe",
            },
            HelpCommand {
                cmd: "asciiquarium",
                desc: "ASCII aquarium",
            },
            HelpCommand {
                cmd: "yes 'I use Arch btw'",
                desc: "Infinite Arch",
            },
            HelpCommand {
                cmd: "curl parrot.live",
                desc: "Party parrot",
            },
            HelpCommand {
                cmd: "toilet -f mono12 Lantern",
                desc: "Fancy ASCII text",
            },
            HelpCommand {
                cmd: "cal",
                desc: "Calendar",
            },
            HelpCommand {
                cmd: "factor 42069",
                desc: "Prime factors",
            },
            HelpCommand {
                cmd: "echo 'Lantern DE best DE' | rev",
                desc: "Reverse text",
            },
        ],
    },
];

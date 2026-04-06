//! Embedded icon assets for Lantern DE.
//!
//! All Lantern-owned SVG/PNG icons are compiled into the binary via `include_bytes!`.
//! Use `get(name)` to retrieve raw bytes by filename.

// ── Bar status icons ────────────────────────────────────────────────────────

const SPARK_BATTERY_CHARGING: &[u8] = include_bytes!("../../icons/bar/spark-battery-charging(1).svg");
const SPARK_BATTERY_HIGH: &[u8] = include_bytes!("../../icons/bar/spark-battery-high.svg");
const SPARK_BATTERY_LOW: &[u8] = include_bytes!("../../icons/bar/spark-battery-low.svg");
const SPARK_BATTERY_MEDIUM: &[u8] = include_bytes!("../../icons/bar/spark-battery-medium.svg");
const SPARK_BLUETOOTH_CONNECTED: &[u8] = include_bytes!("../../icons/bar/spark-bluetooth-connected.svg");
const SPARK_BLUETOOTH_OFF: &[u8] = include_bytes!("../../icons/bar/spark-bluetooth-off.svg");
const SPARK_BLUETOOTH_ON: &[u8] = include_bytes!("../../icons/bar/spark-bluetooth-on.svg");
const SPARK_BRIGHTNESS_HIGH: &[u8] = include_bytes!("../../icons/bar/spark-brightness-high.svg");
const SPARK_BRIGHTNESS_LOW: &[u8] = include_bytes!("../../icons/bar/spark-brightness-low.svg");
const SPARK_MENU_ALL: &[u8] = include_bytes!("../../icons/bar/spark-menu-all.svg");
const SPARK_MENU_DEVELOPMENT: &[u8] = include_bytes!("../../icons/bar/spark-menu-development.svg");
const SPARK_MENU_FAVORITES: &[u8] = include_bytes!("../../icons/bar/spark-menu-favorites.svg");
const SPARK_MENU_GRAPHICS: &[u8] = include_bytes!("../../icons/bar/spark-menu-graphics.svg");
const SPARK_MENU_INTERNET: &[u8] = include_bytes!("../../icons/bar/spark-menu-internet.svg");
const SPARK_MENU_LOCKSCREEN: &[u8] = include_bytes!("../../icons/bar/spark-menu-lockscreen.svg");
const SPARK_MENU_LOGOUT: &[u8] = include_bytes!("../../icons/bar/spark-menu-logout.svg");
const SPARK_MENU_MEDIA: &[u8] = include_bytes!("../../icons/bar/spark-menu-media.svg");
const SPARK_MENU_RESTART: &[u8] = include_bytes!("../../icons/bar/spark-menu-restart.svg");
const SPARK_MENU_SETTINGS: &[u8] = include_bytes!("../../icons/bar/spark-menu-settings.svg");
const SPARK_MENU_SHUTDOWN: &[u8] = include_bytes!("../../icons/bar/spark-menu-shutdown.svg");
const SPARK_MENU_SLEEP: &[u8] = include_bytes!("../../icons/bar/spark-menu-sleep.svg");
const SPARK_MENU_SYSTEM: &[u8] = include_bytes!("../../icons/bar/spark-menu-system.svg");
const SPARK_NOTIFICATION_BELL: &[u8] = include_bytes!("../../icons/bar/spark-notification-bell.svg");
const SPARK_SOUND_HIGH: &[u8] = include_bytes!("../../icons/bar/spark-sound-high.svg");
const SPARK_SOUND_LOW: &[u8] = include_bytes!("../../icons/bar/spark-sound-low.svg");
const SPARK_SOUND_MEDIUM: &[u8] = include_bytes!("../../icons/bar/spark-sound-medium.svg");
const SPARK_SOUND_MUTED: &[u8] = include_bytes!("../../icons/bar/spark-sound-muted.svg");
const SPARK_TEMP_COOL: &[u8] = include_bytes!("../../icons/bar/spark-temp-cool.svg");
const SPARK_TEMP_HOT: &[u8] = include_bytes!("../../icons/bar/spark-temp-hot.svg");
const SPARK_TEMP_WARM: &[u8] = include_bytes!("../../icons/bar/spark-temp-warm.svg");
const SPARK_USB: &[u8] = include_bytes!("../../icons/bar/spark-usb.svg");
const SPARK_WIFI_HIGH: &[u8] = include_bytes!("../../icons/bar/spark-wifi-high.svg");
const SPARK_WIFI_LOW: &[u8] = include_bytes!("../../icons/bar/spark-wifi-low.svg");
const SPARK_WIFI_MEDIUM: &[u8] = include_bytes!("../../icons/bar/spark-wifi-medium.svg");
const BAR_TERMINAL: &[u8] = include_bytes!("../../icons/bar/terminal.svg");

// ── App icons ───────────────────────────────────────────────────────────────

const LNTRN_BROWSER: &[u8] = include_bytes!("../../icons/apps/lntrn-browser.svg");
const LNTRN_CALCULATOR: &[u8] = include_bytes!("../../icons/apps/lntrn-calculator.svg");
const LNTRN_FILE_MANAGER: &[u8] = include_bytes!("../../icons/apps/lntrn-file-manager.svg");
const LNTRN_GIT: &[u8] = include_bytes!("../../icons/apps/lntrn-git.svg");
const LNTRN_IMAGE_VIEWER: &[u8] = include_bytes!("../../icons/apps/lntrn-image-viewer.svg");
const LNTRN_MEDIA_PLAYER: &[u8] = include_bytes!("../../icons/apps/lntrn-media-player.svg");
const LNTRN_NOTEPAD: &[u8] = include_bytes!("../../icons/apps/lntrn-notepad.svg");
const LNTRN_SCREENSHOT: &[u8] = include_bytes!("../../icons/apps/lntrn-screenshot.svg");
const LNTRN_SNAPSHOT: &[u8] = include_bytes!("../../icons/apps/lntrn-snapshot.svg");
const LNTRN_SYSMON: &[u8] = include_bytes!("../../icons/apps/lntrn-sysmon.svg");
const LNTRN_SYSTEM_SETTINGS: &[u8] = include_bytes!("../../icons/apps/lntrn-system-settings.svg");
const LNTRN_TERMINAL_SVG: &[u8] = include_bytes!("../../icons/apps/lntrn-terminal.svg");
const LNTRN_TERMINAL_PNG: &[u8] = include_bytes!("../../icons/apps/lntrn-terminal.png");
const LNTRN_PNG: &[u8] = include_bytes!("../../icons/apps/lntrn.png");

// ── Cursor icons ────────────────────────────────────────────────────────────

const CURSOR_DEFAULT: &[u8] = include_bytes!("../../icons/cursors/lntrn-cursor.svg");
const CURSOR_2: &[u8] = include_bytes!("../../icons/cursors/lntrn-cursor-2.svg");
const CURSOR_EW: &[u8] = include_bytes!("../../icons/cursors/lntrn-cursor-ew.svg");
const CURSOR_NESW: &[u8] = include_bytes!("../../icons/cursors/lntrn-cursor-nesw.svg");
const CURSOR_NS: &[u8] = include_bytes!("../../icons/cursors/lntrn-cursor-ns.svg");
const CURSOR_NWSE: &[u8] = include_bytes!("../../icons/cursors/lntrn-cursor-nwse.svg");

// ── Folder icons: Standard ──────────────────────────────────────────────────

const FOLDER_FILE_MANAGER: &[u8] = include_bytes!("../../icons/folders/Standard/lntrn-file-manager.svg");
const FOLDER_DESKTOP: &[u8] = include_bytes!("../../icons/folders/Standard/lntrn-folder-desktop.svg");
const FOLDER_DOCUMENTS: &[u8] = include_bytes!("../../icons/folders/Standard/lntrn-folder-documents.svg");
const FOLDER_DOWNLOADS: &[u8] = include_bytes!("../../icons/folders/Standard/lntrn-folder-downloads.svg");
const FOLDER_MUSIC: &[u8] = include_bytes!("../../icons/folders/Standard/lntrn-folder-music.svg");
const FOLDER_PICTURES: &[u8] = include_bytes!("../../icons/folders/Standard/lntrn-folder-pictures.svg");
const FOLDER_PROJECTS: &[u8] = include_bytes!("../../icons/folders/Standard/lntrn-folder-projects.svg");
const FOLDER_VIDEOS: &[u8] = include_bytes!("../../icons/folders/Standard/lntrn-folder-videos.svg");

// ── Folder icons: Colors ────────────────────────────────────────────────────

const FOLDER_BLACK: &[u8] = include_bytes!("../../icons/folders/Colors/lntrn-folder-black.svg");
const FOLDER_BLUE: &[u8] = include_bytes!("../../icons/folders/Colors/lntrn-folder-blue.svg");
const FOLDER_GREEN: &[u8] = include_bytes!("../../icons/folders/Colors/lntrn-folder-green.svg");
const FOLDER_ORANGE: &[u8] = include_bytes!("../../icons/folders/Colors/lntrn-folder-orange.svg");
const FOLDER_PURPLE: &[u8] = include_bytes!("../../icons/folders/Colors/lntrn-folder-purple.svg");
const FOLDER_RED: &[u8] = include_bytes!("../../icons/folders/Colors/lntrn-folder-red.svg");
const FOLDER_YELLOW: &[u8] = include_bytes!("../../icons/folders/Colors/lntrn-folder-yellow.svg");

// ── Folder icons: Awesome ───────────────────────────────────────────────────

const FOLDER_ALCHEMY: &[u8] = include_bytes!("../../icons/folders/Awesome/lntrn-folder-alchemy.svg");
const FOLDER_ARCADE: &[u8] = include_bytes!("../../icons/folders/Awesome/lntrn-folder-arcade.svg");
const FOLDER_ARCTIC: &[u8] = include_bytes!("../../icons/folders/Awesome/lntrn-folder-arctic.svg");
const FOLDER_AURORA: &[u8] = include_bytes!("../../icons/folders/Awesome/lntrn-folder-aurora.svg");
const FOLDER_BUTTERFLY: &[u8] = include_bytes!("../../icons/folders/Awesome/lntrn-folder-butterfly.svg");
const FOLDER_CASINO: &[u8] = include_bytes!("../../icons/folders/Awesome/lntrn-folder-casino.svg");
const FOLDER_CHERRYBLOSSOM: &[u8] = include_bytes!("../../icons/folders/Awesome/lntrn-folder-cherryblossom.svg");
const FOLDER_CIRCUIT: &[u8] = include_bytes!("../../icons/folders/Awesome/lntrn-folder-circuit.svg");
const FOLDER_COSMIC: &[u8] = include_bytes!("../../icons/folders/Awesome/lntrn-folder-cosmic.svg");
const FOLDER_CRYSTAL: &[u8] = include_bytes!("../../icons/folders/Awesome/lntrn-folder-crystal.svg");
const FOLDER_CYBERPUNK: &[u8] = include_bytes!("../../icons/folders/Awesome/lntrn-folder-cyberpunk.svg");
const FOLDER_DESERT: &[u8] = include_bytes!("../../icons/folders/Awesome/lntrn-folder-desert.svg");
const FOLDER_DNA: &[u8] = include_bytes!("../../icons/folders/Awesome/lntrn-folder-dna.svg");
const FOLDER_ENCHANTED: &[u8] = include_bytes!("../../icons/folders/Awesome/lntrn-folder-enchanted.svg");
const FOLDER_HAUNTED: &[u8] = include_bytes!("../../icons/folders/Awesome/lntrn-folder-haunted.svg");
const FOLDER_LAVA: &[u8] = include_bytes!("../../icons/folders/Awesome/lntrn-folder-lava.svg");
const FOLDER_MATRIX: &[u8] = include_bytes!("../../icons/folders/Awesome/lntrn-folder-matrix.svg");
const FOLDER_MUSHROOM: &[u8] = include_bytes!("../../icons/folders/Awesome/lntrn-folder-mushroom.svg");
const FOLDER_OBSERVATORY: &[u8] = include_bytes!("../../icons/folders/Awesome/lntrn-folder-observatory.svg");
const FOLDER_OCEAN: &[u8] = include_bytes!("../../icons/folders/Awesome/lntrn-folder-ocean.svg");
const FOLDER_STORM: &[u8] = include_bytes!("../../icons/folders/Awesome/lntrn-folder-storm.svg");
const FOLDER_TREASURE: &[u8] = include_bytes!("../../icons/folders/Awesome/lntrn-folder-treasure.svg");
const FOLDER_VAPORWAVE: &[u8] = include_bytes!("../../icons/folders/Awesome/lntrn-folder-vaporwave.svg");
const FOLDER_WAVE: &[u8] = include_bytes!("../../icons/folders/Awesome/lntrn-folder-wave.svg");

// ── Public API ──────────────────────────────────────────────────────────────

/// Get embedded icon bytes by filename.
///
/// Bar/app/cursor icons use flat names: `"spark-sound-high.svg"`, `"lntrn-cursor.svg"`
/// Folder icons use path-style: `"folders/Standard/lntrn-folder-desktop.svg"`
pub fn get(name: &str) -> Option<&'static [u8]> {
    Some(match name {
        // Bar status icons
        "spark-battery-charging(1).svg" => SPARK_BATTERY_CHARGING,
        "spark-battery-high.svg" => SPARK_BATTERY_HIGH,
        "spark-battery-low.svg" => SPARK_BATTERY_LOW,
        "spark-battery-medium.svg" => SPARK_BATTERY_MEDIUM,
        "spark-bluetooth-connected.svg" => SPARK_BLUETOOTH_CONNECTED,
        "spark-bluetooth-off.svg" => SPARK_BLUETOOTH_OFF,
        "spark-bluetooth-on.svg" => SPARK_BLUETOOTH_ON,
        "spark-brightness-high.svg" => SPARK_BRIGHTNESS_HIGH,
        "spark-brightness-low.svg" => SPARK_BRIGHTNESS_LOW,
        "spark-menu-all.svg" => SPARK_MENU_ALL,
        "spark-menu-development.svg" => SPARK_MENU_DEVELOPMENT,
        "spark-menu-favorites.svg" => SPARK_MENU_FAVORITES,
        "spark-menu-graphics.svg" => SPARK_MENU_GRAPHICS,
        "spark-menu-internet.svg" => SPARK_MENU_INTERNET,
        "spark-menu-lockscreen.svg" => SPARK_MENU_LOCKSCREEN,
        "spark-menu-logout.svg" => SPARK_MENU_LOGOUT,
        "spark-menu-media.svg" => SPARK_MENU_MEDIA,
        "spark-menu-restart.svg" => SPARK_MENU_RESTART,
        "spark-menu-settings.svg" => SPARK_MENU_SETTINGS,
        "spark-menu-shutdown.svg" => SPARK_MENU_SHUTDOWN,
        "spark-menu-sleep.svg" => SPARK_MENU_SLEEP,
        "spark-menu-system.svg" => SPARK_MENU_SYSTEM,
        "spark-notification-bell.svg" => SPARK_NOTIFICATION_BELL,
        "spark-sound-high.svg" => SPARK_SOUND_HIGH,
        "spark-sound-low.svg" => SPARK_SOUND_LOW,
        "spark-sound-medium.svg" => SPARK_SOUND_MEDIUM,
        "spark-sound-muted.svg" => SPARK_SOUND_MUTED,
        "spark-temp-cool.svg" => SPARK_TEMP_COOL,
        "spark-temp-hot.svg" => SPARK_TEMP_HOT,
        "spark-temp-warm.svg" => SPARK_TEMP_WARM,
        "spark-usb.svg" => SPARK_USB,
        "spark-wifi-high.svg" => SPARK_WIFI_HIGH,
        "spark-wifi-low.svg" => SPARK_WIFI_LOW,
        "spark-wifi-medium.svg" => SPARK_WIFI_MEDIUM,
        "terminal.svg" => BAR_TERMINAL,

        // App icons
        "lntrn-browser.svg" => LNTRN_BROWSER,
        "lntrn-calculator.svg" => LNTRN_CALCULATOR,
        "lntrn-file-manager.svg" => LNTRN_FILE_MANAGER,
        "lntrn-git.svg" => LNTRN_GIT,
        "lntrn-image-viewer.svg" => LNTRN_IMAGE_VIEWER,
        "lntrn-media-player.svg" => LNTRN_MEDIA_PLAYER,
        "lntrn-notepad.svg" => LNTRN_NOTEPAD,
        "lntrn-screenshot.svg" => LNTRN_SCREENSHOT,
        "lntrn-snapshot.svg" => LNTRN_SNAPSHOT,
        "lntrn-sysmon.svg" => LNTRN_SYSMON,
        "lntrn-system-settings.svg" => LNTRN_SYSTEM_SETTINGS,
        "lntrn-terminal.svg" => LNTRN_TERMINAL_SVG,
        "lntrn-terminal.png" => LNTRN_TERMINAL_PNG,
        "lntrn.png" => LNTRN_PNG,

        // Cursor icons
        "lntrn-cursor.svg" => CURSOR_DEFAULT,
        "lntrn-cursor-2.svg" => CURSOR_2,
        "lntrn-cursor-ew.svg" => CURSOR_EW,
        "lntrn-cursor-nesw.svg" => CURSOR_NESW,
        "lntrn-cursor-ns.svg" => CURSOR_NS,
        "lntrn-cursor-nwse.svg" => CURSOR_NWSE,

        // Folder icons: Standard
        "folders/Standard/lntrn-file-manager.svg" => FOLDER_FILE_MANAGER,
        "folders/Standard/lntrn-folder-desktop.svg" => FOLDER_DESKTOP,
        "folders/Standard/lntrn-folder-documents.svg" => FOLDER_DOCUMENTS,
        "folders/Standard/lntrn-folder-downloads.svg" => FOLDER_DOWNLOADS,
        "folders/Standard/lntrn-folder-music.svg" => FOLDER_MUSIC,
        "folders/Standard/lntrn-folder-pictures.svg" => FOLDER_PICTURES,
        "folders/Standard/lntrn-folder-projects.svg" => FOLDER_PROJECTS,
        "folders/Standard/lntrn-folder-videos.svg" => FOLDER_VIDEOS,

        // Folder icons: Colors
        "folders/Colors/lntrn-folder-black.svg" => FOLDER_BLACK,
        "folders/Colors/lntrn-folder-blue.svg" => FOLDER_BLUE,
        "folders/Colors/lntrn-folder-green.svg" => FOLDER_GREEN,
        "folders/Colors/lntrn-folder-orange.svg" => FOLDER_ORANGE,
        "folders/Colors/lntrn-folder-purple.svg" => FOLDER_PURPLE,
        "folders/Colors/lntrn-folder-red.svg" => FOLDER_RED,
        "folders/Colors/lntrn-folder-yellow.svg" => FOLDER_YELLOW,

        // Folder icons: Awesome
        "folders/Awesome/lntrn-folder-alchemy.svg" => FOLDER_ALCHEMY,
        "folders/Awesome/lntrn-folder-arcade.svg" => FOLDER_ARCADE,
        "folders/Awesome/lntrn-folder-arctic.svg" => FOLDER_ARCTIC,
        "folders/Awesome/lntrn-folder-aurora.svg" => FOLDER_AURORA,
        "folders/Awesome/lntrn-folder-butterfly.svg" => FOLDER_BUTTERFLY,
        "folders/Awesome/lntrn-folder-casino.svg" => FOLDER_CASINO,
        "folders/Awesome/lntrn-folder-cherryblossom.svg" => FOLDER_CHERRYBLOSSOM,
        "folders/Awesome/lntrn-folder-circuit.svg" => FOLDER_CIRCUIT,
        "folders/Awesome/lntrn-folder-cosmic.svg" => FOLDER_COSMIC,
        "folders/Awesome/lntrn-folder-crystal.svg" => FOLDER_CRYSTAL,
        "folders/Awesome/lntrn-folder-cyberpunk.svg" => FOLDER_CYBERPUNK,
        "folders/Awesome/lntrn-folder-desert.svg" => FOLDER_DESERT,
        "folders/Awesome/lntrn-folder-dna.svg" => FOLDER_DNA,
        "folders/Awesome/lntrn-folder-enchanted.svg" => FOLDER_ENCHANTED,
        "folders/Awesome/lntrn-folder-haunted.svg" => FOLDER_HAUNTED,
        "folders/Awesome/lntrn-folder-lava.svg" => FOLDER_LAVA,
        "folders/Awesome/lntrn-folder-matrix.svg" => FOLDER_MATRIX,
        "folders/Awesome/lntrn-folder-mushroom.svg" => FOLDER_MUSHROOM,
        "folders/Awesome/lntrn-folder-observatory.svg" => FOLDER_OBSERVATORY,
        "folders/Awesome/lntrn-folder-ocean.svg" => FOLDER_OCEAN,
        "folders/Awesome/lntrn-folder-storm.svg" => FOLDER_STORM,
        "folders/Awesome/lntrn-folder-treasure.svg" => FOLDER_TREASURE,
        "folders/Awesome/lntrn-folder-vaporwave.svg" => FOLDER_VAPORWAVE,
        "folders/Awesome/lntrn-folder-wave.svg" => FOLDER_WAVE,

        _ => return None,
    })
}

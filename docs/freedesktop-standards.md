# Freedesktop.org Standards Reference

Standards that Lantern DE must support for Linux desktop compatibility.
These are the protocols and conventions that let existing Linux apps work
seamlessly across any compliant desktop environment.

---

## Desktop Entries (.desktop files)

**Spec:** [Desktop Entry Specification](https://specifications.freedesktop.org/desktop-entry-spec/latest/)

Standard format for application launchers, used by app menus, file managers,
and MIME handlers across all DEs.

**Location:** `/usr/share/applications/` (system) and `~/.local/share/applications/` (user)

**Key fields:**
```ini
[Desktop Entry]
Type=Application
Name=Firefox
GenericName=Web Browser
Comment=Browse the web
Exec=firefox %u
Icon=firefox
Terminal=false
Categories=Network;WebBrowser;
MimeType=text/html;application/xhtml+xml;
StartupNotify=true
StartupWMClass=firefox
```

**What Lantern needs:**
- Read `.desktop` files to populate app launchers / menus
- Respect `Exec` field variable expansion (`%u`, `%f`, `%U`, `%F`)
- Resolve `Icon` field via icon theme lookup
- Parse `Categories` for menu organization
- Handle `MimeType` for default application associations

---

## XDG Base Directory

**Spec:** [XDG Base Directory Specification](https://specifications.freedesktop.org/basedir-spec/latest/)

Standard paths for config, data, cache, and runtime files.

| Variable | Default | Purpose |
|---|---|---|
| `XDG_CONFIG_HOME` | `~/.config` | User config files |
| `XDG_DATA_HOME` | `~/.local/share` | User data files |
| `XDG_CACHE_HOME` | `~/.cache` | Non-essential cached data |
| `XDG_STATE_HOME` | `~/.local/state` | Persistent state (logs, history) |
| `XDG_RUNTIME_DIR` | `/run/user/$UID` | Runtime sockets, pipes (tmpfs) |
| `XDG_DATA_DIRS` | `/usr/local/share:/usr/share` | System data search path |
| `XDG_CONFIG_DIRS` | `/etc/xdg` | System config search path |

**What Lantern needs:**
- All Lantern config goes in `$XDG_CONFIG_HOME/lantern-de/`
- Always check env vars first, fall back to defaults
- Search `XDG_DATA_DIRS` when looking for `.desktop` files, icons, etc.

---

## Icon Theme

**Spec:** [Icon Theme Specification](https://specifications.freedesktop.org/icon-theme-spec/latest/)

How to find the correct icon for an app, action, or MIME type.

**Lookup order:**
1. Current theme directory (e.g., `~/.local/share/icons/MyTheme/`)
2. System theme directories (`/usr/share/icons/MyTheme/`)
3. Fallback to `hicolor` theme (`/usr/share/icons/hicolor/`)
4. Pixmaps fallback (`/usr/share/pixmaps/`)

**Icon directories contain size variants:**
```
icons/hicolor/
    48x48/apps/firefox.png
    scalable/apps/firefox.svg
    symbolic/apps/firefox-symbolic.svg
```

**What Lantern needs:**
- Icon lookup function: given a name + size, resolve to a file path
- Support PNG and SVG formats
- Respect `index.theme` files for directory metadata
- `hicolor` is always the ultimate fallback

---

## StatusNotifierItem (SNI) — System Tray

**Spec:** [StatusNotifierItem](https://www.freedesktop.org/wiki/Specifications/StatusNotifierItem/)
**Protocol:** D-Bus

The modern system tray protocol. Apps register tray icons via D-Bus instead
of the legacy XEmbed protocol.

**D-Bus interfaces:**
- `org.kde.StatusNotifierWatcher` — Host registers as watcher, apps register items
- `org.kde.StatusNotifierItem` — Individual tray item interface
- `org.kde.StatusNotifierHost` — A host (the bar) consuming items

**Each tray item exposes:**
- `IconName` / `IconPixmap` — The tray icon
- `Title` — Tooltip title
- `Status` — Active, Passive, NeedsAttention
- `Menu` — D-Bus menu path (via dbusmenu protocol)
- `Activate()` / `SecondaryActivate()` — Left/right click actions

**Apps using SNI:** Discord, Steam, KeePassXC, Telegram, Slack, etc.

**What Lantern needs:**
- Implement `StatusNotifierHost` (register on D-Bus as a watcher)
- Listen for registered `StatusNotifierItem` signals
- Render tray icons in the bar
- Forward click events to items via D-Bus method calls
- Support `com.canonical.dbusmenu` for tray icon context menus

---

## MPRIS — Media Player Remote Interface

**Spec:** [MPRIS D-Bus Interface Specification](https://specifications.freedesktop.org/mpris-spec/latest/)
**Protocol:** D-Bus

Standardized media player control. Any MPRIS-aware widget can control any
MPRIS-aware player.

**D-Bus service name:** `org.mpris.MediaPlayer2.<player_name>`

**Key interfaces:**
- `org.mpris.MediaPlayer2` — App identity (name, icon, supported URI schemes)
- `org.mpris.MediaPlayer2.Player` — Playback control
  - Methods: `Play()`, `Pause()`, `PlayPause()`, `Stop()`, `Next()`, `Previous()`, `Seek()`
  - Properties: `PlaybackStatus`, `Metadata`, `Volume`, `Position`, `Rate`

**Metadata map keys:**
- `xesam:title` — Track title
- `xesam:artist` — Artist name(s)
- `xesam:album` — Album name
- `mpris:artUrl` — Album art URL
- `mpris:length` — Track duration (microseconds)

**What Lantern needs (if building a media Spark):**
- D-Bus client to discover `org.mpris.MediaPlayer2.*` services
- Read metadata properties for display
- Call playback methods for controls
- Listen for `PropertiesChanged` signals for live updates

---

## Desktop Notifications

**Spec:** [Desktop Notifications Specification](https://specifications.freedesktop.org/notification-spec/latest/)
**Protocol:** D-Bus

Apps send notifications via D-Bus; a notification daemon displays them.

**D-Bus interface:** `org.freedesktop.Notifications`

**Key method:** `Notify(app_name, replaces_id, icon, summary, body, actions, hints, timeout)`

**Hints include:**
- `urgency` — Low (0), Normal (1), Critical (2)
- `image-data` — Inline image
- `sound-file` — Sound to play
- `desktop-entry` — Associated `.desktop` file

**What Lantern needs (eventually):**
- Implement notification daemon (claim `org.freedesktop.Notifications` on D-Bus)
- Display notification popups
- Handle actions (buttons in notifications)
- Notification history / notification center

---

## Window Manager Hints (EWMH/ICCCM)

**Spec:** [Extended Window Manager Hints](https://specifications.freedesktop.org/wm-spec/latest/)

X11 properties and atoms that window managers, taskbars, and pagers use to
communicate about windows.

**Key atoms for the bar:**
- `_NET_WM_WINDOW_TYPE` — DOCK, NORMAL, DIALOG, POPUP_MENU, etc.
- `_NET_WM_STATE` — ABOVE, SKIP_TASKBAR, SKIP_PAGER, STICKY, FULLSCREEN
- `_NET_WM_STRUT` / `_NET_WM_STRUT_PARTIAL` — Reserved screen space
- `_NET_CLIENT_LIST` — List of managed windows (for taskbar window list)
- `_NET_ACTIVE_WINDOW` — Currently focused window
- `_NET_WM_NAME` — UTF-8 window title
- `_NET_WM_ICON` — Window icon
- `_NET_CURRENT_DESKTOP` / `_NET_NUMBER_OF_DESKTOPS` — Workspace info
- `_NET_WORKAREA` — Usable screen area (minus struts)

**What Lantern needs (already partially done):**
- ✅ DOCK type, strut registration (done in x11.rs)
- ✅ SKIP_TASKBAR, SKIP_PAGER, STICKY, ABOVE (done)
- Future: Read `_NET_CLIENT_LIST` for a window list / task switcher Spark
- Future: Read `_NET_ACTIVE_WINDOW` for highlighting active window

---

## Autostart

**Spec:** [Desktop Application Autostart Specification](https://specifications.freedesktop.org/autostart-spec/latest/)

**Locations:**
- `$XDG_CONFIG_HOME/autostart/` (user)
- `$XDG_CONFIG_DIRS/autostart/` (system, typically `/etc/xdg/autostart/`)

Apps place `.desktop` files here to run on login. Same format as regular
`.desktop` files with optional `OnlyShowIn` / `NotShowIn` fields to target
specific DEs.

**What Lantern needs (eventually):**
- Scan autostart directories on session start
- Respect `OnlyShowIn=Lantern;` and `NotShowIn` filters
- Set `$XDG_CURRENT_DESKTOP=Lantern` in the session environment

---

## MIME Applications

**Spec:** [Association between MIME types and applications](https://specifications.freedesktop.org/mime-apps-spec/latest/)

How the system knows which app opens which file type.

**Files:**
- `~/.config/mimeapps.list` — User default associations
- `/usr/share/applications/mimeapps.list` — System defaults
- Per-desktop: `~/.config/lantern-de-mimeapps.list`

**Format:**
```ini
[Default Applications]
text/html=firefox.desktop
image/png=lantern-image-viewer.desktop

[Added Associations]
text/html=firefox.desktop;chromium.desktop;
```

**What Lantern needs (for file manager / app launcher):**
- Read `mimeapps.list` to determine default apps
- Allow users to set default applications

---

## Summary: Implementation Priority for the Bar

| Priority | Standard | Why |
|---|---|---|
| ✅ Done | EWMH (dock, struts) | Bar registers as dock |
| 🔜 Next | Desktop Entries | App launcher / menu needs these |
| 🔜 Next | Icon Theme | Display app icons |
| 🔜 Next | XDG Base Directory | Config paths (partially done) |
| Later | SNI (System Tray) | Tray icons for existing apps |
| Later | MPRIS | Media controls Spark |
| Later | Notifications | Notification daemon |
| Later | Autostart | Session management |
| Later | MIME Apps | File associations |

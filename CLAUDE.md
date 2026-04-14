# Lantern DE

Custom Linux Arch btw Wayland desktop environment built with Smithay 0.7 in Rust with wgpu.

## Build & Deploy
```bash
cargo build --release -p lntrn-compositor
cp target/release/lntrn-compositor /tmp/lntrn-compositor-new && mv -f /tmp/lntrn-compositor-new ~/.lantern/bin/lntrn-compositor
```
All binaries go to `~/.lantern/bin/`. All config lives in `~/.lantern/config/`. Logs go to `~/.lantern/log/`.

## UI & Theming
All window applications in the workspace must use `lntrn-ui` for widgets and `lntrn-theme` for colors/typography to keep a consistent look across the DE.

- Import widgets from `lntrn_ui::gpu` (Button, TextInput, TextLabel, Panel, TitleBar, ScrollArea, etc.)
- Use `FoxPalette::from_theme(variant)` for all colors — never hardcode color values
- Use `lntrn_theme` font size constants and `FontSize` enum for typography
- Use `InteractionContext` for unified hit-testing and interaction state
- Use `lntrn_ui::animation` for easing/duration constants

## Preferences
- Always prefer building our own dependencies over using external crates. Minimal outside dependencies — we build all our own stuff! Only reach for an external crate when it would be incredibly difficult to implement ourselves.
- Output scale: 1.0 native (DP-1: 2560x1440, HDMI-A-1: 1920x1080). Non-Lantern apps scaled via env vars (GDK_DPI_SCALE=1.25, QT_SCALE_FACTOR=1.25).
- Large font sizes, minimum of 16 or 18. User has poor eyesight — always err on the side of BIGGER text and UI elements. When in doubt, make it larger.
- When given tasks you will ask questions using the `AskUserQuestion` tool.
- Files must be kept at less than 600 lines of code and flagged at 500 lines. If you feel there is a reasonable exception for keeping a file together you can explain your reasoning.
- You are friendly, funny, hype, make jokes, and use emojis. You bounce of my chaotic gremlin ADHD energy and we make awesome projects together.
- Commit messages are short - just the feature name or fix. No long descriptions. Do not add yourself as a coauther or add any other information beyond the commit message.
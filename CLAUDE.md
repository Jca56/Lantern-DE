# Lantern DE

Custom Linux Arch btw Wayland desktop environment built with Smithay 0.7 in Rust with wgpu.

## Build & Deploy
```bash
cargo build --release -p lntrn-compositor
cp target/release/lntrn-compositor /tmp/lntrn-compositor-new && mv -f /tmp/lntrn-compositor-new ~/.lantern/bin/lntrn-compositor
```
All binaries go to `~/.lantern/bin/`. All config lives in `~/.lantern/config/`. Logs go to `~/.lantern/log/`.

## Preferences
- Always prefer building our own dependencies over using external crates. Minimal outside dependencies — we build all our own stuff! Only reach for an external crate when it would be incredibly difficult to implement ourselves.
- Output scale: 1.25 (1920x1200 physical, 1536x960 logical)
- Large font sizes. User has poor eyesight — always err on the side of BIGGER text and UI elements. When in doubt, make it larger.
- Ask questions using the `AskUserQuestion` tool before making any changes to code.
- Files should be flagged at 500-599 lines and must stay under 700 lines. Files at 1,000+ lines should be flagged for splitting. If you feel there is a reasonable exception for keeping a file together you can explain your reasoning.
- You are friendly, funny, hype, make jokes, and use emojis. You bounce of my chaotic gremlin ADHD energy and we make awesome projects together.
- Commit messages are short - just the feature name or fix. No long descriptions. Do not add yourself as a coauther or add any other information beyond the commit message.
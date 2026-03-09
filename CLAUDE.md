# Lantern DE

Custom Linux Arch btw Wayland desktop environment built with Smithay 0.7 in Rust with wgpu.

## Build & Deploy
```bash
cargo build --release -p lntrn-compositor
cp target/release/lntrn-compositor /tmp/lntrn-compositor-new && mv -f /tmp/lntrn-compositor-new ~/.local/bin/lntrn-compositor
```
Never use `~/.cargo/bin/` — that's only updated by `cargo install`.

## Preferences
- Always prefer building our own dependencies over using external crates. Minimal outside dependencies — we build all our own stuff! Only reach for an external crate when it would be incredibly difficult to implement ourselves.
- Output scale: 1.25 (1920x1200 physical, 1536x960 logical)
- Large font sizes, minimum of 14 or 16.
- When given tasks you will ask questions using the `AskUserQuestion` tool.
- Files must be kept at less than 600 lines of code and flagged at 500 lines. If you feel there is a reasonable exception for keeping a file together you can explain your reasoning.
- You are friendly, funny, hype, make jokes, and use emojis. You bounce of my chaotic gremlin ADHD energy and we make awesome projects together.
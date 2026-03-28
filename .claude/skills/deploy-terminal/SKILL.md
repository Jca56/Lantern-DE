---
name: deploy-terminal
description: Build and deploy lntrn-terminal to ~/.lantern/bin
---

Build and deploy the terminal:

1. Run: `cargo build --release -p lntrn-terminal`
2. If build succeeds: `cp target/release/lntrn-terminal /tmp/lntrn-terminal-new && mv -f /tmp/lntrn-terminal-new ~/.lantern/bin/lntrn-terminal`
3. Report success or failure

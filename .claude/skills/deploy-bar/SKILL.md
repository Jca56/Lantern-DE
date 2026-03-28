---
name: deploy-bar
description: Build and deploy lntrn-bar to ~/.lantern/bin
---

Build and deploy the bar:

1. Run: `cargo build --release -p lntrn-bar`
2. If build succeeds: `cp target/release/lntrn-bar /tmp/lntrn-bar-new && mv -f /tmp/lntrn-bar-new ~/.lantern/bin/lntrn-bar`
3. Kill and relaunch: `pkill lntrn-bar; sleep 0.3; lntrn-bar &`
4. Report success or failure

IMPORTANT: Never use `pkill -f` — it matches the full cmdline and will kill the compositor too.

---
name: deploy-fox
description: Build and deploy lntrn-file-manager to ~/.local/bin
---

Build and deploy the file manager:

1. Run: `cargo build --release -p lntrn-file-manager`
2. If build succeeds: `cp target/release/lntrn-file-manager /tmp/lntrn-file-manager-new && mv -f /tmp/lntrn-file-manager-new ~/.local/bin/lntrn-file-manager`
3. Report success or failure

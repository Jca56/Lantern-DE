# ⚠️ lntrn-bar is DEPRECATED

**Replaced by:** `lntrn-command-center`

The command-center is now the Lantern shell bar. It's launched by the
compositor (Super-tap / hot corners) and owns the clock, app tray, audio,
system info — everything this crate used to do.

## Status

- **Not auto-started by anything.** No XDG autostart entry, no
  session-manager spawn. It only runs if launched by hand.
- When run manually it defaults to the **bottom** edge of the screen.
- Kept in the workspace for reference and salvageable widgets.

## If you want it gone for good

```bash
# from the workspace root
git rm -r lntrn-bar
# then drop "lntrn-bar" from the workspace members in the root Cargo.toml
```

Everything is recoverable via git history.

# Git Feature & Sidebar — Phased Implementation Plan

Status legend: ⬜ todo · 🟡 in progress · ✅ done

## Architecture recap (what exists today)
- **Worker pattern** (`git/worker.rs`): UI sends `GitCmd` → worker runs an `ops::` fn → emits `GitEvent` (`Status`/`Branches`/`GraphData`/`Message`/`Error`). Adding a feature = new `GitCmd` variant + `ops::` fn + match arm + `GitAction` + UI affordance.
- **ops** (`git/ops.rs`): `status`, `stage`, `unstage`, `commit`, `push`, `push_new_branch` (already exists!), `pull`, `list_branches`, `switch_branch`, `log_structured`, `ahead_behind`.
- **UI** (`git_sidebar.rs`, **668 lines — OVER the limit**): branch header (collapsible list + switch), commit input, push/pull buttons, changes (stage/unstage all + per-file), recent commits (30), status toast.
- **Sidebar shell** (`sidebar/`): mode toggle [Files][Git], now **resizable** (done) with `manual_width`, `resize_to`, `reset_width`.

## ⚠️ Pre-req: split `git_sidebar.rs` (Phase 0)
It's already over 600 lines and every phase adds to it. Split first, mirroring `sidebar/`:
- `git_sidebar/mod.rs` — state, `GitAction`, layout consts
- `git_sidebar/draw.rs` — `draw_git_sidebar` + helpers
- `git_sidebar/input.rs` — `handle_click`, `handle_right_click`, key/char input, `contains`
Also verify **Pull button is surfaced** in the UI (action is wired; confirm the button draws + hit-tests).

---

## Phase 0 — Foundation & quick wins  ✅ DONE
Low risk, unblocks everything else.
1. ✅ Split `git_sidebar.rs` into `git_sidebar/{mod,draw,input}.rs` (all < 500 lines).
2. ✅ Confirmed **Pull** renders (draw.rs) + hit-tests (input.rs) — already wired. Clone → Phase 3.
3. ✅ **Persist sidebar width**: `SidebarConfig.width: Option<f32>` in `terminal.toml`, restored via `apply_saved_width` on startup, saved on resize-end.
4. ✅ **Double-click resize handle → `reset_width()`** (400ms window in `events.rs`, clears saved width).

## Phase 1 — Per-file right-click context menu  ⬜
Mirror the file sidebar's existing context-menu pattern.
- New `git_sidebar` state: `context_menu: Option<(file_path, x, y)>`.
- Items: **Stage/Unstage** (toggle), **Discard changes** (`git checkout -- <f>` / `git restore`), **Open in editor** (`$EDITOR`/xdg-open), **Copy path**, **View history** (→ feeds Phase 3).
- New `GitCmd::Discard(String)` + `ops::discard(repo, path)`. Discard is destructive → confirm inline or require a modifier.
- `handle_right_click` in `git_sidebar/input.rs`; draw menu in `draw.rs`; dispatch in `git_app.rs`.

## Phase 2 — Inline diff viewer  ⬜  (the big one)
Click a changed file → see the diff.
- `ops::diff(repo, path, staged: bool) -> Vec<DiffLine>` via `git diff [--cached] -- <path>`; parse into `{kind: Context|Add|Del|Hunk, text}`.
- New `GitCmd::FetchDiff{path, staged}` → `GitEvent::Diff(path, Vec<DiffLine>)`.
- State: `expanded_diff: Option<(String, Vec<DiffLine>)>`. Clicking a file row toggles it.
- Draw: render diff lines indented under the file row — green adds / red dels / dim context, monospace, clipped + scrollable. (Decide: inline-expand vs. dedicated diff pane in the terminal grid. Inline-expand is simpler & matches the sidebar feel — recommend starting there.)
- Stretch: stage/unstage individual **hunks** (`git apply --cached`).

## Phase 3 — Clone + stash + branch ops + fetch  ⬜
Round out everyday workflows. All follow the worker recipe.
- **Clone** (the headline ask): `GitCmd::Clone{url, dest}` → `ops::clone(url, dest)` runs `git clone <url> <dest>`.
  - UI: when there's **no repo open**, the git sidebar shows an empty state with a **URL input + destination** (default to active pane CWD, or a folder picker / typed path) and a **Clone** button. Clone is long-running → show progress/spinner via `GitEvent::Message` and stream output if feasible.
  - On success: `set_root` the file sidebar + `open_git_repo` on the new path so it immediately populates.
  - Note: clone runs *outside* an existing repo, so it can't reuse the `repo_path`-gated worker arms — handle `dest` explicitly. Consider running it in the active terminal pane instead for live output (decide during impl).
- **Stash**: `GitCmd::Stash`, `StashPop`, `StashList` → `ops::stash/stash_pop/stash_list`. New collapsible "Stashes" section.
- **Branches**: create (reuse `push_new_branch` for the push side; `ops::create_branch`), delete (`ops::delete_branch`, confirm). Add "＋ New branch" affordance + per-branch right-click delete in the expanded branch list.
- **Fetch**: `GitCmd::Fetch` + `ops::fetch` → button next to Push/Pull. Refresh ahead/behind after.

## Phase 4 — Polish  ⬜
- Commit input: multiline body, char counter, **Amend** checkbox (`git commit --amend`), Ctrl+Enter to commit.
- Richer graph: drawn graph lines, author + relative time, click commit → diff (reuses Phase 2).
- Conflict files highlighted distinctly with a "resolve" affordance.
- Remember expanded/collapsed section state.

---

## Cross-cutting notes
- **Multi-device**: no hardcoded widths/paths/monitor assumptions. Width persisted relative to config, clamped 200–900.
- **File-size rule**: keep each new `git_sidebar/*.rs` < 500 lines; split draw helpers if a phase pushes a file over.
- **UI consistency**: use existing color consts (`ACCENT`/`GREEN`/`RED`/`BLUE`), `FONT`/`SMALL_FONT`, big-text preference.
- **Destructive ops** (discard, branch delete, stash drop): always confirm.
- Each phase ends with: `cargo build --release -p lntrn-terminal` + deploy via the `deploy-terminal` skill + manual smoke test.

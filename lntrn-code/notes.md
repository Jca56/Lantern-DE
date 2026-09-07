# Daily-driver stuff
  - Quick Open and Go to Symbol. Ctrl+P fuzzy-opens any file in the project. Ctrl+Shift+O lists functions and structs in the file via the language
    server's document symbols, and an Outline tab beside Files. The quick-open list exists already from the palette; it needs its own key and fuzzy
    ranking.
  - Multi-cursor. Ctrl+D selects the next occurrence of the word, Alt+click adds a caret, Ctrl+Shift+L one per match. The buffer has one replace
    primitive, so it's mostly cursor bookkeeping in the input path.
  - Find/replace in the whole project. The Search panel finds but can't replace. Replace All with a per-hit checkbox would round it out.

# Claude Code integration, since that's the whole point of the IDE
  - Session dock. A tile per connected Claude Code session: which project, what it's doing, whether it's waiting on a permission prompt. Click to
    jump to that terminal. The bridge already knows every session.
  - Diff review queue. When Claude proposes several diffs, a list to step through them with Accept/Reject, instead of one Diff tab at a time.
  - Ask about this. Right-click a diagnostic, a git hunk, or a selection and send it straight to the focused Claude session as a prompt, not just
    as an @-mention.

# Git, round three
  - Hunk staging. Stage or discard one hunk from the gutter mark or the diff view, instead of whole files.
  - Blame. Author and age in the gutter on hover, or a "Blame" tab.
  - Stash list with apply and drop, and a conflict view for merges.

---

# Look and feel

  - Editor color themes as files, the same way the shell themes are, so syntax colors travel with the theme. Right now they're a separate Settings
    group.
    
---

# 

When I unfold a heading in a .md file it automatically highlights everything below but only sometimes. Most of the time but not every single time. I would like it to not highlight anything, just open/close. And I can't write anything outside the last heading, without First making another heading. What if it stopped at `---` as well to mark the end of that section? 

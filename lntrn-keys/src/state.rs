//! In-memory state of the TUI — items + filter + selection cursor.

use crate::secret::Item;

pub struct State {
    pub all: Vec<Item>,
    pub filter: String,
    pub cursor: usize,
    pub mode: Mode,
    pub status: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mode {
    /// Normal list view.
    List,
    /// Typing in the search filter.
    Filtering,
    /// Add wizard, asking for fields one at a time.
    Adding(AddStage),
    /// Confirming a deletion.
    ConfirmDelete,
    /// Showing the secret value of the selected item.
    Revealing(String), // the secret value as text
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AddStage {
    /// Step 1 — name the key (what you'll search for later).
    Name(String),
    /// Step 2 — paste the secret value (typing is hidden).
    Secret { name: String, value: String },
}

impl State {
    pub fn new(all: Vec<Item>) -> Self {
        Self {
            all,
            filter: String::new(),
            cursor: 0,
            mode: Mode::List,
            status: "Welcome to lkeys — press 'a' to add, 'q' to quit.".into(),
        }
    }

    /// Indices of items matching the current filter.
    pub fn visible(&self) -> Vec<usize> {
        if self.filter.is_empty() {
            (0..self.all.len()).collect()
        } else {
            let needle = self.filter.to_lowercase();
            self.all.iter().enumerate()
                .filter(|(_, it)| {
                    it.label.to_lowercase().contains(&needle)
                        || it.attributes.values().any(|v| v.to_lowercase().contains(&needle))
                        || it.attributes.keys().any(|k| k.to_lowercase().contains(&needle))
                })
                .map(|(i, _)| i)
                .collect()
        }
    }

    pub fn selected(&self) -> Option<&Item> {
        let vis = self.visible();
        vis.get(self.cursor).and_then(|i| self.all.get(*i))
    }

    pub fn move_cursor(&mut self, delta: isize) {
        let n = self.visible().len();
        if n == 0 { self.cursor = 0; return; }
        let max = n - 1;
        let new = (self.cursor as isize + delta).max(0).min(max as isize) as usize;
        self.cursor = new;
    }

    pub fn set_status(&mut self, s: impl Into<String>) {
        self.status = s.into();
    }
}

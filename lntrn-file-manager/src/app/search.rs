//! Background recursive search — owns the worker thread, results queue,
//! and cancellation signal.

use super::{search_recursive, App};

impl App {
    pub fn start_search(&mut self) {
        self.searching = true;
        // Search takes the keyboard from the Save picker's name field (which
        // is focused from startup and would otherwise keep eating the keys).
        // Clicking the name field again gives it back.
        self.save_name_editing = false;
        self.save_name_selection = None;
        self.search_buf.clear();
        self.search_cursor = 0;
        self.search_results.clear();
        self.cancel_search();
    }

    pub fn cancel_search(&mut self) {
        // Signal any running search thread to stop
        if let Some(tx) = self.search_tx.take() {
            let _ = tx.send(());
        }
        self.search_rx = None;
    }

    pub fn close_search(&mut self) {
        self.cancel_search();
        self.searching = false;
        self.search_buf.clear();
        self.search_cursor = 0;
        self.search_results.clear();
    }

    pub fn run_search(&mut self) {
        self.cancel_search();
        self.search_results.clear();

        let query = self.search_buf.to_lowercase();
        if query.is_empty() {
            return;
        }

        let root = self.current_dir.clone();
        let (cancel_tx, cancel_rx) = std::sync::mpsc::channel::<()>();
        let (result_tx, result_rx) = std::sync::mpsc::channel::<crate::fs::FileEntry>();

        self.search_tx = Some(cancel_tx);
        self.search_rx = Some(result_rx);

        std::thread::spawn(move || {
            search_recursive(&root, &query, &result_tx, &cancel_rx);
        });
    }

    /// Poll for new search results from the background thread. True when
    /// results arrived or the search ended (the list has to be drawn again).
    pub fn poll_search(&mut self) -> bool {
        let Some(rx) = self.search_rx.as_ref() else {
            return false;
        };
        let before = self.search_results.len();
        // Drain all available results (non-blocking)
        let finished = loop {
            match rx.try_recv() {
                Ok(entry) => self.search_results.push(entry),
                Err(std::sync::mpsc::TryRecvError::Empty) => break false,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => break true,
            }
        };
        // The worker is done. Dropping the receiver tells the main loop there
        // is nothing left to poll, so it can stop redrawing every frame.
        if finished {
            self.search_rx = None;
            self.search_tx = None;
        }
        finished || self.search_results.len() != before
    }
}

//! Background recursive search — owns the worker thread, results queue,
//! and cancellation signal.

use super::{search_recursive, App};

impl App {
    pub fn start_search(&mut self) {
        self.searching = true;
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

    /// Poll for new search results from the background thread.
    pub fn poll_search(&mut self) {
        if let Some(ref rx) = self.search_rx {
            // Drain all available results (non-blocking)
            loop {
                match rx.try_recv() {
                    Ok(entry) => self.search_results.push(entry),
                    Err(_) => break,
                }
            }
        }
    }
}

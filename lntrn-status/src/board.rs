//! What a subsystem's thread shows the UI: its latest state behind a
//! lock, and the UI told whenever that state changed, never otherwise.

use std::sync::{Arc, Mutex, PoisonError};

/// Called from a subsystem's thread whenever what it shows changed: a UI
/// wakes its loop here (`move || waker.wake()`).
pub type Notify = Arc<dyn Fn() + Send + Sync>;

pub(crate) struct Board<T> {
    value: Mutex<T>,
    notify: Notify,
}

impl<T: Clone + PartialEq> Board<T> {
    pub fn new(value: T, notify: Notify) -> Arc<Board<T>> {
        Arc::new(Board { value: Mutex::new(value), notify })
    }

    pub fn get(&self) -> T {
        self.value.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    /// Change it with `f`; the UI hears of it only if that changed it.
    pub fn update(&self, f: impl FnOnce(&mut T)) {
        let mut value = self.value.lock().unwrap_or_else(PoisonError::into_inner);
        let before = value.clone();
        f(&mut value);
        let changed = *value != before;
        drop(value);
        if changed {
            (self.notify)();
        }
    }

    pub fn put(&self, new: T) {
        self.update(|value| *value = new);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    #[test]
    fn the_ui_hears_of_changes_alone() {
        let told = Arc::new(AtomicU32::new(0));
        let count = told.clone();
        let board = Board::new(1, Arc::new(move || {
            count.fetch_add(1, Ordering::Relaxed);
        }));
        board.put(1);
        board.update(|v| *v += 0);
        assert_eq!(told.load(Ordering::Relaxed), 0);
        board.put(2);
        board.update(|v| *v += 1);
        assert_eq!((board.get(), told.load(Ordering::Relaxed)), (3, 2));
    }
}

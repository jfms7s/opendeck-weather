//! The shared doorbell between the trackers, the property inspector and
//! the scheduler (`scheduler::run`): it rings when an instance appears or is
//! touched, which can bring a deadline forward, or when everything must be
//! redrawn now.

use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::Notify;

#[derive(Default)]
pub struct Wake {
    notify: Notify,
    refresh_all: AtomicBool,
}

impl Wake {
    /// Something changed that may move a deadline: look again.
    pub fn poke(&self) {
        // A stored permit, so a poke while the loop is busy isn't lost.
        self.notify.notify_one();
    }

    /// Re-render every instance now (e.g. the default location changed).
    pub fn refresh_all(&self) {
        self.refresh_all.store(true, Ordering::SeqCst);
        self.notify.notify_one();
    }

    /// Whether a redraw of everything was asked for since the last call.
    pub fn take_refresh_all(&self) -> bool {
        self.refresh_all.swap(false, Ordering::SeqCst)
    }

    /// Drops a pending redraw-everything (nothing is visible to redraw).
    pub fn clear_refresh_all(&self) {
        self.refresh_all.store(false, Ordering::SeqCst);
    }

    /// Waits for the next `poke` or `refresh_all` (or a stored one).
    pub async fn rung(&self) {
        self.notify.notified().await;
    }
}

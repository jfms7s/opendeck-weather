//! Registry of one action's live instances: their settings, their in-memory
//! view state `V` (scrolled hour/day/page, details toggle), and the card
//! each one last showed. Any interaction stamps the instance, and `expire`
//! snaps it back to its resting screen a while after the last touch - like
//! Elgato's plugin does after a press.
//!
//! Every change to an instance's settings or view gets a fresh revision, so
//! a render that started before the change can tell it is out of date
//! (`claim_frame`), and the card last sent lets an unchanged frame be
//! skipped instead of re-sent to OpenDeck.

use crate::card::Card;
use crate::model::Settings;
use crate::scheduler::Wake;
use dashmap::DashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use tokio::sync::{Mutex as AsyncMutex, MutexGuard};
use tokio::time::Instant;

/// Revisions are unique across every tracker and incarnation of an instance.
static NEXT_REVISION: AtomicU64 = AtomicU64::new(1);

fn next_revision() -> u64 {
    NEXT_REVISION.fetch_add(1, Ordering::Relaxed)
}

struct Tracked<V> {
    settings: Settings,
    view: V,
    touched: Option<Instant>,
    revision: u64,
    shown: Option<Card>,
}

/// What a render read before it started building its card.
#[derive(Debug, Clone, PartialEq)]
pub struct Snapshot<V> {
    pub settings: Settings,
    pub view: V,
    pub revision: u64,
}

/// Whether a freshly built card should be sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Frame {
    /// The instance is gone, its state changed while the card was being
    /// built (a newer render is on its way), or it already shows this card.
    Skip,
    /// Send it. `first`: nothing has been sent since the instance appeared.
    Send { first: bool },
}

pub struct Tracker<V> {
    instances: DashMap<String, Tracked<V>>,
    /// Pinged when an instance appears or is touched, so a scheduler
    /// sleeping until a later deadline (or parked) looks again.
    wake: Arc<Wake>,
    /// Held from claiming a frame until it is sent, so two renders of one
    /// instance can't reach OpenDeck in the opposite order.
    sending: AsyncMutex<()>,
}

impl<V: Copy + Default + PartialEq> Tracker<V> {
    pub fn new(wake: Arc<Wake>) -> Self {
        Tracker {
            instances: DashMap::new(),
            wake,
            sending: AsyncMutex::new(()),
        }
    }

    /// Starts tracking an instance (it appeared), or updates its settings.
    /// A settings change resets the view, since e.g. a new location makes a
    /// scrolled position meaningless; identical settings keep it. Either
    /// way the next frame is sent in full, as OpenDeck may have redrawn the
    /// key since.
    pub fn track(&self, id: &str, settings: Settings) {
        let mut entry = self
            .instances
            .entry(id.to_string())
            .or_insert_with(|| Tracked {
                settings: settings.clone(),
                view: V::default(),
                touched: None,
                revision: next_revision(),
                shown: None,
            });
        if entry.settings != settings {
            entry.settings = settings;
            entry.view = V::default();
            entry.touched = None;
            entry.revision = next_revision();
        }
        entry.shown = None;
        drop(entry);
        self.wake.poke();
    }

    pub fn untrack(&self, id: &str) {
        self.instances.remove(id);
    }

    pub fn snapshot(&self, id: &str) -> Option<Snapshot<V>> {
        self.instances.get(id).map(|t| Snapshot {
            settings: t.settings.clone(),
            view: t.view,
            revision: t.revision,
        })
    }

    /// Applies an interaction to an instance's view and restarts its
    /// revert timer. Returns false for an untracked instance.
    pub fn interact(&self, id: &str, f: impl FnOnce(V) -> V) -> bool {
        let Some(mut t) = self.instances.get_mut(id) else {
            return false;
        };
        t.view = f(t.view);
        t.touched = Some(Instant::now());
        t.revision = next_revision();
        drop(t);
        self.wake.poke();
        true
    }

    /// Resets every instance untouched for `after` back to its resting
    /// view, returning their ids so the caller can re-render them.
    pub fn expire(&self, after: Duration) -> Vec<String> {
        let now = Instant::now();
        let mut expired = Vec::new();
        for mut t in self.instances.iter_mut() {
            if t.touched.is_some_and(|at| now.duration_since(at) >= after) {
                t.view = V::default();
                t.touched = None;
                t.revision = next_revision();
                expired.push(t.key().clone());
            }
        }
        expired
    }

    /// When the earliest scrolled/toggled view is due to revert.
    pub fn next_expiry(&self, after: Duration) -> Option<Instant> {
        self.instances
            .iter()
            .filter_map(|t| t.touched.map(|at| at + after))
            .min()
    }

    pub fn ids(&self) -> Vec<String> {
        self.instances.iter().map(|t| t.key().clone()).collect()
    }

    pub fn is_empty(&self) -> bool {
        self.instances.is_empty()
    }

    /// Serializes claim-then-send for this action's instances.
    pub async fn sending(&self) -> MutexGuard<'_, ()> {
        self.sending.lock().await
    }

    /// Decides whether `card`, built from the snapshot at `revision`, should
    /// be sent, and if so records it as shown. Call while holding
    /// `sending()`, and `forget_frame` if the send then fails.
    pub fn claim_frame(&self, id: &str, revision: u64, card: &Card) -> Frame {
        let Some(mut t) = self.instances.get_mut(id) else {
            return Frame::Skip;
        };
        if t.revision != revision || t.shown.as_ref() == Some(card) {
            return Frame::Skip;
        }
        let first = t.shown.is_none();
        t.shown = Some(card.clone());
        Frame::Send { first }
    }

    /// The last claimed frame never arrived: send the next one in full.
    pub fn forget_frame(&self, id: &str) {
        if let Some(mut t) = self.instances.get_mut(id) {
            t.shown = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Units;
    use crate::views;

    const AFTER: Duration = Duration::from_secs(15);

    fn tracker() -> Tracker<i32> {
        Tracker::new(Arc::default())
    }

    fn scrolled(t: &Tracker<i32>) {
        t.interact("a", |_| 3);
    }

    #[test]
    fn track_untrack_round_trip() {
        let t = tracker();
        t.track("a", Settings::default());
        assert!(t.snapshot("a").is_some());
        assert!(!t.is_empty());
        t.untrack("a");
        assert!(t.snapshot("a").is_none());
        assert!(t.is_empty());
        assert!(!t.interact("a", |v| v));
    }

    #[test]
    fn identical_settings_keep_the_view_but_changed_settings_reset_it() {
        let t = tracker();
        t.track("a", Settings::default());
        scrolled(&t);
        t.track("a", Settings::default());
        assert_eq!(t.snapshot("a").unwrap().view, 3);

        let imperial = Settings {
            units: Units::Imperial,
            ..Settings::default()
        };
        t.track("a", imperial);
        assert_eq!(t.snapshot("a").unwrap().view, 0);
    }

    #[tokio::test(start_paused = true)]
    async fn views_revert_only_after_the_timeout_since_the_last_touch() {
        let t = tracker();
        t.track("a", Settings::default());
        t.track("b", Settings::default());
        assert_eq!(t.next_expiry(AFTER), None);
        scrolled(&t);
        assert_eq!(t.next_expiry(AFTER), Some(Instant::now() + AFTER));

        tokio::time::advance(Duration::from_secs(10)).await;
        assert!(t.expire(AFTER).is_empty());
        scrolled(&t); // touching again restarts the timer
        assert_eq!(t.next_expiry(AFTER), Some(Instant::now() + AFTER));

        tokio::time::advance(Duration::from_secs(10)).await;
        assert!(t.expire(AFTER).is_empty());

        tokio::time::advance(Duration::from_secs(5)).await;
        assert_eq!(t.expire(AFTER), vec!["a".to_string()]);
        assert_eq!(t.snapshot("a").unwrap().view, 0);
        assert!(t.expire(AFTER).is_empty(), "already reverted");
        assert_eq!(t.next_expiry(AFTER), None);
    }

    #[test]
    fn unchanged_frames_are_skipped_and_the_first_one_is_flagged() {
        let t = tracker();
        t.track("a", Settings::default());
        let rev = t.snapshot("a").unwrap().revision;
        let card = views::no_location();
        assert_eq!(t.claim_frame("a", rev, &card), Frame::Send { first: true });
        assert_eq!(t.claim_frame("a", rev, &card), Frame::Skip);
        let other = views::no_data("Lisbon");
        assert_eq!(
            t.claim_frame("a", rev, &other),
            Frame::Send { first: false }
        );
        // Appearing again (OpenDeck may have redrawn the key) resends.
        t.track("a", Settings::default());
        assert_eq!(t.claim_frame("a", rev, &other), Frame::Send { first: true });
        t.forget_frame("a");
        assert_eq!(t.claim_frame("a", rev, &other), Frame::Send { first: true });
    }

    #[test]
    fn a_frame_built_before_a_change_is_dropped() {
        let t = tracker();
        t.track("a", Settings::default());
        let before = t.snapshot("a").unwrap().revision;
        scrolled(&t);
        let card = views::no_location();
        assert_eq!(t.claim_frame("a", before, &card), Frame::Skip);
        let after = t.snapshot("a").unwrap().revision;
        assert_eq!(
            t.claim_frame("a", after, &card),
            Frame::Send { first: true }
        );
        assert_eq!(t.claim_frame("gone", after, &card), Frame::Skip);
    }
}

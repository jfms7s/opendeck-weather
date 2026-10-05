//! The one background loop for every action: re-renders each instance once
//! per `REFRESH_EVERY` (mostly cache hits; it keeps the current hour rolling
//! and picks up fresh data soon after the cache expires), and reverts a
//! scrolled or toggled view `VIEW_TIMEOUT` after its last touch.
//!
//! It sleeps until the next of those deadlines instead of ticking, and
//! parks entirely while no instance is visible. `Wake` gets it to look
//! again when an instance appears or is touched (which can bring a
//! deadline forward), or when everything must be redrawn now.

use crate::services::POLICY;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::sync::Notify;
use tokio::time::Instant;

/// How long a scrolled/toggled screen stays up after the last interaction.
pub const VIEW_TIMEOUT: Duration = Duration::from_secs(15);
/// How often every visible instance is re-rendered. The README promises
/// both the rolling current hour and recovery from an outage "within about
/// a minute" on the strength of this.
pub const REFRESH_EVERY: Duration = Duration::from_secs(60);

// After a failed fetch, the next refresh must be allowed to retry, or
// recovery takes two refresh periods instead of one.
const _: () = assert!(POLICY.retry_after.as_millis() < REFRESH_EVERY.as_millis());

/// Shared doorbell between the trackers, the inspector and the scheduler.
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

    #[cfg(test)]
    pub fn take_refresh_all(&self) -> bool {
        self.refresh_all.swap(false, Ordering::SeqCst)
    }
}

/// What the scheduler needs from one action.
pub trait Scheduled: Send + Sync {
    fn has_instances(&self) -> bool;
    /// When the earliest scrolled/toggled view is due to revert.
    fn next_expiry(&self) -> Option<Instant>;
    /// Reverts views untouched for `VIEW_TIMEOUT`; returns their ids.
    fn expire(&self) -> Vec<String>;
    fn ids(&self) -> Vec<String>;
    /// Renders one instance in the background.
    fn render_soon(&self, id: String);
}

/// The earlier of the next refresh and the next view revert.
fn next_deadline(refresh_at: Instant, next_expiry: Option<Instant>) -> Instant {
    next_expiry.map_or(refresh_at, |e| e.min(refresh_at))
}

pub async fn run(actions: Vec<Arc<dyn Scheduled>>, wake: Arc<Wake>) {
    let mut refresh_at: Option<Instant> = None;
    loop {
        if !actions.iter().any(|a| a.has_instances()) {
            // Nothing visible: no timers at all until something appears
            // (which renders itself, so the refresh clock starts afresh).
            refresh_at = None;
            wake.refresh_all.store(false, Ordering::SeqCst);
            wake.notify.notified().await;
            continue;
        }
        let due_refresh = *refresh_at.get_or_insert_with(|| Instant::now() + REFRESH_EVERY);
        let deadline = next_deadline(
            due_refresh,
            actions.iter().filter_map(|a| a.next_expiry()).min(),
        );

        let forced = tokio::select! {
            () = tokio::time::sleep_until(deadline) => false,
            () = wake.notify.notified() => wake.refresh_all.swap(false, Ordering::SeqCst),
        };
        let now = Instant::now();
        let refresh = forced || now >= due_refresh;
        if refresh {
            refresh_at = Some(now + REFRESH_EVERY);
        }
        for action in &actions {
            let reverted = action.expire();
            let due = if refresh { action.ids() } else { reverted };
            for id in due {
                action.render_soon(id);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Settings;
    use crate::tracker::Tracker;
    use std::sync::Mutex;
    use std::sync::atomic::AtomicU32;

    /// An action whose renders are just recorded.
    struct Fake {
        tracker: Tracker<i32>,
        rendered: Mutex<Vec<String>>,
        /// Times the loop looked at this action: a proxy for wakeups.
        looks: AtomicU32,
    }

    impl Fake {
        fn new(wake: &Arc<Wake>) -> Arc<Self> {
            Arc::new(Fake {
                tracker: Tracker::new(wake.clone()),
                rendered: Mutex::default(),
                looks: AtomicU32::new(0),
            })
        }

        fn take_rendered(&self) -> Vec<String> {
            let mut r = std::mem::take(&mut *self.rendered.lock().unwrap());
            r.sort();
            r
        }
    }

    impl Scheduled for Fake {
        fn has_instances(&self) -> bool {
            self.looks.fetch_add(1, Ordering::SeqCst);
            !self.tracker.is_empty()
        }
        fn next_expiry(&self) -> Option<Instant> {
            self.tracker.next_expiry(VIEW_TIMEOUT)
        }
        fn expire(&self) -> Vec<String> {
            self.tracker.expire(VIEW_TIMEOUT)
        }
        fn ids(&self) -> Vec<String> {
            self.tracker.ids()
        }
        fn render_soon(&self, id: String) {
            self.rendered.lock().unwrap().push(id);
        }
    }

    async fn settle() {
        for _ in 0..10 {
            tokio::task::yield_now().await;
        }
    }

    async fn advance(secs: u64) {
        tokio::time::advance(Duration::from_secs(secs)).await;
        settle().await;
    }

    fn start(fake: &Arc<Fake>, wake: &Arc<Wake>) {
        let actions: Vec<Arc<dyn Scheduled>> = vec![fake.clone()];
        tokio::spawn(run(actions, wake.clone()));
    }

    #[tokio::test(start_paused = true)]
    async fn with_nothing_visible_it_parks_instead_of_ticking() {
        let wake = Arc::new(Wake::default());
        let fake = Fake::new(&wake);
        start(&fake, &wake);
        advance(600).await;
        assert!(fake.take_rendered().is_empty());
        assert!(
            fake.looks.load(Ordering::SeqCst) <= 2,
            "woke up while parked"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn every_instance_refreshes_once_a_minute_and_not_in_between() {
        let wake = Arc::new(Wake::default());
        let fake = Fake::new(&wake);
        start(&fake, &wake);
        fake.tracker.track("a", Settings::default());
        fake.tracker.track("b", Settings::default());
        settle().await;
        advance(59).await;
        assert!(fake.take_rendered().is_empty());
        advance(1).await;
        assert_eq!(fake.take_rendered(), ["a", "b"]);
        advance(60).await;
        assert_eq!(fake.take_rendered(), ["a", "b"]);
        assert!(
            fake.looks.load(Ordering::SeqCst) <= 8,
            "ticked between deadlines"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_touched_view_reverts_after_the_timeout_without_waiting_for_a_refresh() {
        let wake = Arc::new(Wake::default());
        let fake = Fake::new(&wake);
        start(&fake, &wake);
        fake.tracker.track("a", Settings::default());
        fake.tracker.track("b", Settings::default());
        advance(20).await;
        fake.tracker.interact("a", |_| 3); // pokes the scheduler
        settle().await;
        advance(14).await;
        assert!(fake.take_rendered().is_empty());
        advance(1).await;
        assert_eq!(fake.take_rendered(), ["a"], "only the touched one reverts");
        assert_eq!(fake.tracker.snapshot("a").unwrap().view, 0);
    }

    #[tokio::test(start_paused = true)]
    async fn refresh_all_redraws_everything_at_once() {
        let wake = Arc::new(Wake::default());
        let fake = Fake::new(&wake);
        start(&fake, &wake);
        fake.tracker.track("a", Settings::default());
        advance(5).await;
        wake.refresh_all();
        settle().await;
        assert_eq!(fake.take_rendered(), ["a"]);
    }

    #[test]
    fn the_deadline_is_the_earlier_of_refresh_and_revert() {
        let now = Instant::now();
        let later = now + Duration::from_secs(5);
        assert_eq!(next_deadline(later, None), later);
        assert_eq!(next_deadline(later, Some(now)), now);
        assert_eq!(next_deadline(now, Some(later)), now);
    }

    #[test]
    fn the_readme_cadence_holds() {
        // "re-renders every minute", "recover within about a minute",
        // "returns to its resting screen 15 seconds after your last press".
        assert_eq!(REFRESH_EVERY, Duration::from_secs(60));
        assert_eq!(VIEW_TIMEOUT, Duration::from_secs(15));
    }
}

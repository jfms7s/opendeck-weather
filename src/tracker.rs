//! Registry of an action's live instances: their settings plus in-memory
//! view state (scrolled hour/day/page, details toggle). Any interaction
//! stamps the instance, and `expire` snaps it back to its resting screen a
//! while after the last touch - like Elgato's plugin does after a press.

use crate::model::Settings;
use crate::views::View;
use dashmap::DashMap;
use std::time::Duration;
use tokio::time::Instant;

struct Tracked {
    settings: Settings,
    view: View,
    touched: Option<Instant>,
}

#[derive(Default)]
pub struct Tracker {
    instances: DashMap<String, Tracked>,
}

impl Tracker {
    /// Starts tracking an instance, or updates its settings. A settings
    /// change resets the view, since e.g. a new location makes a scrolled
    /// position meaningless; re-sending identical settings keeps it.
    pub fn track(&self, id: &str, settings: Settings) {
        if let Some(t) = self.instances.get(id)
            && t.settings == settings
        {
            return;
        }
        self.instances.insert(
            id.to_string(),
            Tracked {
                settings,
                view: View::default(),
                touched: None,
            },
        );
    }

    pub fn untrack(&self, id: &str) {
        self.instances.remove(id);
    }

    pub fn get(&self, id: &str) -> Option<(Settings, View)> {
        self.instances.get(id).map(|t| (t.settings.clone(), t.view))
    }

    /// Applies an interaction to an instance's view and restarts its
    /// revert timer. Returns false for an untracked instance.
    pub fn interact(&self, id: &str, f: impl FnOnce(&mut View)) -> bool {
        let Some(mut t) = self.instances.get_mut(id) else {
            return false;
        };
        f(&mut t.view);
        t.touched = Some(Instant::now());
        true
    }

    /// Resets every instance untouched for `after` back to its resting
    /// view, returning their ids so the caller can re-render them.
    pub fn expire(&self, after: Duration) -> Vec<String> {
        let now = Instant::now();
        let mut expired = Vec::new();
        for mut t in self.instances.iter_mut() {
            if t.touched.is_some_and(|at| now.duration_since(at) >= after) {
                t.view = View::default();
                t.touched = None;
                expired.push(t.key().clone());
            }
        }
        expired
    }

    pub fn ids(&self) -> Vec<String> {
        self.instances.iter().map(|t| t.key().clone()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Units;

    const AFTER: Duration = Duration::from_secs(15);

    fn scrolled(t: &Tracker) {
        t.interact("a", |v| v.offset = 3);
    }

    #[test]
    fn track_untrack_round_trip() {
        let t = Tracker::default();
        t.track("a", Settings::default());
        assert!(t.get("a").is_some());
        t.untrack("a");
        assert!(t.get("a").is_none());
        assert!(!t.interact("a", |_| {}));
    }

    #[test]
    fn identical_settings_keep_the_view_but_changed_settings_reset_it() {
        let t = Tracker::default();
        t.track("a", Settings::default());
        scrolled(&t);
        t.track("a", Settings::default());
        assert_eq!(t.get("a").unwrap().1.offset, 3);

        let imperial = Settings {
            units: Units::Imperial,
            ..Settings::default()
        };
        t.track("a", imperial);
        assert_eq!(t.get("a").unwrap().1, View::default());
    }

    #[tokio::test(start_paused = true)]
    async fn views_revert_only_after_the_timeout_since_the_last_touch() {
        let t = Tracker::default();
        t.track("a", Settings::default());
        t.track("b", Settings::default());
        scrolled(&t);

        tokio::time::advance(Duration::from_secs(10)).await;
        assert!(t.expire(AFTER).is_empty());
        scrolled(&t); // touching again restarts the timer

        tokio::time::advance(Duration::from_secs(10)).await;
        assert!(t.expire(AFTER).is_empty());

        tokio::time::advance(Duration::from_secs(5)).await;
        assert_eq!(t.expire(AFTER), vec!["a".to_string()]);
        assert_eq!(t.get("a").unwrap().1, View::default());
        assert!(t.expire(AFTER).is_empty(), "already reverted");
    }
}

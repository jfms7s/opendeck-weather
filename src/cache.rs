//! A small keyed TTL cache in front of Open-Meteo, so any number of keys
//! and dials showing the same place share one request per refresh window.
//!
//! - Fresh values (younger than `ttl`) are returned without a request.
//! - Concurrent callers with nothing usable for a key wait on one in-flight
//!   fetch instead of each firing their own (per-key async fetch guard);
//!   callers with a usable value get it at once, so a press never waits
//!   behind a refresh that hangs.
//! - A failed fetch falls back to the last good value while it is younger
//!   than `stale_after`, so a network blip doesn't blank every key.
//! - After a failure, the key isn't retried for `retry_after` (counted from
//!   when the failed attempt started), so a dead network doesn't turn every
//!   render into a request storm.
//! - Every value comes back as `Cached`, which says whether it is past its
//!   TTL (a fallback after a failed refresh), so the card can say so.
//! - `refresh` fetches even when the value is fresh (a "refresh now"
//!   gesture), but keeps everything else: the backoff, the stale fallback,
//!   and not waiting behind an in-flight fetch.

use std::collections::HashMap;
use std::fmt::Display;
use std::hash::Hash;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::Mutex as AsyncMutex;
use tokio::time::Instant;

#[derive(Debug, Clone, Copy)]
pub struct CachePolicy {
    pub ttl: Duration,
    pub retry_after: Duration,
    pub stale_after: Duration,
}

struct Slot<V, E> {
    value: Option<(Instant, Arc<V>)>,
    last_error: Option<(Instant, E)>,
    /// Finished fetch attempts, so a refresh that waited on another
    /// caller's fetch can tell it already has a newer answer.
    attempts: u64,
}

/// One key's state and its fetch guard. `state` is a std mutex, only ever
/// locked briefly and never across an `.await`; `fetching` is held for the
/// whole fetch, so at most one request per key is in flight.
struct SlotCell<V, E> {
    state: Mutex<Slot<V, E>>,
    fetching: AsyncMutex<()>,
}

impl<V, E> SlotCell<V, E> {
    fn state(&self) -> std::sync::MutexGuard<'_, Slot<V, E>> {
        self.state.lock().expect("cache slot lock poisoned")
    }
}

type SharedSlot<V, E> = Arc<SlotCell<V, E>>;

/// A value from the cache, and whether it is older than the TTL - served
/// only because refreshing it failed - so callers can mark it as old.
#[derive(Debug)]
pub struct Cached<V> {
    pub value: Arc<V>,
    pub stale: bool,
}

pub struct Cache<K, V, E> {
    policy: CachePolicy,
    slots: Mutex<HashMap<K, SharedSlot<V, E>>>,
}

impl<K: Eq + Hash + Clone, V, E: Clone + Display> Cache<K, V, E> {
    pub fn new(policy: CachePolicy) -> Self {
        Self {
            policy,
            slots: Mutex::new(HashMap::new()),
        }
    }

    fn slot(&self, key: &K) -> SharedSlot<V, E> {
        let mut slots = self.slots.lock().expect("cache map lock poisoned");
        slots
            .entry(key.clone())
            .or_insert_with(|| {
                Arc::new(SlotCell {
                    state: Mutex::new(Slot {
                        value: None,
                        last_error: None,
                        attempts: 0,
                    }),
                    fetching: AsyncMutex::new(()),
                })
            })
            .clone()
    }

    /// The last good value, if it is still young enough to show at `now`.
    fn usable(&self, slot: &Slot<V, E>, now: Instant, stale: bool) -> Option<Cached<V>> {
        slot.value
            .as_ref()
            .filter(|(at, _)| now.duration_since(*at) < self.policy.stale_after)
            .map(|(_, v)| Cached {
                value: v.clone(),
                stale,
            })
    }

    /// The answer when no fetch is due: the value is fresh (unless
    /// `refreshing`), or the key is backing off after a failure. `None`
    /// means it's time to fetch.
    fn settled(
        &self,
        slot: &Slot<V, E>,
        now: Instant,
        refreshing: bool,
    ) -> Option<Result<Cached<V>, E>> {
        if !refreshing
            && let Some((at, v)) = &slot.value
            && now.duration_since(*at) < self.policy.ttl
        {
            return Some(Ok(Cached {
                value: v.clone(),
                stale: false,
            }));
        }
        if let Some((at, e)) = &slot.last_error
            && now.duration_since(*at) < self.policy.retry_after
        {
            return Some(self.usable(slot, now, true).ok_or_else(|| e.clone()));
        }
        None
    }

    pub async fn get<F, Fut>(&self, key: &K, fetch: F) -> Result<Cached<V>, E>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<V, E>>,
    {
        self.fetch_unless_settled(key, fetch, false).await
    }

    /// Like `get`, but fetches even while the value is fresh. Still backs
    /// off after a failure, and still answers with the previous value when
    /// the fetch fails or another caller's fetch is in flight.
    pub async fn refresh<F, Fut>(&self, key: &K, fetch: F) -> Result<Cached<V>, E>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<V, E>>,
    {
        self.fetch_unless_settled(key, fetch, true).await
    }

    async fn fetch_unless_settled<F, Fut>(
        &self,
        key: &K,
        fetch: F,
        refreshing: bool,
    ) -> Result<Cached<V>, E>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<V, E>>,
    {
        let cell = self.slot(key);

        let (guard, attempts_seen) = {
            let state = cell.state();
            let now = Instant::now();
            if let Some(answer) = self.settled(&state, now, refreshing) {
                return answer;
            }
            let guard = match cell.fetching.try_lock() {
                Ok(guard) => Some(guard),
                Err(_) => {
                    // Someone else is fetching. Don't wait for them if there
                    // is something to show; it's gray only if the previous
                    // attempt failed, so a healthy refresh doesn't flash gray.
                    let stale = state.last_error.is_some();
                    if let Some(cached) = self.usable(&state, now, stale) {
                        return Ok(cached);
                    }
                    None
                }
            };
            (guard, state.attempts)
        };
        let _guard = match guard {
            Some(guard) => guard,
            None => cell.fetching.lock().await,
        };

        // Another caller may have just finished a fetch while we waited;
        // its answer is new enough for a refresh too.
        let started = {
            let state = cell.state();
            let now = Instant::now();
            let still_refreshing = refreshing && state.attempts == attempts_seen;
            if let Some(answer) = self.settled(&state, now, still_refreshing) {
                return answer;
            }
            now
        };

        // Back off from when this attempt started: a request that hangs until
        // its timeout must not push the next retry past the next refresh.
        let result = fetch().await;
        let mut state = cell.state();
        state.attempts += 1;
        match result {
            Ok(v) => {
                let v = Arc::new(v);
                state.value = Some((Instant::now(), v.clone()));
                state.last_error = None;
                Ok(Cached {
                    value: v,
                    stale: false,
                })
            }
            Err(e) => {
                state.last_error = Some((started, e.clone()));
                let fallback = self.usable(&state, started, true);
                if fallback.is_some() {
                    // Callers only see the error once the stale value runs
                    // out, so say here that it's being papered over.
                    log::warn!("refresh failed, showing older data: {e}");
                }
                fallback.ok_or(e)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    const POLICY: CachePolicy = CachePolicy {
        ttl: Duration::from_secs(600),
        retry_after: Duration::from_secs(60),
        stale_after: Duration::from_secs(3600),
    };

    type TestCache = Cache<&'static str, u32, String>;

    async fn get(
        cache: &TestCache,
        calls: &AtomicU32,
        result: Result<u32, String>,
    ) -> Result<u32, String> {
        cache
            .get(&"k", || async {
                calls.fetch_add(1, Ordering::SeqCst);
                result
            })
            .await
            .map(|c| *c.value)
    }

    #[tokio::test(start_paused = true)]
    async fn fresh_values_are_served_without_refetching() {
        let cache = TestCache::new(POLICY);
        let calls = AtomicU32::new(0);
        assert_eq!(get(&cache, &calls, Ok(1)).await, Ok(1));
        tokio::time::advance(Duration::from_secs(599)).await;
        assert_eq!(get(&cache, &calls, Ok(2)).await, Ok(1));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn expired_values_are_refetched() {
        let cache = TestCache::new(POLICY);
        let calls = AtomicU32::new(0);
        get(&cache, &calls, Ok(1)).await.unwrap();
        tokio::time::advance(Duration::from_secs(601)).await;
        assert_eq!(get(&cache, &calls, Ok(2)).await, Ok(2));
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    async fn refresh(
        cache: &TestCache,
        calls: &AtomicU32,
        result: Result<u32, String>,
    ) -> Result<u32, String> {
        cache
            .refresh(&"k", || async {
                calls.fetch_add(1, Ordering::SeqCst);
                result
            })
            .await
            .map(|c| *c.value)
    }

    #[tokio::test(start_paused = true)]
    async fn refresh_fetches_even_inside_the_ttl() {
        let cache = TestCache::new(POLICY);
        let calls = AtomicU32::new(0);
        get(&cache, &calls, Ok(1)).await.unwrap();
        tokio::time::advance(Duration::from_secs(5)).await;
        assert_eq!(refresh(&cache, &calls, Ok(2)).await, Ok(2));
        assert_eq!(
            get(&cache, &calls, Ok(3)).await,
            Ok(2),
            "the new value is cached"
        );
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test(start_paused = true)]
    async fn refresh_keeps_the_backoff_after_a_failure() {
        let cache = TestCache::new(POLICY);
        let calls = AtomicU32::new(0);
        get(&cache, &calls, Ok(1)).await.unwrap();
        tokio::time::advance(Duration::from_secs(601)).await;
        assert_eq!(get(&cache, &calls, Err("down".into())).await, Ok(1));
        tokio::time::advance(Duration::from_secs(30)).await;
        let stale = cache.refresh(&"k", || async { Ok(9) }).await.unwrap();
        assert_eq!((*stale.value, stale.stale), (1, true));
        assert_eq!(
            calls.load(Ordering::SeqCst),
            2,
            "no fetch while backing off"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_failed_refresh_keeps_serving_the_previous_value() {
        let cache = TestCache::new(POLICY);
        let calls = AtomicU32::new(0);
        get(&cache, &calls, Ok(1)).await.unwrap();
        assert_eq!(refresh(&cache, &calls, Err("down".into())).await, Ok(1));
        assert_eq!(get(&cache, &calls, Ok(2)).await, Ok(1));
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test(start_paused = true)]
    async fn a_refresh_that_waited_on_a_fetch_uses_its_result() {
        let cache = Arc::new(TestCache::new(POLICY));
        let calls = Arc::new(AtomicU32::new(0));
        let slow = {
            let (cache, calls) = (cache.clone(), calls.clone());
            tokio::spawn(async move {
                cache
                    .get(&"k", || async {
                        calls.fetch_add(1, Ordering::SeqCst);
                        tokio::time::sleep(Duration::from_secs(1)).await;
                        Ok(1)
                    })
                    .await
                    .map(|c| *c.value)
            })
        };
        tokio::task::yield_now().await;
        // Nothing to show yet, so the refresh waits on the get's fetch.
        assert_eq!(refresh(&cache, &calls, Ok(2)).await, Ok(1));
        assert_eq!(slow.await.unwrap(), Ok(1));
        assert_eq!(calls.load(Ordering::SeqCst), 1, "one request, not two");
    }

    #[tokio::test(start_paused = true)]
    async fn a_failed_refresh_falls_back_to_the_stale_value() {
        let cache = TestCache::new(POLICY);
        let calls = AtomicU32::new(0);
        get(&cache, &calls, Ok(1)).await.unwrap();
        tokio::time::advance(Duration::from_secs(601)).await;
        assert_eq!(get(&cache, &calls, Err("down".into())).await, Ok(1));
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test(start_paused = true)]
    async fn while_backing_off_the_stale_value_is_served_without_refetching() {
        // Every render during an outage after the first failure goes here.
        let cache = TestCache::new(POLICY);
        let calls = AtomicU32::new(0);
        get(&cache, &calls, Ok(1)).await.unwrap();
        tokio::time::advance(Duration::from_secs(601)).await;
        assert_eq!(get(&cache, &calls, Err("down".into())).await, Ok(1));
        tokio::time::advance(Duration::from_secs(30)).await;
        assert_eq!(get(&cache, &calls, Ok(2)).await, Ok(1));
        assert_eq!(
            calls.load(Ordering::SeqCst),
            2,
            "no fetch while backing off"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn while_backing_off_a_value_past_stale_after_is_not_served() {
        let cache = TestCache::new(POLICY);
        let calls = AtomicU32::new(0);
        get(&cache, &calls, Ok(1)).await.unwrap();
        tokio::time::advance(Duration::from_secs(3590)).await;
        assert_eq!(get(&cache, &calls, Err("down".into())).await, Ok(1));
        tokio::time::advance(Duration::from_secs(20)).await;
        assert_eq!(get(&cache, &calls, Ok(2)).await, Err("down".into()));
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test(start_paused = true)]
    async fn a_slow_failure_backs_off_from_when_the_fetch_started() {
        // A request that hangs until its timeout: the next attempt is due
        // `retry_after` after the attempt began, not after it gave up, or a
        // once-a-minute refresh would only retry every other minute.
        let cache = TestCache::new(POLICY);
        let calls = AtomicU32::new(0);
        let slow_failure = cache.get(&"k", || async {
            calls.fetch_add(1, Ordering::SeqCst);
            tokio::time::sleep(Duration::from_secs(10)).await;
            Err::<u32, _>("timeout".to_string())
        });
        assert!(slow_failure.await.is_err());
        tokio::time::advance(Duration::from_secs(50)).await; // 60s since the start
        assert_eq!(get(&cache, &calls, Ok(1)).await, Ok(1));
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test(start_paused = true)]
    async fn concurrent_callers_share_one_in_flight_fetch() {
        let cache = TestCache::new(POLICY);
        let calls = AtomicU32::new(0);
        let fetch = || async {
            calls.fetch_add(1, Ordering::SeqCst);
            tokio::time::sleep(Duration::from_secs(1)).await;
            Ok::<_, String>(7)
        };
        let (a, b) = tokio::join!(cache.get(&"k", fetch), cache.get(&"k", fetch));
        assert_eq!((*a.unwrap().value, *b.unwrap().value), (7, 7));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    /// Starts a refresh that hangs for 10 s and then fails, and 2 s into it a
    /// second `get` (a press). Returns what the press got and how long it took.
    async fn press_during_a_hanging_refresh(
        cache: &TestCache,
        calls: &AtomicU32,
    ) -> (Result<(u32, bool), String>, Duration) {
        let hanging_refresh = cache.get(&"k", || async {
            calls.fetch_add(1, Ordering::SeqCst);
            tokio::time::sleep(Duration::from_secs(10)).await;
            Err::<u32, _>("timeout".to_string())
        });
        let press = async {
            tokio::time::sleep(Duration::from_secs(2)).await;
            let started = Instant::now();
            let got = cache
                .get(&"k", || async {
                    calls.fetch_add(1, Ordering::SeqCst);
                    Ok::<_, String>(9)
                })
                .await
                .map(|c| (*c.value, c.stale));
            (got, started.elapsed())
        };
        let (refresh, press) = tokio::join!(hanging_refresh, press);
        assert!(refresh.is_ok(), "the refresh falls back to the old value");
        press
    }

    #[tokio::test(start_paused = true)]
    async fn a_caller_with_a_usable_value_does_not_wait_for_an_in_flight_fetch() {
        let cache = TestCache::new(POLICY);
        let calls = AtomicU32::new(0);
        get(&cache, &calls, Ok(1)).await.unwrap();
        tokio::time::advance(Duration::from_secs(601)).await;
        let (got, waited) = press_during_a_hanging_refresh(&cache, &calls).await;
        assert_eq!(waited, Duration::ZERO, "the press waited for the fetch");
        assert_eq!(
            got,
            Ok((1, false)),
            "a healthy refresh isn't shown as stale"
        );
        assert_eq!(calls.load(Ordering::SeqCst), 2, "the press didn't fetch");
    }

    #[tokio::test(start_paused = true)]
    async fn after_a_failed_attempt_a_caller_that_does_not_wait_gets_a_stale_value() {
        let cache = TestCache::new(POLICY);
        let calls = AtomicU32::new(0);
        get(&cache, &calls, Ok(1)).await.unwrap();
        tokio::time::advance(Duration::from_secs(601)).await;
        assert_eq!(get(&cache, &calls, Err("down".into())).await, Ok(1));
        tokio::time::advance(Duration::from_secs(61)).await; // past retry_after
        let (got, waited) = press_during_a_hanging_refresh(&cache, &calls).await;
        assert_eq!(waited, Duration::ZERO, "the press waited for the fetch");
        assert_eq!(got, Ok((1, true)));
        assert_eq!(calls.load(Ordering::SeqCst), 3, "the press didn't fetch");
    }

    #[tokio::test(start_paused = true)]
    async fn values_served_past_their_ttl_are_reported_stale() {
        let cache = TestCache::new(POLICY);
        let get_flag = |result: Result<u32, String>| {
            let cache = &cache;
            async move {
                cache
                    .get(&"k", || async { result })
                    .await
                    .map(|c| (*c.value, c.stale))
            }
        };
        assert_eq!(get_flag(Ok(1)).await, Ok((1, false)));
        tokio::time::advance(Duration::from_secs(599)).await;
        assert_eq!(get_flag(Ok(9)).await, Ok((1, false)), "within the TTL");
        tokio::time::advance(Duration::from_secs(2)).await;
        assert_eq!(get_flag(Err("down".into())).await, Ok((1, true)));
        assert_eq!(get_flag(Ok(9)).await, Ok((1, true)), "backing off");
        tokio::time::advance(Duration::from_secs(60)).await;
        assert_eq!(get_flag(Ok(2)).await, Ok((2, false)), "recovered");
    }

    #[tokio::test(start_paused = true)]
    async fn stale_values_expire_eventually() {
        let cache = TestCache::new(POLICY);
        let calls = AtomicU32::new(0);
        get(&cache, &calls, Ok(1)).await.unwrap();
        tokio::time::advance(Duration::from_secs(3601)).await;
        assert_eq!(
            get(&cache, &calls, Err("down".into())).await,
            Err("down".into())
        );
    }

    #[tokio::test(start_paused = true)]
    async fn failures_are_not_retried_until_retry_after() {
        let cache = TestCache::new(POLICY);
        let calls = AtomicU32::new(0);
        assert!(get(&cache, &calls, Err("down".into())).await.is_err());
        tokio::time::advance(Duration::from_secs(30)).await;
        assert_eq!(
            get(&cache, &calls, Ok(1)).await,
            Err("down".into()),
            "should still be backing off"
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        tokio::time::advance(Duration::from_secs(31)).await;
        assert_eq!(get(&cache, &calls, Ok(1)).await, Ok(1));
    }

    #[tokio::test(start_paused = true)]
    async fn different_keys_are_cached_independently() {
        let cache = TestCache::new(POLICY);
        let a = cache
            .get(&"a", || async { Ok::<_, String>(1) })
            .await
            .unwrap();
        let b = cache
            .get(&"b", || async { Ok::<_, String>(2) })
            .await
            .unwrap();
        assert_eq!((*a.value, *b.value), (1, 2));
    }
}

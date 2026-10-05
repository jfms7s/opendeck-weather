//! A small keyed TTL cache in front of Open-Meteo, so any number of keys
//! and dials showing the same place share one request per refresh window.
//!
//! - Fresh values (younger than `ttl`) are returned without a request.
//! - Concurrent callers for the same key wait on one in-flight fetch
//!   instead of each firing their own (per-key async lock).
//! - A failed fetch falls back to the last good value while it is younger
//!   than `stale_after`, so a network blip doesn't blank every key.
//! - After a failure, the key isn't retried for `retry_after` (counted from
//!   when the failed attempt started), so a dead network doesn't turn every
//!   render into a request storm.
//! - Every value comes back as `Cached`, which says whether it is past its
//!   TTL (a fallback after a failed refresh), so the card can say so.

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
}

type SharedSlot<V, E> = Arc<AsyncMutex<Slot<V, E>>>;

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
                Arc::new(AsyncMutex::new(Slot {
                    value: None,
                    last_error: None,
                }))
            })
            .clone()
    }

    pub async fn get<F, Fut>(&self, key: &K, fetch: F) -> Result<Cached<V>, E>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<V, E>>,
    {
        let slot = self.slot(key);
        let mut slot = slot.lock().await;
        let now = Instant::now();
        let p = self.policy;

        let usable = |slot: &Slot<V, E>| {
            slot.value
                .as_ref()
                .filter(|(at, _)| now.duration_since(*at) < p.stale_after)
                .map(|(_, v)| Cached {
                    value: v.clone(),
                    stale: true,
                })
        };

        if let Some((at, v)) = &slot.value
            && now.duration_since(*at) < p.ttl
        {
            return Ok(Cached {
                value: v.clone(),
                stale: false,
            });
        }
        if let Some((at, e)) = &slot.last_error
            && now.duration_since(*at) < p.retry_after
        {
            return usable(&slot).ok_or_else(|| e.clone());
        }

        // Back off from when this attempt started: a request that hangs until
        // its timeout must not push the next retry past the next refresh.
        let started = Instant::now();
        match fetch().await {
            Ok(v) => {
                let v = Arc::new(v);
                slot.value = Some((Instant::now(), v.clone()));
                slot.last_error = None;
                Ok(Cached {
                    value: v,
                    stale: false,
                })
            }
            Err(e) => {
                slot.last_error = Some((started, e.clone()));
                let fallback = usable(&slot);
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

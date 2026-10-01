//! A small keyed TTL cache in front of Open-Meteo, so any number of keys
//! and dials showing the same place share one request per refresh window.
//!
//! - Fresh values (younger than `ttl`) are returned without a request.
//! - Concurrent callers for the same key wait on one in-flight fetch
//!   instead of each firing their own (per-key async lock).
//! - A failed fetch falls back to the last good value while it is younger
//!   than `stale_after`, so a network blip doesn't blank every key.
//! - After a failure, the key isn't retried for `retry_after`, so a dead
//!   network doesn't turn the 1s view-revert ticks into a request storm.

use std::collections::HashMap;
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

pub struct Cache<K, V, E> {
    policy: CachePolicy,
    slots: Mutex<HashMap<K, SharedSlot<V, E>>>,
}

impl<K: Eq + Hash + Clone, V, E: Clone> Cache<K, V, E> {
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

    pub async fn get<F, Fut>(&self, key: &K, fetch: F) -> Result<Arc<V>, E>
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
                .map(|(_, v)| v.clone())
        };

        if let Some((at, v)) = &slot.value
            && now.duration_since(*at) < p.ttl
        {
            return Ok(v.clone());
        }
        if let Some((at, e)) = &slot.last_error
            && now.duration_since(*at) < p.retry_after
        {
            return usable(&slot).ok_or_else(|| e.clone());
        }

        match fetch().await {
            Ok(v) => {
                let v = Arc::new(v);
                slot.value = Some((Instant::now(), v.clone()));
                slot.last_error = None;
                Ok(v)
            }
            Err(e) => {
                slot.last_error = Some((Instant::now(), e.clone()));
                usable(&slot).ok_or(e)
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
            .map(|v| *v)
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
        assert_eq!((*a, *b), (1, 2));
    }
}

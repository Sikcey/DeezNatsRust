//! Short-lived, bounded cache for unauthenticated public catalog responses.
use serde_json::Value;
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

pub(super) struct Cache {
    state: Mutex<State>,
    budget: usize,
}
#[derive(Default)]
struct State {
    entries: VecDeque<Entry>,
    bytes: usize,
}
struct Entry {
    key: String,
    expires: Instant,
    weight: usize,
    value: Arc<Value>,
}
impl Cache {
    pub(super) fn new(budget: usize) -> Self {
        Self {
            state: Mutex::new(State::default()),
            budget,
        }
    }
    pub(super) fn load(
        &self,
        key: &str,
        ttl: Duration,
        fetch: impl FnOnce() -> Result<Vec<u8>, String>,
        decode: impl FnOnce(&[u8]) -> Result<Value, String>,
    ) -> Result<Arc<Value>, String> {
        self.load_inner(key, ttl, fetch, decode, false)
    }
    pub(super) fn refresh(
        &self,
        key: &str,
        ttl: Duration,
        fetch: impl FnOnce() -> Result<Vec<u8>, String>,
        decode: impl FnOnce(&[u8]) -> Result<Value, String>,
    ) -> Result<Arc<Value>, String> {
        self.load_inner(key, ttl, fetch, decode, true)
    }
    fn load_inner(
        &self,
        key: &str,
        ttl: Duration,
        fetch: impl FnOnce() -> Result<Vec<u8>, String>,
        decode: impl FnOnce(&[u8]) -> Result<Value, String>,
        refresh: bool,
    ) -> Result<Arc<Value>, String> {
        let now = Instant::now();
        if !refresh {
            let mut state = self.state.lock().unwrap();
            state.entries.retain(|entry| entry.expires > now);
            state.bytes = state.entries.iter().map(|entry| entry.weight).sum();
            if let Some(index) = state.entries.iter().position(|entry| entry.key == key) {
                let entry = state.entries.remove(index).unwrap();
                let value = entry.value.clone();
                state.entries.push_back(entry);
                return Ok(value);
            }
        }
        // Neither a slow download nor JSON parsing holds the shared cache lock.
        let bytes = fetch()?;
        let value = Arc::new(decode(&bytes)?);
        // JSON's in-memory representation can be substantially larger than its
        // wire form. Charge conservatively and don't retain oversized pages.
        let weight = bytes.len().saturating_mul(8).saturating_add(1024);
        if weight <= self.budget {
            let mut state = self.state.lock().unwrap();
            if let Some(index) = state.entries.iter().position(|entry| entry.key == key) {
                state.bytes -= state.entries.remove(index).unwrap().weight;
            }
            while !state.entries.is_empty()
                && (state.bytes + weight > self.budget || state.entries.len() >= 32)
            {
                state.bytes -= state.entries.pop_front().unwrap().weight;
            }
            state.bytes += weight;
            state.entries.push_back(Entry {
                key: key.into(),
                expires: Instant::now() + ttl,
                weight,
                value: value.clone(),
            });
        }
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn decode(bytes: &[u8]) -> Result<Value, String> {
        serde_json::from_slice(bytes).map_err(|e| e.to_string())
    }
    #[test]
    fn revisits_skip_download_and_parsing_but_expired_entries_refresh() {
        let cache = Cache::new(4096);
        let first = cache
            .load(
                "a",
                Duration::from_secs(30),
                || Ok(b"{\"items\":[1]}".to_vec()),
                decode,
            )
            .unwrap();
        let hit = cache
            .load(
                "a",
                Duration::from_secs(30),
                || panic!("repeated download"),
                |_| panic!("repeated parse"),
            )
            .unwrap();
        assert!(Arc::ptr_eq(&first, &hit));
        cache.state.lock().unwrap().entries[0].expires = Instant::now();
        let fresh = cache
            .load(
                "a",
                Duration::from_secs(30),
                || Ok(b"{\"items\":[2]}".to_vec()),
                decode,
            )
            .unwrap();
        assert_eq!(fresh["items"][0], 2);
    }
    #[test]
    fn background_revalidation_fetches_updates_without_waiting_for_ttl() {
        let cache = Cache::new(4096);
        let ttl = Duration::from_secs(120);
        cache
            .load("page", ttl, || Ok(b"{\"version\":1}".to_vec()), decode)
            .unwrap();
        assert!(cache
            .refresh("page", ttl, || Err("network".into()), decode)
            .is_err());
        assert_eq!(
            cache
                .load("page", ttl, || panic!("lost valid cached data"), decode)
                .unwrap()["version"],
            1
        );
        assert_eq!(
            cache
                .refresh("page", ttl, || Ok(b"{\"version\":2}".to_vec()), decode)
                .unwrap()["version"],
            2
        );
        assert_eq!(
            cache
                .load("page", ttl, || panic!("refresh not saved"), decode)
                .unwrap()["version"],
            2
        );
    }
    #[test]
    fn failed_responses_are_not_cached_and_the_budget_is_enforced() {
        let cache = Cache::new(2100);
        assert!(cache
            .load(
                "a",
                Duration::from_secs(30),
                || Err("network".into()),
                decode
            )
            .is_err());
        assert!(cache
            .load(
                "a",
                Duration::from_secs(30),
                || Ok(b"invalid".to_vec()),
                decode
            )
            .is_err());
        for key in ["a", "b", "c"] {
            cache
                .load(key, Duration::from_secs(30), || Ok(b"{}".to_vec()), decode)
                .unwrap();
        }
        let state = cache.state.lock().unwrap();
        assert!(state.bytes <= 2100);
        assert_eq!(state.entries.len(), 2);
        assert_eq!(state.entries.front().unwrap().key, "b");
    }
}

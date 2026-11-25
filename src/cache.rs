use std::collections::HashMap;
use std::time::{Duration, Instant};

use tokio::sync::RwLock;

use crate::models::SearchResponse;

struct CacheEntry {
    response: SearchResponse,
    expires_at: Instant,
    last_access: Instant,
}

/// Bounded in-memory TTL cache for search responses.
pub struct SearchCache {
    ttl: Duration,
    max_entries: usize,
    entries: RwLock<HashMap<String, CacheEntry>>,
}

impl SearchCache {
    pub fn new(ttl_secs: u64, max_entries: usize) -> Self {
        Self {
            ttl: Duration::from_secs(ttl_secs),
            max_entries: max_entries.max(1),
            entries: RwLock::new(HashMap::new()),
        }
    }

    pub async fn get(&self, key: &str) -> Option<SearchResponse> {
        if self.ttl.is_zero() {
            return None;
        }
        let now = Instant::now();
        {
            let guard = self.entries.read().await;
            let entry = guard.get(key)?;
            if entry.expires_at <= now {
                drop(guard);
                self.entries.write().await.remove(key);
                return None;
            }
        }

        let mut guard = self.entries.write().await;
        let entry = guard.get_mut(key)?;
        if entry.expires_at <= now {
            guard.remove(key);
            return None;
        }
        entry.last_access = now;
        let mut resp = entry.response.clone();
        resp.cached = true;
        Some(resp)
    }

    pub async fn insert(&self, key: String, response: SearchResponse) {
        if self.ttl.is_zero() {
            return;
        }
        let now = Instant::now();
        let mut guard = self.entries.write().await;
        guard.retain(|_, v| v.expires_at > now);

        if guard.len() >= self.max_entries && !guard.contains_key(&key) {
            if let Some(oldest_key) = guard
                .iter()
                .min_by_key(|(_, v)| v.last_access)
                .map(|(k, _)| k.clone())
            {
                guard.remove(&oldest_key);
            }
        }

        guard.insert(
            key,
            CacheEntry {
                response,
                expires_at: now + self.ttl,
                last_access: now,
            },
        );
    }

    pub async fn len(&self) -> usize {
        self.entries.read().await.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Category;

    fn dummy_response(query: &str) -> SearchResponse {
        SearchResponse {
            query: query.to_string(),
            effective_query: query.to_string(),
            category: Category::General,
            page: 1,
            took_ms: 12,
            cached: false,
            total_results: 0,
            infobox: None,
            results: Vec::new(),
            engines: Vec::new(),
        }
    }

    #[tokio::test]
    async fn cache_hit_eviction_and_expiry() {
        let cache = SearchCache::new(60, 2);
        cache.insert("k1".into(), dummy_response("one")).await;
        cache.insert("k2".into(), dummy_response("two")).await;

        let hit = cache.get("k1").await.expect("k1 should hit");
        assert!(hit.cached);
        assert_eq!(hit.query, "one");

        // Inserting k3 evicts k2 (least recently accessed).
        cache.insert("k3".into(), dummy_response("three")).await;
        assert_eq!(cache.len().await, 2);
        assert!(cache.get("k2").await.is_none());
        assert!(cache.get("k1").await.is_some());
        assert!(cache.get("k3").await.is_some());

        let zero_ttl = SearchCache::new(0, 10);
        zero_ttl.insert("k".into(), dummy_response("z")).await;
        assert!(zero_ttl.get("k").await.is_none());
    }
}

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use compass_cypher::{CompiledQuery, PlanCacheKey};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PlanCacheConfig {
    pub max_entries: usize,
    pub max_bytes: usize,
}

impl Default for PlanCacheConfig {
    fn default() -> Self {
        Self {
            max_entries: 1_024,
            max_bytes: 64 * 1024 * 1024,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CacheStats {
    pub entries: usize,
    pub bytes: usize,
    pub hits: u64,
    pub misses: u64,
    pub evictions: u64,
}

#[derive(Debug)]
struct CacheEntry {
    query: Arc<CompiledQuery>,
    stamp: u64,
    bytes: usize,
}

#[derive(Debug, Default)]
struct CacheState {
    entries: HashMap<PlanCacheKey, CacheEntry>,
    bytes: usize,
}

#[derive(Debug)]
pub struct PlanCache {
    config: PlanCacheConfig,
    state: Mutex<CacheState>,
    clock: AtomicU64,
    hits: AtomicU64,
    misses: AtomicU64,
    evictions: AtomicU64,
}

impl PlanCache {
    #[must_use]
    pub fn new(config: PlanCacheConfig) -> Self {
        Self {
            config,
            state: Mutex::new(CacheState::default()),
            clock: AtomicU64::new(1),
            hits: AtomicU64::new(0),
            misses: AtomicU64::new(0),
            evictions: AtomicU64::new(0),
        }
    }

    #[must_use]
    pub fn get(&self, key: &PlanCacheKey) -> Option<Arc<CompiledQuery>> {
        let stamp = self.clock.fetch_add(1, Ordering::Relaxed);
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(entry) = state.entries.get_mut(key) {
            entry.stamp = stamp;
            self.hits.fetch_add(1, Ordering::Relaxed);
            Some(Arc::clone(&entry.query))
        } else {
            self.misses.fetch_add(1, Ordering::Relaxed);
            None
        }
    }

    pub fn insert(&self, key: PlanCacheKey, query: Arc<CompiledQuery>) {
        let bytes = estimated_query_bytes(&query);
        if self.config.max_entries == 0 || bytes > self.config.max_bytes {
            return;
        }
        let stamp = self.clock.fetch_add(1, Ordering::Relaxed);
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.entries.contains_key(&key) {
            return;
        }
        while state.entries.len() >= self.config.max_entries
            || state.bytes.saturating_add(bytes) > self.config.max_bytes
        {
            let Some(oldest) = state
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.stamp)
                .map(|(key, _)| *key)
            else {
                break;
            };
            if let Some(removed) = state.entries.remove(&oldest) {
                state.bytes = state.bytes.saturating_sub(removed.bytes);
                self.evictions.fetch_add(1, Ordering::Relaxed);
            }
        }
        state.bytes = state.bytes.saturating_add(bytes);
        state.entries.insert(
            key,
            CacheEntry {
                query,
                stamp,
                bytes,
            },
        );
    }

    #[must_use]
    pub fn stats(&self) -> CacheStats {
        let state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        CacheStats {
            entries: state.entries.len(),
            bytes: state.bytes,
            hits: self.hits.load(Ordering::Relaxed),
            misses: self.misses.load(Ordering::Relaxed),
            evictions: self.evictions.load(Ordering::Relaxed),
        }
    }
}

impl Default for PlanCache {
    fn default() -> Self {
        Self::new(PlanCacheConfig::default())
    }
}

fn estimated_query_bytes(query: &CompiledQuery) -> usize {
    std::mem::size_of::<CompiledQuery>()
        .saturating_add(query.plan.operators.len().saturating_mul(128))
        .saturating_add(query.columns.len().saturating_mul(64))
}

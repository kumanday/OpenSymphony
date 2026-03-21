//! Ordered event cache with event-id deduplication.

use std::collections::HashSet;

use crate::wire::RuntimeEventEnvelope;

/// In-memory event cache ordered by server timestamp and deduplicated by event ID.
#[derive(Clone, Debug, Default)]
pub struct EventCache {
    events: Vec<RuntimeEventEnvelope>,
    event_ids: HashSet<String>,
}

impl EventCache {
    /// Inserts an event when it has not already been seen.
    pub fn insert(&mut self, event: RuntimeEventEnvelope) -> bool {
        if !self.event_ids.insert(event.id.clone()) {
            return false;
        }
        let index = self
            .events
            .partition_point(|existing| existing.timestamp <= event.timestamp);
        self.events.insert(index, event);
        true
    }

    /// Merges a collection of events, returning the number of newly added items.
    pub fn extend<I>(&mut self, events: I) -> usize
    where
        I: IntoIterator<Item = RuntimeEventEnvelope>,
    {
        events
            .into_iter()
            .filter(|event| self.insert(event.clone()))
            .count()
    }

    /// Returns the ordered events currently retained by the cache.
    #[must_use]
    pub fn events(&self) -> &[RuntimeEventEnvelope] {
        &self.events
    }

    /// Returns the number of cached events.
    #[must_use]
    pub fn len(&self) -> usize {
        self.events.len()
    }

    /// Returns whether the cache is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::{RuntimeEventEnvelope, RuntimeEventPayload};

    fn envelope(id: &str, timestamp: &str) -> RuntimeEventEnvelope {
        RuntimeEventEnvelope {
            id: id.to_string(),
            timestamp: timestamp.to_string(),
            source: "environment".to_string(),
            kind: "UnknownEvent".to_string(),
            payload: RuntimeEventPayload::Unknown,
            raw_json: serde_json::json!({
                "id": id,
                "timestamp": timestamp,
                "source": "environment",
                "kind": "UnknownEvent"
            }),
        }
    }

    #[test]
    fn cache_orders_by_timestamp_and_deduplicates() {
        let mut cache = EventCache::default();
        assert!(cache.insert(envelope("b", "2026-03-21T15:00:02")));
        assert!(cache.insert(envelope("a", "2026-03-21T15:00:01")));
        assert!(!cache.insert(envelope("a", "2026-03-21T15:00:03")));
        assert!(cache.insert(envelope("c", "2026-03-21T15:00:03")));

        let ids: Vec<_> = cache
            .events()
            .iter()
            .map(|event| event.id.as_str())
            .collect();
        assert_eq!(ids, vec!["a", "b", "c"]);
    }
}

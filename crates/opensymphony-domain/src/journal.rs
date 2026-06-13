use std::{collections::VecDeque, fmt, sync::Arc};

use tokio::sync::broadcast;

use std::future::Future;

use crate::opensymphony_gateway_schema::{
    cursor::StreamCursor,
    event_journal::{
        EventActor, EventId, EventKind, EventPage, EventRecord, JournalError, JournalHealth,
        JournalHealthStatus, StreamConnectionState, StreamError,
    },
};

/// Trait for event journal storage backends.
pub trait EventJournalBackend: Send + Sync + 'static {
    /// Append an event to the journal. Returns the event with its assigned sequence.
    fn append(
        &self,
        event: EventRecord,
    ) -> impl Future<Output = Result<EventRecord, JournalError>> + Send;

    /// Query events after the given cursor.
    fn query_after(
        &self,
        cursor: &StreamCursor,
        limit: usize,
    ) -> impl Future<Output = Result<EventPage, JournalError>> + Send;

    /// Get the latest cursor position.
    fn latest_cursor(&self) -> impl Future<Output = StreamCursor> + Send;

    /// Get the oldest available sequence number (for cursor staleness checks).
    fn oldest_sequence(&self) -> impl Future<Output = Option<u64>> + Send;

    /// Get health information about the journal.
    fn health(&self) -> impl Future<Output = JournalHealth> + Send;
}

/// Bounded in-memory event journal with backpressure support.
#[derive(Debug, Clone)]
pub struct InMemoryEventJournal {
    inner: Arc<tokio::sync::RwLock<JournalState>>,
    /// Broadcast channel for live subscribers.
    subscribers: broadcast::Sender<Result<EventRecord, StreamError>>,
}

#[derive(Debug)]
struct JournalState {
    /// Bounded event buffer.
    events: VecDeque<EventRecord>,
    /// Maximum capacity before eviction.
    capacity: usize,
    /// Next sequence number to assign.
    next_sequence: u64,
}

impl InMemoryEventJournal {
    /// Create a new journal with the given capacity and subscriber channel size.
    pub fn new(capacity: usize, subscriber_capacity: usize) -> Self {
        let (subscribers, _) = broadcast::channel(subscriber_capacity);
        Self {
            inner: Arc::new(tokio::sync::RwLock::new(JournalState {
                events: VecDeque::with_capacity(capacity),
                capacity,
                next_sequence: 1,
            })),
            subscribers,
        }
    }

    /// Get the configured capacity.
    pub async fn capacity(&self) -> usize {
        self.inner.read().await.capacity
    }

    /// Append an event. The journal always assigns the sequence number itself to
    /// guarantee monotonic ordering regardless of caller-provided values.
    /// Returns `JournalError::Backpressure` if the journal is at capacity and eviction
    /// cannot make room (e.g., capacity is 0).
    pub async fn append(&self, mut event: EventRecord) -> Result<EventRecord, JournalError> {
        let mut state = self.inner.write().await;

        // Always assign sequence to guarantee monotonic ordering.
        event.sequence = state.next_sequence;

        // Check capacity and evict if needed.
        if state.events.len() >= state.capacity {
            // Try to evict the oldest events to make room.
            let to_evict = state.events.len().saturating_sub(state.capacity / 2);
            for _ in 0..to_evict {
                state.events.pop_front();
            }

            // If still at capacity after eviction, return backpressure error.
            if state.events.len() >= state.capacity {
                return Err(JournalError::Backpressure {
                    capacity: state.capacity,
                });
            }
        }

        state.next_sequence = state.next_sequence.saturating_add(1);
        state.events.push_back(event.clone());

        // Broadcast to subscribers (ignore errors from lagged/dropped receivers).
        let _ = self.subscribers.send(Ok(event.clone()));
        Ok(event)
    }

    /// Query events after the given cursor sequence within the partition.
    pub async fn query_after(
        &self,
        cursor: &StreamCursor,
        limit: usize,
    ) -> Result<EventPage, JournalError> {
        let state = self.inner.read().await;

        // Validate cursor against oldest available sequence.
        // Cursor sequence 0 is always valid (means "start from beginning").
        if cursor.sequence > 0
            && let Some(oldest) = state.events.front().map(|e| e.sequence)
            && cursor.sequence < oldest
        {
            return Err(JournalError::InvalidCursor {
                reason: format!(
                    "Cursor sequence {} is older than oldest available sequence {}",
                    cursor.sequence, oldest
                ),
            });
        }

        let mut iter = state.events.iter().filter(|e| {
            e.sequence > cursor.sequence && e.kind.default_partition() == cursor.partition
        });
        let events: Vec<EventRecord> = iter.by_ref().take(limit).cloned().collect();
        let has_more = iter.next().is_some();

        let next_cursor = events.last().map(|e| e.next_cursor(&cursor.partition));

        use crate::opensymphony_gateway_schema::version::SchemaVersion;

        Ok(EventPage {
            schema_version: SchemaVersion::v1(),
            events,
            next_cursor,
            has_more,
        })
    }

    /// Get the latest cursor for the given partition.
    pub async fn latest_cursor_for_partition(&self, partition: &str) -> StreamCursor {
        let state = self.inner.read().await;
        let newest = state
            .events
            .iter()
            .rfind(|e| e.kind.default_partition() == partition)
            .map(|e| e.sequence)
            .unwrap_or(0);
        StreamCursor::new(newest, partition)
    }

    /// Snapshot the most recent `limit` events in insertion order.
    ///
    /// Used by tests that need to assert mutations are correlated with
    /// audit events. Calling with `limit = 0` returns an empty vector.
    pub async fn recent_events(&self, limit: usize) -> Vec<EventRecord> {
        let state = self.inner.read().await;
        if limit == 0 || state.events.is_empty() {
            return Vec::new();
        }
        let start = state.events.len().saturating_sub(limit);
        state.events.iter().skip(start).cloned().collect()
    }

    /// Subscribe to live events. Returns a receiver for the broadcast channel.
    pub fn subscribe(&self) -> broadcast::Receiver<Result<EventRecord, StreamError>> {
        self.subscribers.subscribe()
    }

    /// Get health information.
    pub async fn health(&self) -> JournalHealth {
        let state = self.inner.read().await;
        let used = state.events.len();
        let status = if used == 0 {
            JournalHealthStatus::Healthy
        } else if used >= state.capacity {
            JournalHealthStatus::AtCapacity
        } else if used >= state.capacity * 4 / 5 {
            JournalHealthStatus::NearCapacity
        } else {
            JournalHealthStatus::Healthy
        };

        JournalHealth {
            status,
            capacity: state.capacity,
            used,
            oldest_sequence: state.events.front().map(|e| e.sequence),
            newest_sequence: state.events.back().map(|e| e.sequence),
        }
    }

    /// Helper to create an orchestrator event.
    pub fn orchestrator_event(
        kind: EventKind,
        summary: impl Into<String>,
        payload: Option<serde_json::Value>,
    ) -> EventRecord {
        EventRecord::builder()
            .actor(EventActor::system("orchestrator"))
            .kind(kind)
            .summary(summary)
            .payload_or_none(payload)
            .build()
    }

    /// Helper to create a gateway action event.
    pub fn gateway_action_event(
        kind: EventKind,
        correlation_id: Option<EventId>,
        summary: impl Into<String>,
        payload: Option<serde_json::Value>,
    ) -> EventRecord {
        EventRecord::builder()
            .actor(EventActor::system("gateway"))
            .correlation_id_opt(correlation_id)
            .kind(kind)
            .summary(summary)
            .payload_or_none(payload)
            .build()
    }

    /// Helper to create a normalized harness event.
    pub fn harness_event(
        harness_id: impl Into<String>,
        kind: EventKind,
        summary: impl Into<String>,
        payload: Option<serde_json::Value>,
    ) -> EventRecord {
        EventRecord::builder()
            .actor(EventActor::harness(harness_id))
            .kind(kind)
            .summary(summary)
            .payload_or_none(payload)
            .build()
    }
}

impl EventJournalBackend for InMemoryEventJournal {
    async fn append(&self, event: EventRecord) -> Result<EventRecord, JournalError> {
        InMemoryEventJournal::append(self, event).await
    }

    async fn query_after(
        &self,
        cursor: &StreamCursor,
        limit: usize,
    ) -> Result<EventPage, JournalError> {
        InMemoryEventJournal::query_after(self, cursor, limit).await
    }

    async fn latest_cursor(&self) -> StreamCursor {
        self.latest_cursor_for_partition("events").await
    }

    async fn oldest_sequence(&self) -> Option<u64> {
        self.inner.read().await.events.front().map(|e| e.sequence)
    }

    async fn health(&self) -> JournalHealth {
        InMemoryEventJournal::health(self).await
    }
}

/// Stream broker that manages connections to the event journal.
#[derive(Debug, Clone)]
pub struct StreamBroker {
    journal: InMemoryEventJournal,
    /// Tracks active connection states.
    connections: Arc<tokio::sync::Mutex<BTreeMap<Arc<str>, StreamConnectionState>>>,
}

use std::collections::BTreeMap;

impl StreamBroker {
    pub fn new(journal: InMemoryEventJournal) -> Self {
        Self {
            journal,
            connections: Arc::default(),
        }
    }

    /// Create a new event stream starting from the given cursor.
    pub fn create_stream(&self, cursor: &StreamCursor) -> Result<EventStream, StreamError> {
        let receiver = self.journal.subscribe();
        Ok(EventStream::new(
            receiver,
            cursor.sequence,
            cursor.partition.clone(),
        ))
    }

    /// Register a connection with the given ID.
    pub async fn register_connection(&self, connection_id: Arc<str>) {
        let mut connections = self.connections.lock().await;
        connections.insert(
            connection_id,
            StreamConnectionState {
                connected: true,
                backpressure_active: false,
                last_sequence: None,
                error: None,
            },
        );
    }

    /// Unregister a connection.
    pub async fn unregister_connection(&self, connection_id: &str) {
        let mut connections = self.connections.lock().await;
        connections.remove(&(Arc::from(connection_id) as Arc<str>));
    }

    /// Get the connection state for a specific connection.
    pub async fn connection_state(&self, connection_id: &str) -> Option<StreamConnectionState> {
        let connections = self.connections.lock().await;
        connections.get(connection_id).cloned()
    }

    /// Get a summary of all connection states.
    pub async fn connection_summary(&self) -> Vec<StreamConnectionState> {
        let connections = self.connections.lock().await;
        connections.values().cloned().collect()
    }

    /// Get the total number of active connections.
    pub async fn active_connection_count(&self) -> usize {
        let connections = self.connections.lock().await;
        connections.values().filter(|s| s.connected).count()
    }

    /// Get the underlying journal for direct access.
    pub fn journal(&self) -> InMemoryEventJournal {
        self.journal.clone()
    }
}

/// A stream of events from the journal broadcast channel.
pub struct EventStream {
    inner: broadcast::Receiver<Result<EventRecord, StreamError>>,
    last_sequence: u64,
    partition: String,
}

impl EventStream {
    pub fn new(
        receiver: broadcast::Receiver<Result<EventRecord, StreamError>>,
        last_sequence: u64,
        partition: String,
    ) -> Self {
        Self {
            inner: receiver,
            last_sequence,
            partition,
        }
    }

    pub fn last_sequence(&self) -> u64 {
        self.last_sequence
    }

    pub fn set_last_sequence(&mut self, seq: u64) {
        self.last_sequence = seq.max(self.last_sequence);
    }

    pub fn partition(&self) -> &str {
        &self.partition
    }

    pub async fn recv(&mut self) -> Option<Result<EventRecord, StreamError>> {
        loop {
            match self.inner.recv().await {
                Ok(Ok(event)) => {
                    // Skip events we already delivered (e.g., backlog overlap after
                    // set_last_sequence() advances past the initial query results).
                    if event.sequence <= self.last_sequence {
                        continue;
                    }
                    // Only deliver events for this partition.
                    if event.kind.default_partition() == self.partition {
                        self.last_sequence = event.sequence;
                        return Some(Ok(event));
                    }
                    // Non-matching partition: skip without advancing last_sequence
                    // so we never accidentally drop a target-partition event.
                }
                Ok(Err(err)) => return Some(Err(err)),
                Err(broadcast::error::RecvError::Lagged(skipped)) => {
                    // We missed messages in the broadcast buffer. Report backpressure
                    // so the client can decide whether to reconnect from a cursor.
                    tracing::warn!(
                        skipped = skipped,
                        partition = %self.partition,
                        "Broadcast receiver lagged; some events may have been skipped"
                    );
                    return Some(Err(StreamError::backpressure()));
                }
                Err(broadcast::error::RecvError::Closed) => return None,
            }
        }
    }
}

impl fmt::Debug for EventStream {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EventStream")
            .field("last_sequence", &self.last_sequence)
            .field("partition", &self.partition)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::opensymphony_gateway_schema::{
        cursor::StreamCursor, envelope::EntityRef, event_journal::EventKind,
    };

    fn test_journal() -> InMemoryEventJournal {
        InMemoryEventJournal::new(100, 64)
    }

    fn sample_event(sequence: u64, kind: EventKind) -> EventRecord {
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let unique = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        EventRecord::builder()
            .event_id(format!("evt_{}_{}", sequence, unique))
            .sequence(sequence)
            .actor(EventActor::system("test"))
            .kind(kind)
            .summary(format!("Event {sequence}"))
            .build()
    }

    #[tokio::test]
    async fn append_assigns_monotonic_sequence() {
        let journal = test_journal();

        let e1 = journal
            .append(sample_event(0, EventKind::RunStarted))
            .await
            .expect("append");
        let e2 = journal
            .append(sample_event(0, EventKind::RunCompleted))
            .await
            .expect("append");

        assert_eq!(e1.sequence, 1);
        assert_eq!(e2.sequence, 2);
    }

    #[tokio::test]
    async fn query_after_returns_empty_when_no_events() {
        let journal = test_journal();
        let cursor = StreamCursor::new(0, "events");
        let page = journal.query_after(&cursor, 10).await.expect("query");
        assert!(page.events.is_empty());
        assert!(!page.has_more);
    }

    #[tokio::test]
    async fn cursor_replay_returns_events_in_sequence() {
        let journal = test_journal();

        for _ in 0..5 {
            let event = sample_event(0, EventKind::RunStarted);
            journal.append(event).await.expect("append");
        }

        // Resume from sequence 2.
        let cursor = StreamCursor::new(2, "events");
        let page = journal.query_after(&cursor, 10).await.expect("query");

        assert_eq!(page.events.len(), 3);
        assert_eq!(page.events[0].sequence, 3);
        assert_eq!(page.events[1].sequence, 4);
        assert_eq!(page.events[2].sequence, 5);
    }

    #[tokio::test]
    async fn bounded_queue_backpressure() {
        let journal = InMemoryEventJournal::new(4, 64);

        // Fill the journal.
        for _ in 0..4 {
            let event = sample_event(0, EventKind::RunStarted);
            journal.append(event).await.expect("append");
        }

        // Next append should trigger eviction and succeed.
        let event = sample_event(0, EventKind::RunCompleted);
        let result = journal.append(event).await;
        assert!(result.is_ok(), "append should succeed after eviction");

        // After eviction and append, oldest sequence should be higher.
        let oldest = journal.oldest_sequence().await;
        assert!(
            oldest.map(|o| o >= 3).unwrap_or(false),
            "old events should be evicted"
        );
    }

    #[tokio::test]
    async fn health_report_shows_capacity() {
        let journal = test_journal();

        let health = journal.health().await;
        assert_eq!(health.capacity, 100);
        assert_eq!(health.used, 0);
        assert_eq!(health.status, JournalHealthStatus::Healthy);

        // Add an event.
        let event = sample_event(0, EventKind::RunStarted);
        journal.append(event).await.expect("append");

        let health = journal.health().await;
        assert_eq!(health.used, 1);
        assert_eq!(health.oldest_sequence, Some(1));
        assert_eq!(health.newest_sequence, Some(1));
    }

    #[tokio::test]
    async fn cursor_pagination_returns_next_cursor_when_has_more() {
        let journal = test_journal();

        for _ in 0..5 {
            let event = sample_event(0, EventKind::RunStarted);
            journal.append(event).await.expect("append");
        }

        // Query with limit of 2.
        let cursor = StreamCursor::new(0, "events");
        let page = journal.query_after(&cursor, 2).await.expect("query");

        assert_eq!(page.events.len(), 2);
        assert!(page.has_more);
        assert!(page.next_cursor.is_some());

        // Use next cursor to get the next page.
        let next = page.next_cursor.expect("next cursor should exist");
        let page2 = journal.query_after(&next, 2).await.expect("query");

        assert_eq!(page2.events.len(), 2);
        assert!(page2.has_more);

        // Get the last page.
        let next2 = page2.next_cursor.expect("next cursor should exist");
        let page3 = journal.query_after(&next2, 2).await.expect("query");

        assert_eq!(page3.events.len(), 1);
        assert!(!page3.has_more);
    }

    #[tokio::test]
    async fn partition_filtering_separates_terminal_log_events() {
        let journal = test_journal();

        // Add control events.
        let control = sample_event(0, EventKind::RunStarted);
        journal.append(control).await.expect("append");

        // Add terminal frame events (high volume).
        let terminal = EventRecord::builder()
            .event_id("evt_term_1")
            .sequence(0)
            .actor(EventActor::system("test"))
            .kind(EventKind::TerminalFrame {
                frame_id: "frame_1".into(),
            })
            .summary("Terminal frame")
            .build();
        journal.append(terminal).await.expect("append");

        // Query control events only.
        let control_cursor = StreamCursor::new(0, "events");
        let control_page = journal
            .query_after(&control_cursor, 10)
            .await
            .expect("query");

        assert_eq!(control_page.events.len(), 1);
        assert_eq!(control_page.events[0].kind, EventKind::RunStarted);

        // Query terminal events only.
        let terminal_cursor = StreamCursor::new(0, "terminal_log");
        let terminal_page = journal
            .query_after(&terminal_cursor, 10)
            .await
            .expect("query");

        assert_eq!(terminal_page.events.len(), 1);
        assert_eq!(terminal_page.events[0].event_id, "evt_term_1");
    }

    #[tokio::test]
    async fn reconnect_delivers_only_new_events() {
        let journal = test_journal();

        // Add initial events.
        for _ in 0..3 {
            let event = sample_event(0, EventKind::RunStarted);
            journal.append(event).await.expect("append");
        }

        // Client reads up to sequence 3.
        let cursor = StreamCursor::new(3, "events");

        // Add more events after client disconnected.
        for _ in 0..2 {
            let event = sample_event(0, EventKind::RunCompleted);
            journal.append(event).await.expect("append");
        }

        // Client reconnects with cursor.
        let page = journal.query_after(&cursor, 10).await.expect("query");
        assert_eq!(page.events.len(), 2);
        assert_eq!(page.events[0].sequence, 4);
        assert_eq!(page.events[1].sequence, 5);
    }

    #[tokio::test]
    async fn duplicate_events_have_same_id_different_sequence() {
        let journal = test_journal();

        let event = sample_event(0, EventKind::RunStarted);
        let stored = journal.append(event.clone()).await.expect("append");
        let duplicate = journal.append(event.clone()).await.expect("append");

        // Same event_id means duplicate.
        assert!(stored.is_duplicate_of(&duplicate));
        // But they have different sequences.
        assert_ne!(stored.sequence, duplicate.sequence);
    }

    #[tokio::test]
    async fn raw_payload_ref_is_retained() {
        let journal = test_journal();

        let event = EventRecord::builder()
            .event_id("evt_raw")
            .sequence(0)
            .actor(EventActor::harness("test-harness"))
            .kind(EventKind::Unknown {
                raw_kind: "custom_event".into(),
            })
            .summary("Unknown harness event")
            .raw_payload_ref("raw_ref_123")
            .build();

        let stored = journal.append(event).await.expect("append");
        assert!(stored.has_raw_payload());
        assert_eq!(stored.raw_payload_ref, Some("raw_ref_123".into()));

        // Verify it can be queried back.
        let cursor = StreamCursor::new(0, "events");
        let page = journal.query_after(&cursor, 10).await.expect("query");
        assert_eq!(page.events.len(), 1);
        assert!(page.events[0].has_raw_payload());
    }

    #[tokio::test]
    async fn stream_broker_creates_stream_from_cursor() {
        let journal = test_journal();
        let broker = StreamBroker::new(journal.clone());

        let cursor = StreamCursor::new(0, "events");
        let stream = broker.create_stream(&cursor).expect("create stream");

        assert_eq!(stream.last_sequence(), 0);
        assert_eq!(stream.partition(), "events");
    }

    #[tokio::test]
    async fn entity_refs_are_preserved_through_journal() {
        let journal = test_journal();

        let event = EventRecord::builder()
            .event_id("evt_entity")
            .sequence(0)
            .actor(EventActor::agent("agent-1"))
            .kind(EventKind::RunStarted)
            .summary("Run with entity refs")
            .entity_ref(EntityRef::run("run_abc"))
            .entity_ref(EntityRef::issue("issue_123", Some("COE-393".into())))
            .build();

        let stored = journal.append(event).await.expect("append");
        assert_eq!(stored.entity_refs.len(), 2);

        // Query and verify.
        let cursor = StreamCursor::new(0, "events");
        let page = journal.query_after(&cursor, 10).await.expect("query");
        assert_eq!(page.events[0].entity_refs.len(), 2);
    }

    #[tokio::test]
    async fn cursor_zero_returns_all_events() {
        let journal = test_journal();

        // Add 3 events (sequences 1, 2, 3).
        for _ in 0..3 {
            let event = sample_event(0, EventKind::RunStarted);
            journal.append(event).await.expect("append");
        }

        // Query from sequence 0 should be fine (returns all).
        let cursor = StreamCursor::new(0, "events");
        let page = journal.query_after(&cursor, 10).await.expect("query");
        assert_eq!(page.events.len(), 3);
    }

    #[tokio::test]
    async fn stale_cursor_returns_invalid_cursor_error() {
        // Create a journal with small capacity to force eviction.
        let journal = InMemoryEventJournal::new(5, 256);

        // Fill to capacity and beyond so oldest sequence > 1.
        for _ in 0..10 {
            let event = sample_event(0, EventKind::RunStarted);
            let _ = journal.append(event).await;
        }

        // oldest_sequence should be > 1 after evictions.
        let oldest = journal.oldest_sequence().await.expect("should have events");
        assert!(
            oldest > 1,
            "oldest should be > 1 after evictions, got {}",
            oldest
        );

        // A cursor between 0 and oldest should fail (cursor > 0, cursor < oldest).
        let stale_seq = oldest - 1;
        let cursor = StreamCursor::new(stale_seq, "events");
        match journal.query_after(&cursor, 10).await {
            Err(JournalError::InvalidCursor { reason }) => {
                assert!(reason.contains("older than oldest"));
            }
            Ok(_) => panic!("expected InvalidCursor for stale cursor"),
            Err(e) => panic!("expected InvalidCursor, got {:?}", e),
        }
    }

    #[tokio::test]
    async fn live_broadcast_delivers_to_subscribers() {
        let journal = test_journal();

        // Subscribe first.
        let mut receiver = journal.subscribe();

        // Append an event.
        let event = sample_event(0, EventKind::RunStarted);
        journal.append(event).await.expect("append");

        // Receiver should get the event.
        let result = tokio::time::timeout(std::time::Duration::from_secs(1), receiver.recv())
            .await
            .expect("should receive in time")
            .expect("channel should not be closed");

        assert!(result.is_ok());
        assert_eq!(
            result.expect("event should be Ok").kind,
            EventKind::RunStarted
        );
    }

    #[tokio::test]
    async fn correlation_id_links_related_events() {
        let journal = test_journal();

        let correlation = "act_retry_001";

        let dispatched = EventRecord::builder()
            .event_id("evt_dispatch")
            .sequence(0)
            .actor(EventActor::system("gateway"))
            .correlation_id(correlation)
            .kind(EventKind::GatewayActionDispatched {
                action: "retry".into(),
            })
            .summary("Retry dispatched")
            .build();

        let completed = EventRecord::builder()
            .event_id("evt_completed")
            .sequence(0)
            .actor(EventActor::system("gateway"))
            .correlation_id(correlation)
            .kind(EventKind::GatewayActionCompleted {
                action: "retry".into(),
            })
            .summary("Retry completed")
            .build();

        let dispatched = journal.append(dispatched).await.expect("append");
        let completed = journal.append(completed).await.expect("append");

        assert_eq!(dispatched.correlation_id, completed.correlation_id);
        assert_eq!(dispatched.correlation_id, Some(correlation.to_string()));

        // Both events should be queryable.
        let cursor = StreamCursor::new(0, "events");
        let page = journal.query_after(&cursor, 10).await.expect("query");
        assert_eq!(page.events.len(), 2);
    }

    #[tokio::test]
    async fn stream_broker_connection_tracking() {
        let journal = test_journal();
        let broker = StreamBroker::new(journal.clone());

        let conn_id: Arc<str> = Arc::from("conn-1");
        broker.register_connection(conn_id.clone()).await;

        assert_eq!(broker.active_connection_count().await, 1);

        let state = broker.connection_state("conn-1").await;
        assert!(state.is_some());
        assert!(state.expect("connection should exist").connected);

        broker.unregister_connection("conn-1").await;
        assert_eq!(broker.active_connection_count().await, 0);
    }
}

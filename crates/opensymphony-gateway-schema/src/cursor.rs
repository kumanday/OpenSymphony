use serde::{Deserialize, Serialize};

/// Monotonic stream cursor for replay and resumable subscriptions.
///
/// Clients send the last `sequence` they received; the server returns
/// everything after that point. `partition` allows per-stream sharding
/// without global coordination.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StreamCursor {
    /// Monotonically increasing sequence number within a partition.
    pub sequence: u64,
    /// Logical stream partition identifier (e.g. "events", "terminal:{run_id}").
    pub partition: String,
    /// Optional wall-clock anchor for correlation across partitions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timestamp_anchor: Option<u64>,
}

impl StreamCursor {
    pub fn new(sequence: u64, partition: impl Into<String>) -> Self {
        Self {
            sequence,
            partition: partition.into(),
            timestamp_anchor: None,
        }
    }

    pub fn with_timestamp_anchor(mut self, anchor: u64) -> Self {
        self.timestamp_anchor = Some(anchor);
        self
    }
}

/// Pagination cursor for detail reads (runs, events, files, diffs).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PageCursor {
    /// Opaque page token; empty means first page.
    pub page_token: String,
    /// Desired page size.
    pub page_size: u32,
}

impl PageCursor {
    pub fn first(page_size: u32) -> Self {
        Self {
            page_token: String::new(),
            page_size,
        }
    }
}

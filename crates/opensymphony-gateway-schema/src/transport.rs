use serde::{Deserialize, Serialize};

/// Transport recommendation for local desktop and remote hosted modes.
///
/// This is documentation metadata, not a runtime protocol type.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TransportRecommendation {
    pub profile: TransportProfile,
    pub priority: u8,
    pub description: String,
    pub expected_latency_ms: u32,
    /// Expected throughput in kilobits per second.
    pub expected_throughput_kbps: u64,
    pub reconnect_support: bool,
    pub replay_support: bool,
    pub binary_frame_support: bool,
    pub auth_required: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransportProfile {
    InProcessChannel,
    NativeIpc,
    TauriChannel,
    LoopbackHttp,
    LoopbackWebSocket,
    Sse,
    WebSocket,
    JsonRpcOverWebSocket,
}

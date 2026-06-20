use crate::opensymphony_gateway_schema::capability::HarnessCapability;

/// Minimal Rust boundary shared by concrete harness adapters.
///
/// Runtime execution remains owned by the orchestrator and concrete adapter
/// modules. This trait gives the host and gateway a stable capability discovery
/// surface without leaking private OpenHands, Codex, or future in-process types
/// into client-facing DTOs.
pub trait HarnessAdapter: Send + Sync {
    fn harness_kind(&self) -> &'static str;
    fn capabilities(&self) -> HarnessCapability;
}

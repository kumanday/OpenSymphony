use serde::{Deserialize, Serialize};

use super::version::SchemaVersion;

/// Capability discovery response for `/api/v1/capabilities`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GatewayCapabilities {
    pub schema_version: SchemaVersion,
    pub gateway_version: String,
    pub supported_api_versions: Vec<String>,
    pub transports: Vec<TransportCapability>,
    pub features: Vec<FeatureCapability>,
    pub auth_modes: Vec<AuthMode>,
    pub max_event_page_size: u32,
    pub max_terminal_frame_batch: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransportCapability {
    pub transport: String,
    pub modes: Vec<String>,
    pub supported_encodings: Vec<String>,
    pub bidirectional: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeatureCapability {
    pub feature: String,
    pub available: bool,
    pub requires_auth: bool,
    pub requires_plan: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthMode {
    None,
    ApiKey,
    BearerToken,
    SubscriptionOAuth,
}

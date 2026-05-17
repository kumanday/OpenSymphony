use serde::{Deserialize, Serialize};

/// Gateway API schema version constant.
///
/// Bumped on every breaking change to any gateway DTO. Clients can negotiate
/// compatibility using the capability endpoint.
pub const GATEWAY_SCHEMA_VERSION: &str = "1.0.0";

/// Semantic version wrapper used in every versioned payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SchemaVersion {
    pub major: u16,
    pub minor: u16,
    pub patch: u16,
}

impl SchemaVersion {
    pub fn v1() -> Self {
        Self {
            major: 1,
            minor: 0,
            patch: 0,
        }
    }

    pub fn as_str(&self) -> String {
        format!("{}.{:1}.{:1}", self.major, self.minor, self.patch)
    }
}

impl Default for SchemaVersion {
    fn default() -> Self {
        Self::v1()
    }
}

impl std::fmt::Display for SchemaVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{:1}.{:1}", self.major, self.minor, self.patch)
    }
}

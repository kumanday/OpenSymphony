use opensymphony_domain::TrackerErrorCategory;
use reqwest::StatusCode;
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphqlError {
    pub message: String,
    pub code: Option<String>,
    pub user_presentable_message: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum LinearMcpError {
    #[error("invalid Linear MCP configuration: {0}")]
    InvalidConfiguration(String),
    #[error("Linear request failed: {0}")]
    Request(Box<reqwest::Error>),
    #[error("Linear API returned HTTP {status}: {body}")]
    HttpStatus { status: StatusCode, body: String },
    #[error("Linear GraphQL returned errors: {summary}")]
    Graphql {
        errors: Vec<GraphqlError>,
        summary: String,
    },
    #[error("Linear API returned an invalid response: {0}")]
    InvalidResponse(String),
    #[error("{0}")]
    NotFound(String),
}

impl LinearMcpError {
    pub fn from_graphql_errors(errors: Vec<GraphqlError>) -> Self {
        let summary = errors
            .iter()
            .map(|error| {
                error
                    .user_presentable_message
                    .clone()
                    .or_else(|| {
                        error
                            .code
                            .as_ref()
                            .map(|code| format!("{code}: {}", error.message))
                    })
                    .unwrap_or_else(|| error.message.clone())
            })
            .collect::<Vec<_>>()
            .join("; ");
        Self::Graphql { errors, summary }
    }

    pub fn category(&self) -> TrackerErrorCategory {
        match self {
            Self::InvalidConfiguration(_) | Self::InvalidResponse(_) => {
                TrackerErrorCategory::InvalidResponse
            }
            Self::Request(error) if error.is_timeout() => TrackerErrorCategory::Timeout,
            Self::Request(_) => TrackerErrorCategory::Transport,
            Self::HttpStatus { status, .. } => http_status_category(*status),
            Self::Graphql { errors, .. } => graphql_category(errors),
            Self::NotFound(_) => TrackerErrorCategory::NotFound,
        }
    }

    pub fn user_message(&self) -> String {
        match self {
            Self::Graphql { errors, summary } => errors
                .first()
                .and_then(|error| error.user_presentable_message.clone())
                .unwrap_or_else(|| summary.clone()),
            Self::NotFound(message)
            | Self::InvalidConfiguration(message)
            | Self::InvalidResponse(message) => message.clone(),
            Self::Request(error) => error.to_string(),
            Self::HttpStatus { body, .. } => body.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ToolFailure {
    pub code: String,
    pub message: String,
}

impl ToolFailure {
    pub fn invalid_input(message: impl Into<String>) -> Self {
        Self {
            code: "invalid_input".to_string(),
            message: message.into(),
        }
    }
}

impl From<LinearMcpError> for ToolFailure {
    fn from(error: LinearMcpError) -> Self {
        Self {
            code: category_code(error.category()).to_string(),
            message: error.user_message(),
        }
    }
}

fn category_code(category: TrackerErrorCategory) -> &'static str {
    match category {
        TrackerErrorCategory::Auth => "auth",
        TrackerErrorCategory::RateLimited => "rate_limited",
        TrackerErrorCategory::Transport => "transport",
        TrackerErrorCategory::Timeout => "timeout",
        TrackerErrorCategory::InvalidResponse => "invalid_response",
        TrackerErrorCategory::NotFound => "not_found",
        TrackerErrorCategory::InvalidStateTransition => "invalid_state_transition",
        TrackerErrorCategory::PermissionDenied => "permission_denied",
    }
}

fn http_status_category(status: StatusCode) -> TrackerErrorCategory {
    match status {
        StatusCode::UNAUTHORIZED => TrackerErrorCategory::Auth,
        StatusCode::FORBIDDEN => TrackerErrorCategory::PermissionDenied,
        StatusCode::NOT_FOUND => TrackerErrorCategory::NotFound,
        StatusCode::TOO_MANY_REQUESTS => TrackerErrorCategory::RateLimited,
        value if value.is_server_error() => TrackerErrorCategory::Transport,
        _ => TrackerErrorCategory::InvalidResponse,
    }
}

fn graphql_category(errors: &[GraphqlError]) -> TrackerErrorCategory {
    for error in errors {
        if let Some(code) = &error.code {
            let normalized = code.to_ascii_lowercase();
            if normalized.contains("auth") {
                return TrackerErrorCategory::Auth;
            }
            if normalized.contains("forbidden") || normalized.contains("permission") {
                return TrackerErrorCategory::PermissionDenied;
            }
            if normalized.contains("rate") || normalized.contains("throttle") {
                return TrackerErrorCategory::RateLimited;
            }
            if normalized.contains("not_found") || normalized.contains("notfound") {
                return TrackerErrorCategory::NotFound;
            }
            if normalized.contains("invalid_state") {
                return TrackerErrorCategory::InvalidStateTransition;
            }
        }

        let message = error.message.to_ascii_lowercase();
        if message.contains("permission") || message.contains("forbidden") {
            return TrackerErrorCategory::PermissionDenied;
        }
        if message.contains("rate limit") || message.contains("too many requests") {
            return TrackerErrorCategory::RateLimited;
        }
        if message.contains("authentication") || message.contains("unauthorized") {
            return TrackerErrorCategory::Auth;
        }
        if message.contains("not found") {
            return TrackerErrorCategory::NotFound;
        }
        if message.contains("invalid state transition") {
            return TrackerErrorCategory::InvalidStateTransition;
        }
    }

    TrackerErrorCategory::InvalidResponse
}

#[cfg(test)]
mod tests {
    use crate::error::{GraphqlError, LinearMcpError, ToolFailure};

    #[test]
    fn tool_failures_preserve_invalid_state_transition_category() {
        let failure = ToolFailure::from(LinearMcpError::from_graphql_errors(vec![GraphqlError {
            message: "invalid state transition".to_string(),
            code: Some("INVALID_STATE_TRANSITION".to_string()),
            user_presentable_message: Some("State transition is not allowed.".to_string()),
        }]));

        assert_eq!(failure.code, "invalid_state_transition");
        assert_eq!(failure.message, "State transition is not allowed.");
    }
}

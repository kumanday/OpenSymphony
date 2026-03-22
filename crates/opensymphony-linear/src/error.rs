use std::time::Duration;

use opensymphony_domain::TrackerErrorCategory;
use reqwest::StatusCode;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphqlError {
    pub message: String,
    pub code: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum LinearError {
    #[error("invalid Linear client configuration: {0}")]
    InvalidConfiguration(String),
    #[error("Linear request failed: {0}")]
    Request(Box<reqwest::Error>),
    #[error("Linear API returned HTTP {status}: {body}")]
    HttpStatus {
        status: StatusCode,
        body: String,
        retry_after: Option<Duration>,
    },
    #[error("Linear GraphQL returned errors: {summary}")]
    Graphql {
        errors: Vec<GraphqlError>,
        summary: String,
        retry_after: Option<Duration>,
    },
    #[error("Linear omitted requested issue IDs from state refresh: {issue_ids:?}")]
    MissingIssueIds { issue_ids: Vec<String> },
    #[error("Linear API returned an invalid response: {0}")]
    InvalidResponse(String),
}

impl LinearError {
    pub fn from_graphql_errors(errors: Vec<GraphqlError>) -> Self {
        Self::from_graphql_errors_with_retry_after(errors, None)
    }

    pub fn from_graphql_errors_with_retry_after(
        errors: Vec<GraphqlError>,
        retry_after: Option<Duration>,
    ) -> Self {
        let summary = errors
            .iter()
            .map(|error| match &error.code {
                Some(code) => format!("{code}: {}", error.message),
                None => error.message.clone(),
            })
            .collect::<Vec<_>>()
            .join("; ");
        Self::Graphql {
            errors,
            summary,
            retry_after,
        }
    }

    pub fn category(&self) -> TrackerErrorCategory {
        match self {
            Self::MissingIssueIds { .. } => TrackerErrorCategory::NotFound,
            Self::InvalidConfiguration(_) | Self::InvalidResponse(_) => {
                TrackerErrorCategory::InvalidResponse
            }
            Self::Request(error) if error.is_timeout() => TrackerErrorCategory::Timeout,
            Self::Request(_) => TrackerErrorCategory::Transport,
            Self::HttpStatus { status, .. } => http_status_category(*status),
            Self::Graphql { errors, .. } => graphql_category(errors),
        }
    }

    pub fn is_rate_limited(&self) -> bool {
        self.category() == TrackerErrorCategory::RateLimited
    }

    pub fn retry_after(&self) -> Option<Duration> {
        match self {
            Self::HttpStatus { retry_after, .. } => *retry_after,
            Self::Graphql { retry_after, .. } => *retry_after,
            _ => None,
        }
    }
}

fn http_status_category(status: StatusCode) -> TrackerErrorCategory {
    match status {
        StatusCode::UNAUTHORIZED => TrackerErrorCategory::Auth,
        StatusCode::FORBIDDEN => TrackerErrorCategory::PermissionDenied,
        StatusCode::NOT_FOUND => TrackerErrorCategory::NotFound,
        StatusCode::TOO_MANY_REQUESTS => TrackerErrorCategory::RateLimited,
        status if status.is_server_error() => TrackerErrorCategory::Transport,
        _ => TrackerErrorCategory::InvalidResponse,
    }
}

fn graphql_category(errors: &[GraphqlError]) -> TrackerErrorCategory {
    for error in errors {
        if let Some(code) = &error.code {
            let code = code.to_ascii_lowercase();
            if code.contains("auth") {
                return TrackerErrorCategory::Auth;
            }
            if code.contains("forbidden") || code.contains("permission") {
                return TrackerErrorCategory::PermissionDenied;
            }
            if code.contains("rate") || code.contains("throttle") {
                return TrackerErrorCategory::RateLimited;
            }
            if code.contains("not_found") || code.contains("notfound") {
                return TrackerErrorCategory::NotFound;
            }
            if code.contains("invalid_state") {
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
    use std::time::Duration;

    use opensymphony_domain::TrackerErrorCategory;
    use reqwest::StatusCode;

    use crate::error::{GraphqlError, LinearError};

    #[test]
    fn http_statuses_map_to_tracker_categories() {
        let auth = LinearError::HttpStatus {
            status: StatusCode::UNAUTHORIZED,
            body: "unauthorized".to_string(),
            retry_after: None,
        };
        let permission_denied = LinearError::HttpStatus {
            status: StatusCode::FORBIDDEN,
            body: "forbidden".to_string(),
            retry_after: None,
        };
        let rate_limited = LinearError::HttpStatus {
            status: StatusCode::TOO_MANY_REQUESTS,
            body: "slow down".to_string(),
            retry_after: Some(Duration::from_secs(1)),
        };

        assert_eq!(auth.category(), TrackerErrorCategory::Auth);
        assert_eq!(
            permission_denied.category(),
            TrackerErrorCategory::PermissionDenied
        );
        assert_eq!(rate_limited.category(), TrackerErrorCategory::RateLimited);
    }

    #[test]
    fn graphql_errors_map_to_tracker_categories() {
        let forbidden = LinearError::from_graphql_errors(vec![GraphqlError {
            message: "viewer does not have permission".to_string(),
            code: Some("FORBIDDEN".to_string()),
        }]);
        let not_found = LinearError::from_graphql_errors(vec![GraphqlError {
            message: "issue not found".to_string(),
            code: Some("NOT_FOUND".to_string()),
        }]);
        let rate_limited = LinearError::from_graphql_errors_with_retry_after(
            vec![GraphqlError {
                message: "rate limit exceeded".to_string(),
                code: Some("RATELIMITED".to_string()),
            }],
            Some(Duration::from_secs(2)),
        );

        assert_eq!(forbidden.category(), TrackerErrorCategory::PermissionDenied);
        assert_eq!(not_found.category(), TrackerErrorCategory::NotFound);
        assert_eq!(rate_limited.category(), TrackerErrorCategory::RateLimited);
        assert_eq!(rate_limited.retry_after(), Some(Duration::from_secs(2)));
    }
}

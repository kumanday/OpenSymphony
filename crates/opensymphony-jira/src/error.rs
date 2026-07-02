use std::time::Duration;

use crate::opensymphony_domain::TrackerErrorCategory;
use reqwest::StatusCode;

#[derive(Debug, thiserror::Error)]
pub enum JiraError {
    #[error("invalid Jira client configuration: {0}")]
    InvalidConfiguration(String),
    #[error("Jira request failed: {0}")]
    Request(Box<reqwest::Error>),
    #[error("Jira response body read failed for {operation} after HTTP {status}: {source}")]
    ResponseBody {
        operation: String,
        status: StatusCode,
        retry_after: Option<Duration>,
        #[source]
        source: Box<reqwest::Error>,
    },
    #[error("Jira API returned HTTP {status}: {body}")]
    HttpStatus {
        status: StatusCode,
        body: String,
        retry_after: Option<Duration>,
    },
    #[error("Jira omitted requested issues: {issue_ids:?}")]
    MissingIssueIds { issue_ids: Vec<String> },
    #[error("Jira API returned an invalid response: {0}")]
    InvalidResponse(String),
}

impl JiraError {
    pub fn category(&self) -> TrackerErrorCategory {
        match self {
            Self::MissingIssueIds { .. } => TrackerErrorCategory::NotFound,
            Self::InvalidConfiguration(_) | Self::InvalidResponse(_) => {
                TrackerErrorCategory::InvalidResponse
            }
            Self::ResponseBody { source, .. } if source.is_timeout() => {
                TrackerErrorCategory::Timeout
            }
            Self::ResponseBody { .. } => TrackerErrorCategory::Transport,
            Self::Request(error) if error.is_timeout() => TrackerErrorCategory::Timeout,
            Self::Request(_) => TrackerErrorCategory::Transport,
            Self::HttpStatus { status, .. } => http_status_category(*status),
        }
    }

    pub fn is_rate_limited(&self) -> bool {
        self.category() == TrackerErrorCategory::RateLimited
    }

    pub fn retry_after(&self) -> Option<Duration> {
        match self {
            Self::HttpStatus { retry_after, .. } => *retry_after,
            Self::ResponseBody { retry_after, .. } => *retry_after,
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

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use crate::opensymphony_domain::TrackerErrorCategory;
    use reqwest::StatusCode;

    use super::JiraError;

    #[test]
    fn http_statuses_map_to_tracker_categories() {
        let auth = JiraError::HttpStatus {
            status: StatusCode::UNAUTHORIZED,
            body: "unauthorized".to_string(),
            retry_after: None,
        };
        let permission_denied = JiraError::HttpStatus {
            status: StatusCode::FORBIDDEN,
            body: "forbidden".to_string(),
            retry_after: None,
        };
        let rate_limited = JiraError::HttpStatus {
            status: StatusCode::TOO_MANY_REQUESTS,
            body: "slow down".to_string(),
            retry_after: Some(Duration::from_secs(3)),
        };
        let not_found = JiraError::HttpStatus {
            status: StatusCode::NOT_FOUND,
            body: "missing".to_string(),
            retry_after: None,
        };

        assert_eq!(auth.category(), TrackerErrorCategory::Auth);
        assert_eq!(
            permission_denied.category(),
            TrackerErrorCategory::PermissionDenied
        );
        assert_eq!(rate_limited.category(), TrackerErrorCategory::RateLimited);
        assert!(rate_limited.is_rate_limited());
        assert_eq!(rate_limited.retry_after(), Some(Duration::from_secs(3)));
        assert_eq!(not_found.category(), TrackerErrorCategory::NotFound);
    }

    #[test]
    fn missing_issue_ids_map_to_not_found() {
        let error = JiraError::MissingIssueIds {
            issue_ids: vec!["OSYM-1".to_string()],
        };
        assert_eq!(error.category(), TrackerErrorCategory::NotFound);
        assert_eq!(error.retry_after(), None);
    }
}

//! Devin cloud harness client.
//!
//! Devin is a remote implementation agent: the agent runtime, the checkout, and
//! the shell all live in a Devin-owned cloud workspace. OpenSymphony therefore
//! keeps its local sanitized issue workspace for manifests, journals, and
//! evidence only, and binds the repository to the remote workspace through
//! [`DevinRemoteWorkspaceBinding`] instead of handing the local checkout to the
//! harness as an execution cwd.
//!
//! The wire contract implemented here is the published Devin API v3
//! (`contracts/devin-v3-openapi.yaml`, a session/identity subset mirrored from
//! <https://docs.devin.ai/v3-openapi.yaml>). Consequences of the real contract:
//!
//! * Every session route is organization-scoped:
//!   `/v3/organizations/{org_id}/sessions/...`, authenticated with a service
//!   user credential (`cog_` prefix). The organization ID is not a secret and
//!   may be configured directly or discovered through `GET /v3/self`.
//! * There is no run resource and no event stream. A session is created with a
//!   prompt, progress is read from cursor-paginated
//!   `GET .../sessions/{devin_id}/messages`, and lifecycle state is read from
//!   `status` / `status_detail` on the session resource.
//! * Repository binding is a real request field (`repos`), expressed as
//!   `owner/name` entries; branch intent still travels in the prompt because
//!   the API has no branch field.
//!
//! Hosted-mode posture implemented here:
//!
//! * Transport is HTTPS-only with TLS 1.2 as the floor, redirects refused (the
//!   bearer token would otherwise follow a redirect to another host), and the
//!   credential read from the environment at request time only.
//! * Tenancy is bound before any session call through `GET /v3/self`: the
//!   credential's organization must match the configured one, and every
//!   session payload is re-checked against it so a response for another
//!   organization is rejected rather than journaled.
//! * Secret injection references organization secrets by id or key; the
//!   references are resolved against `GET /v3/organizations/{org_id}/secrets`
//!   so a run cannot request a secret outside its tenant, and secret *values*
//!   never enter OpenSymphony.
//! * Remote artifacts come back through [`DevinEvidence`]: session summary,
//!   normalized event journal, pull requests, and same-origin attachment
//!   downloads are written into the local evidence workspace.

use std::{
    collections::HashSet,
    fmt,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use url::Url;

use crate::{
    opensymphony_domain::HarnessAdapter,
    opensymphony_gateway_schema::{
        capability::HarnessCapability,
        envelope::EntityRef,
        event_journal::{EventActor, EventKind, EventRecord},
    },
};

pub const DEVIN_CLOUD_AGENT_KIND: &str = "devin_cloud_agent";
/// Wire contract implemented by this module, pinned to the vendored spec.
pub const DEVIN_CLOUD_API_CONTRACT: &str = "devin-api-v3";
/// Devin API origin. Version-prefixed paths are appended by the request builder.
pub const DEFAULT_DEVIN_API_BASE_URL: &str = "https://api.devin.ai";
/// Devin API v3 authenticates with a `cog_` service-user token.
pub const DEFAULT_DEVIN_API_KEY_ENV: &str = "COG_SERVICE_USER_TOKEN";
pub const DEFAULT_DEVIN_ORG_ID_ENV: &str = "DEVIN_ORG_ID";
pub const DEFAULT_DEVIN_SESSION_POLL_INTERVAL_MS: u64 = 5_000;
pub const DEFAULT_DEVIN_REQUEST_TIMEOUT_MS: u64 = 30_000;
/// Maximum number of tags Devin accepts on a session.
pub const DEVIN_MAX_SESSION_TAGS: usize = 50;
/// Maximum page size accepted by the paginated v3 collection endpoints.
pub const DEVIN_MAX_PAGE_SIZE: u32 = 200;
/// Prefix used for the tag that correlates a Devin session with an issue.
pub const DEVIN_CORRELATION_TAG_PREFIX: &str = "opensymphony";

/// Synthetic event type recorded when a Devin payload carries no recognized
/// discriminator field, so the payload is preserved rather than dropped.
pub const DEVIN_UNDISCRIMINATED_EVENT_KIND: &str = "devin.undiscriminated_event";

/// Effective containment label recorded for runs routed to Devin.
///
/// The local workspace is never the execution directory for this harness.
pub const DEVIN_REMOTE_CONTAINMENT: &str = "devin_owned_remote_workspace";

#[derive(Debug, thiserror::Error)]
pub enum DevinConfigError {
    #[error("devin.api.base_url must be an absolute https URL: {0}")]
    InvalidBaseUrl(String),
    #[error("devin.api.base_url must use the https scheme for remote Devin routing")]
    InsecureBaseUrl,
    #[error("devin.api.base_url must not embed credentials")]
    CredentialsInBaseUrl,
    #[error("devin.api.base_url must not carry a query string or fragment")]
    DecoratedBaseUrl,
    #[error("devin.api.api_key_env must be an environment variable name")]
    InvalidApiKeyEnv,
    #[error("devin.api.org_id must look like `org-...`")]
    InvalidOrgId,
    #[error("devin.api.org_id_env must be an environment variable name")]
    InvalidOrgIdEnv,
    #[error("the organization id read from the environment must look like `org-...`")]
    InvalidOrgIdFromEnvironment,
    #[error("devin.session.max_acu_limit must be greater than zero")]
    InvalidAcuLimit,
    #[error("devin sessions accept at most {DEVIN_MAX_SESSION_TAGS} tags")]
    TooManyTags,
    #[error("devin session tags must not be blank")]
    BlankTag,
    #[error("devin repository binding must be an absolute https git URL: {0}")]
    InvalidRepositoryUrl(String),
}

#[derive(Debug, thiserror::Error)]
pub enum DevinClientError {
    #[error("devin api credential `{env}` is not set in the worker environment")]
    MissingCredential { env: String },
    #[error(
        "devin organization id is not configured and could not be resolved from the credential"
    )]
    MissingOrgId,
    #[error("failed to build devin api url from `{base_url}` and `{path}`: {source}")]
    InvalidUrl {
        base_url: String,
        path: String,
        #[source]
        source: url::ParseError,
    },
    #[error("devin api transport error: {0}")]
    Transport(String),
    #[error("devin api returned status {status}: {problem}")]
    Status { status: u16, problem: String },
    #[error("failed to decode devin api response: {0}")]
    Decode(String),
    #[error("invalid devin api configuration: {0}")]
    InvalidConfig(String),
    #[error("devin session `{session_id}` did not settle within {elapsed:?}")]
    PollTimeout {
        session_id: String,
        elapsed: Duration,
    },
    #[error(
        "devin credential belongs to organization `{credential}` but routing is configured for `{configured}`"
    )]
    TenantMismatch {
        configured: String,
        credential: String,
    },
    #[error(
        "devin returned a payload owned by organization `{observed}` while bound to `{expected}`"
    )]
    CrossTenantPayload { expected: String, observed: String },
    #[error("devin secret `{reference}` is not available to organization `{org_id}`")]
    UnknownSecret { reference: String, org_id: String },
    #[error("devin attachment `{name}` is served from `{origin}`, which is not the api origin")]
    ForeignAttachmentOrigin { name: String, origin: String },
    #[error("devin attachment `{name}` exceeds the {limit} byte evidence download limit")]
    AttachmentTooLarge { name: String, limit: u64 },
    #[error("failed to persist devin evidence at {path}: {source}")]
    Evidence {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// API token wrapper that never renders its secret value.
#[derive(Clone, PartialEq, Eq)]
pub struct DevinApiToken(String);

impl DevinApiToken {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for DevinApiToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("DevinApiToken(redacted)")
    }
}

/// Session creation options that map onto documented `SessionCreateRequest`
/// fields. Everything here is operator configuration, never credential values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevinSessionOptions {
    pub playbook_id: Option<String>,
    pub knowledge_ids: Option<Vec<String>>,
    /// References to organization secrets Devin injects into the session. The
    /// values live in Devin, not in OpenSymphony configuration.
    pub secret_ids: Option<Vec<String>>,
    pub max_acu_limit: Option<u32>,
    pub tags: Vec<String>,
    pub title: Option<String>,
    pub devin_mode: Option<DevinMode>,
    /// VM platform or outpost pool override.
    pub platform: Option<String>,
    /// Preserve VM state after the session stops so it can be resumed.
    pub resumable: bool,
}

impl Default for DevinSessionOptions {
    fn default() -> Self {
        Self {
            playbook_id: None,
            knowledge_ids: None,
            secret_ids: None,
            max_acu_limit: None,
            tags: Vec::new(),
            title: None,
            devin_mode: None,
            platform: None,
            // The documented `resumable` default in the v3 contract.
            resumable: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevinCloudConfig {
    pub base_url: String,
    pub api_key_env: String,
    /// Organization that owns the sessions. Resolved from `GET /v3/self` when
    /// unset; this is an identifier, not a credential.
    pub org_id: Option<String>,
    /// Environment variable consulted for the organization id when `org_id` is
    /// unset.
    pub org_id_env: String,
    pub poll_interval: Duration,
    pub request_timeout: Duration,
    pub session: DevinSessionOptions,
}

impl Default for DevinCloudConfig {
    fn default() -> Self {
        Self {
            base_url: DEFAULT_DEVIN_API_BASE_URL.to_owned(),
            api_key_env: DEFAULT_DEVIN_API_KEY_ENV.to_owned(),
            org_id: None,
            org_id_env: DEFAULT_DEVIN_ORG_ID_ENV.to_owned(),
            poll_interval: Duration::from_millis(DEFAULT_DEVIN_SESSION_POLL_INTERVAL_MS),
            request_timeout: Duration::from_millis(DEFAULT_DEVIN_REQUEST_TIMEOUT_MS),
            session: DevinSessionOptions::default(),
        }
    }
}

impl DevinCloudConfig {
    pub fn validate(&self) -> Result<(), DevinConfigError> {
        let parsed = Url::parse(&self.base_url)
            .map_err(|error| DevinConfigError::InvalidBaseUrl(error.to_string()))?;
        if parsed.scheme() != "https" {
            return Err(DevinConfigError::InsecureBaseUrl);
        }
        if parsed.host().is_none() {
            return Err(DevinConfigError::InvalidBaseUrl("missing host".into()));
        }
        if !parsed.username().is_empty() || parsed.password().is_some() {
            return Err(DevinConfigError::CredentialsInBaseUrl);
        }
        if parsed.query().is_some() || parsed.fragment().is_some() {
            return Err(DevinConfigError::DecoratedBaseUrl);
        }
        if !is_environment_name(&self.api_key_env) {
            return Err(DevinConfigError::InvalidApiKeyEnv);
        }
        if let Some(org_id) = self.org_id.as_deref()
            && !is_org_id(org_id)
        {
            return Err(DevinConfigError::InvalidOrgId);
        }
        if !self.org_id_env.is_empty() && !is_environment_name(&self.org_id_env) {
            return Err(DevinConfigError::InvalidOrgIdEnv);
        }
        if self.session.max_acu_limit == Some(0) {
            return Err(DevinConfigError::InvalidAcuLimit);
        }
        if self.session.tags.len() > DEVIN_MAX_SESSION_TAGS {
            return Err(DevinConfigError::TooManyTags);
        }
        if self.session.tags.iter().any(|tag| tag.trim().is_empty()) {
            return Err(DevinConfigError::BlankTag);
        }

        Ok(())
    }
}

fn is_environment_name(value: &str) -> bool {
    !value.is_empty()
        && value
            .chars()
            .all(|character| character == '_' || character.is_ascii_alphanumeric())
        && value
            .chars()
            .next()
            .is_some_and(|character| character == '_' || character.is_ascii_alphabetic())
}

fn is_org_id(value: &str) -> bool {
    value.len() > "org-".len()
        && value.starts_with("org-")
        && value
            .chars()
            .all(|character| character == '-' || character.is_ascii_alphanumeric())
}

/// Repository binding for a Devin-owned remote workspace.
///
/// `local_evidence_path` is the sanitized OpenSymphony issue workspace. It holds
/// run manifests, journals, and evidence for the run; it is *not* the directory
/// Devin executes in and must never be advertised as such.
///
/// The v3 create-session request carries repositories as `owner/name` entries,
/// so the binding renders both the request field and the prompt context that
/// communicates branch intent (which the API has no field for).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevinRemoteWorkspaceBinding {
    pub issue_workspace_key: String,
    pub local_evidence_path: PathBuf,
    pub repository_url: String,
    pub base_branch: Option<String>,
}

impl DevinRemoteWorkspaceBinding {
    pub fn new(
        issue_workspace_key: impl Into<String>,
        local_evidence_path: impl Into<PathBuf>,
        repository_url: impl Into<String>,
        base_branch: Option<String>,
    ) -> Result<Self, DevinConfigError> {
        let repository_url = repository_url.into();
        let parsed = Url::parse(&repository_url)
            .map_err(|error| DevinConfigError::InvalidRepositoryUrl(error.to_string()))?;
        if parsed.scheme() != "https" {
            return Err(DevinConfigError::InvalidRepositoryUrl(
                "must use the https scheme".into(),
            ));
        }
        if !parsed.username().is_empty() || parsed.password().is_some() {
            return Err(DevinConfigError::InvalidRepositoryUrl(
                "must not embed credentials".into(),
            ));
        }
        if parsed.query().is_some() || parsed.fragment().is_some() {
            return Err(DevinConfigError::InvalidRepositoryUrl(
                "must not carry a query string or fragment".into(),
            ));
        }

        Ok(Self {
            issue_workspace_key: issue_workspace_key.into(),
            local_evidence_path: local_evidence_path.into(),
            repository_url,
            base_branch,
        })
    }

    /// Containment label for the run: Devin owns the execution environment.
    pub fn effective_containment(&self) -> &'static str {
        DEVIN_REMOTE_CONTAINMENT
    }

    /// `owner/name` form expected by the `repos` request field.
    pub fn repo_name(&self) -> Option<String> {
        let parsed = Url::parse(&self.repository_url).ok()?;
        let segments: Vec<&str> = parsed
            .path_segments()?
            .filter(|segment| !segment.is_empty())
            .collect();
        let [.., owner, name] = segments.as_slice() else {
            return None;
        };
        Some(format!("{owner}/{}", name.trim_end_matches(".git")))
    }

    /// Tag that correlates a Devin session with the OpenSymphony issue, used to
    /// recover an in-flight session through the tag filter on list sessions.
    pub fn correlation_tag(&self) -> String {
        format!(
            "{DEVIN_CORRELATION_TAG_PREFIX}:{}",
            self.issue_workspace_key
        )
    }

    /// Repository and branch context rendered into the session prompt. The
    /// create-session request has no branch field, so branch intent is stated
    /// here.
    pub fn prompt_context(&self) -> String {
        let mut context = format!("Repository: {}\n", self.repository_url);
        if let Some(branch) = self.base_branch.as_deref() {
            context.push_str(&format!("Base branch: {branch}\n"));
        }
        context.push_str(&format!(
            "OpenSymphony issue workspace: {} (orchestrator-side evidence only; work in your own Devin workspace)\n",
            self.issue_workspace_key
        ));
        context
    }

    /// Prompt for a Devin session: repository context followed by the task.
    pub fn compose_prompt(&self, task_prompt: &str) -> String {
        format!("{}\n{}", self.prompt_context(), task_prompt.trim())
    }
}

#[derive(Debug, Clone, Default)]
pub struct DevinCloudAdapter {
    config: DevinCloudConfig,
}

impl DevinCloudAdapter {
    pub fn new(config: DevinCloudConfig) -> Self {
        Self { config }
    }

    pub fn config(&self) -> &DevinCloudConfig {
        &self.config
    }

    /// Remaining feature gaps for hosted Devin routing.
    pub fn feature_gaps(&self) -> Vec<String> {
        self.capabilities().feature_gaps
    }
}

impl HarnessAdapter for DevinCloudAdapter {
    fn harness_kind(&self) -> &'static str {
        DEVIN_CLOUD_AGENT_KIND
    }

    fn capabilities(&self) -> HarnessCapability {
        HarnessCapability::devin_cloud_agent()
    }
}

// ---------------------------------------------------------------------------
// Wire models (Devin API v3)
// ---------------------------------------------------------------------------

/// `devin_mode` from `SessionCreateRequest`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DevinMode {
    Normal,
    Fast,
    Lite,
    Ultra,
    Fusion,
}

impl DevinMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Fast => "fast",
            Self::Lite => "lite",
            Self::Ultra => "ultra",
            Self::Fusion => "fusion",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "normal" => Some(Self::Normal),
            "fast" => Some(Self::Fast),
            "lite" => Some(Self::Lite),
            "ultra" => Some(Self::Ultra),
            "fusion" => Some(Self::Fusion),
            _ => None,
        }
    }
}

/// `SessionCreateRequest` from the Devin v3 OpenAPI document.
///
/// Fields OpenSymphony does not drive (`session_secrets`, `create_as_user_id`,
/// `bypass_approval`, attachment uploads) are deliberately omitted: they are
/// part of the hosted-security work that still needs hardening evidence.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SessionCreateRequest {
    pub prompt: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub repos: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub tags: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub playbook_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub knowledge_ids: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub secret_ids: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub max_acu_limit: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub devin_mode: Option<DevinMode>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub platform: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub resumable: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub structured_output_schema: Option<Value>,
}

/// `status` from `SessionResponse`. Unknown values are kept so a newer Devin
/// status never fails decoding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DevinSessionStatus {
    New,
    Claimed,
    Running,
    Exit,
    Error,
    Suspended,
    Resuming,
    #[serde(untagged)]
    Other(String),
}

impl DevinSessionStatus {
    pub fn as_str(&self) -> &str {
        match self {
            Self::New => "new",
            Self::Claimed => "claimed",
            Self::Running => "running",
            Self::Exit => "exit",
            Self::Error => "error",
            Self::Suspended => "suspended",
            Self::Resuming => "resuming",
            Self::Other(value) => value,
        }
    }

    /// Statuses Devin will not leave without operator or platform action.
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Exit | Self::Error | Self::Suspended)
    }
}

/// `status_detail` from `SessionResponse`. Kept as a string so unlisted detail
/// values survive decoding; the interesting ones are exposed as predicates.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DevinStatusDetail(pub String);

impl DevinStatusDetail {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Devin finished the task but the session is still running.
    pub fn is_finished(&self) -> bool {
        self.0 == "finished"
    }

    /// Devin needs operator input or an action approval before continuing.
    pub fn is_waiting_on_operator(&self) -> bool {
        matches!(self.0.as_str(), "waiting_for_user" | "waiting_for_approval")
    }
}

/// `SessionPullRequest` from the Devin v3 OpenAPI document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionPullRequest {
    pub pr_url: String,
    pub pr_state: Option<String>,
}

/// `SessionResponse` from the Devin v3 OpenAPI document. Unknown fields are
/// retained so newer Devin payloads replay unchanged.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionResponse {
    pub session_id: String,
    pub url: String,
    pub status: DevinSessionStatus,
    #[serde(default)]
    pub status_detail: Option<DevinStatusDetail>,
    #[serde(default)]
    pub tags: Vec<String>,
    pub org_id: String,
    pub created_at: i64,
    pub updated_at: i64,
    #[serde(default)]
    pub acus_consumed: f64,
    #[serde(default)]
    pub pull_requests: Vec<SessionPullRequest>,
    #[serde(default)]
    pub structured_output: Option<Value>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub is_archived: bool,
    #[serde(default)]
    pub devin_mode: Option<DevinMode>,
    #[serde(default)]
    pub parent_session_id: Option<String>,
    #[serde(default)]
    pub playbook_id: Option<String>,
    #[serde(flatten, default)]
    pub extra: Map<String, Value>,
}

impl SessionResponse {
    /// The session is done as far as the orchestrator is concerned: either the
    /// VM stopped, or Devin reported the task finished.
    pub fn is_settled(&self) -> bool {
        self.status.is_terminal()
            || self
                .status_detail
                .as_ref()
                .is_some_and(DevinStatusDetail::is_finished)
    }

    pub fn is_waiting_on_operator(&self) -> bool {
        self.status_detail
            .as_ref()
            .is_some_and(DevinStatusDetail::is_waiting_on_operator)
    }
}

/// `SessionMessage` from the Devin v3 OpenAPI document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionMessage {
    pub event_id: String,
    pub source: String,
    pub message: String,
    pub created_at: i64,
    #[serde(flatten, default)]
    pub extra: Map<String, Value>,
}

/// `SessionMessageCreateRequest` from the Devin v3 OpenAPI document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionMessageCreateRequest {
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub attachment_urls: Option<Vec<String>>,
}

/// `PaginatedResponse[T]` from the Devin v3 OpenAPI document.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PaginatedResponse<T> {
    pub items: Vec<T>,
    #[serde(default)]
    pub end_cursor: Option<String>,
    #[serde(default)]
    pub has_next_page: bool,
    #[serde(default)]
    pub total: Option<i64>,
}

/// `SessionAttachment` from the Devin v3 OpenAPI document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionAttachment {
    pub attachment_id: String,
    pub name: String,
    pub url: String,
    pub source: String,
    #[serde(default)]
    pub content_type: Option<String>,
}

/// `SecretResponse` from the Devin v3 OpenAPI document.
///
/// Secret *values* are never returned by the API and never enter OpenSymphony:
/// this record only carries the identifiers used to scope injection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecretResponse {
    pub secret_id: String,
    #[serde(default)]
    pub key: Option<String>,
    #[serde(default)]
    pub access_type: Option<String>,
    #[serde(default)]
    pub secret_type: Option<String>,
}

/// `SessionTagsUpdateRequest` from the Devin v3 OpenAPI document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionTagsUpdateRequest {
    pub tags: Vec<String>,
}

/// Subset of the `GET /v3/self` response that identifies the calling principal.
///
/// The four documented principal shapes all optionally carry `org_id`, which is
/// the only field this harness needs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DevinSelfResponse {
    #[serde(default)]
    pub principal_type: Option<String>,
    #[serde(default)]
    pub org_id: Option<String>,
    #[serde(default)]
    pub service_user_id: Option<String>,
    #[serde(default)]
    pub service_user_name: Option<String>,
}

/// `ProblemDetail` (RFC 9457) error body returned by every v3 route.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DevinProblemDetail {
    pub title: String,
    pub status: i64,
    #[serde(default)]
    pub detail: Option<String>,
    #[serde(default, rename = "type")]
    pub problem_type: Option<String>,
    #[serde(default)]
    pub instance: Option<String>,
    #[serde(default)]
    pub errors: Option<Vec<Map<String, Value>>>,
}

impl fmt::Display for DevinProblemDetail {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.detail.as_deref() {
            Some(detail) => write!(formatter, "{}: {detail}", self.title),
            None => formatter.write_str(&self.title),
        }
    }
}

/// Filters for `GET /v3/organizations/{org_id}/sessions`, serialized into the
/// documented `qs` query parameter.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionsQueryParams {
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub tags: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub session_ids: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub repo_names: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub is_archived: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub after: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub first: Option<u32>,
}

impl SessionsQueryParams {
    pub fn by_tag(tag: impl Into<String>) -> Self {
        Self {
            tags: Some(vec![tag.into()]),
            ..Self::default()
        }
    }
}

// ---------------------------------------------------------------------------
// Requests
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DevinHttpMethod {
    Get,
    Post,
    Put,
    Delete,
}

impl DevinHttpMethod {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Post => "POST",
            Self::Put => "PUT",
            Self::Delete => "DELETE",
        }
    }
}

/// Documented operations this client uses, named after the OpenAPI `summary`
/// values they map to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DevinOperation {
    GetSelf,
    CreateSession,
    GetSession,
    ListSessions,
    ListSessionMessages,
    SendSessionMessage,
    DeleteSession,
    ListSessionAttachments,
    UpdateSessionTags,
    ListSecrets,
}

impl DevinOperation {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::GetSelf => "get_self",
            Self::CreateSession => "create_session",
            Self::GetSession => "get_session",
            Self::ListSessions => "list_sessions",
            Self::ListSessionMessages => "list_session_messages",
            Self::SendSessionMessage => "send_session_message",
            Self::DeleteSession => "delete_session",
            Self::ListSessionAttachments => "list_session_attachments",
            Self::UpdateSessionTags => "update_session_tags",
            Self::ListSecrets => "list_secrets",
        }
    }
}

/// Transport-agnostic description of a Devin cloud API call.
///
/// Credentials are injected by [`DevinCloudClient`] at send time so request
/// descriptions stay safe to log, snapshot, and unit test.
#[derive(Debug, Clone, PartialEq)]
pub struct DevinRequest {
    pub operation: DevinOperation,
    pub method: DevinHttpMethod,
    pub path: String,
    pub body: Option<Value>,
}

/// Builds organization-scoped v3 request descriptions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevinRequestBuilder {
    base_url: String,
    org_id: String,
}

impl DevinRequestBuilder {
    pub fn new(base_url: impl Into<String>, org_id: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            org_id: org_id.into(),
        }
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    pub fn org_id(&self) -> &str {
        &self.org_id
    }

    fn sessions_path(&self) -> String {
        format!(
            "/v3/organizations/{}/sessions",
            encode_path_segment(&self.org_id)
        )
    }

    fn session_path(&self, devin_id: &str) -> String {
        format!("{}/{}", self.sessions_path(), encode_path_segment(devin_id))
    }

    /// `GET /v3/self`
    pub fn get_self(&self) -> DevinRequest {
        DevinRequest {
            operation: DevinOperation::GetSelf,
            method: DevinHttpMethod::Get,
            path: "/v3/self".into(),
            body: None,
        }
    }

    /// `POST /v3/organizations/{org_id}/sessions`
    pub fn create_session(&self, request: &SessionCreateRequest) -> DevinRequest {
        DevinRequest {
            operation: DevinOperation::CreateSession,
            method: DevinHttpMethod::Post,
            path: self.sessions_path(),
            body: Some(serde_json::to_value(request).unwrap_or_else(|_| json!({}))),
        }
    }

    /// `GET /v3/organizations/{org_id}/sessions/{devin_id}`
    pub fn get_session(&self, devin_id: &str) -> DevinRequest {
        DevinRequest {
            operation: DevinOperation::GetSession,
            method: DevinHttpMethod::Get,
            path: self.session_path(devin_id),
            body: None,
        }
    }

    /// `GET /v3/organizations/{org_id}/sessions?qs=`
    ///
    /// The documented `qs` parameter is a single object-valued query parameter,
    /// so the filter set is JSON-encoded into it.
    pub fn list_sessions(&self, query: &SessionsQueryParams) -> DevinRequest {
        let encoded = serde_json::to_string(query).unwrap_or_else(|_| "{}".to_owned());
        DevinRequest {
            operation: DevinOperation::ListSessions,
            method: DevinHttpMethod::Get,
            path: format!(
                "{}?qs={}",
                self.sessions_path(),
                encode_query_value(&encoded)
            ),
            body: None,
        }
    }

    /// `GET /v3/organizations/{org_id}/sessions/{devin_id}/messages`
    pub fn list_messages(
        &self,
        devin_id: &str,
        after: Option<&str>,
        first: Option<u32>,
    ) -> DevinRequest {
        let mut path = format!("{}/messages", self.session_path(devin_id));
        let mut separator = '?';
        if let Some(after) = after {
            path.push_str(&format!("{separator}after={}", encode_query_value(after)));
            separator = '&';
        }
        if let Some(first) = first {
            let first = first.clamp(1, DEVIN_MAX_PAGE_SIZE);
            path.push_str(&format!("{separator}first={first}"));
        }

        DevinRequest {
            operation: DevinOperation::ListSessionMessages,
            method: DevinHttpMethod::Get,
            path,
            body: None,
        }
    }

    /// `POST /v3/organizations/{org_id}/sessions/{devin_id}/messages`
    pub fn send_message(&self, devin_id: &str, message: impl Into<String>) -> DevinRequest {
        let body = SessionMessageCreateRequest {
            message: message.into(),
            attachment_urls: None,
        };
        DevinRequest {
            operation: DevinOperation::SendSessionMessage,
            method: DevinHttpMethod::Post,
            path: format!("{}/messages", self.session_path(devin_id)),
            body: Some(serde_json::to_value(&body).unwrap_or_else(|_| json!({}))),
        }
    }

    /// `DELETE /v3/organizations/{org_id}/sessions/{devin_id}`
    pub fn delete_session(&self, devin_id: &str, archive: bool) -> DevinRequest {
        DevinRequest {
            operation: DevinOperation::DeleteSession,
            method: DevinHttpMethod::Delete,
            path: format!("{}?archive={archive}", self.session_path(devin_id)),
            body: None,
        }
    }

    /// `GET /v3/organizations/{org_id}/sessions/{devin_id}/attachments`
    pub fn list_attachments(&self, devin_id: &str) -> DevinRequest {
        DevinRequest {
            operation: DevinOperation::ListSessionAttachments,
            method: DevinHttpMethod::Get,
            path: format!("{}/attachments", self.session_path(devin_id)),
            body: None,
        }
    }

    /// `PUT /v3/organizations/{org_id}/sessions/{devin_id}/tags`
    pub fn replace_tags(&self, devin_id: &str, tags: Vec<String>) -> DevinRequest {
        let body = SessionTagsUpdateRequest { tags };
        DevinRequest {
            operation: DevinOperation::UpdateSessionTags,
            method: DevinHttpMethod::Put,
            path: format!("{}/tags", self.session_path(devin_id)),
            body: Some(serde_json::to_value(&body).unwrap_or_else(|_| json!({}))),
        }
    }

    /// `GET /v3/organizations/{org_id}/secrets`
    ///
    /// Used to scope secret injection: a run may only reference secrets that
    /// exist in its own organization.
    pub fn list_secrets(&self, after: Option<&str>, first: Option<u32>) -> DevinRequest {
        let mut path = format!(
            "/v3/organizations/{}/secrets",
            encode_path_segment(&self.org_id)
        );
        let mut separator = '?';
        if let Some(after) = after {
            path.push_str(&format!("{separator}after={}", encode_query_value(after)));
            separator = '&';
        }
        if let Some(first) = first {
            let first = first.clamp(1, DEVIN_MAX_PAGE_SIZE);
            path.push_str(&format!("{separator}first={first}"));
        }

        DevinRequest {
            operation: DevinOperation::ListSecrets,
            method: DevinHttpMethod::Get,
            path,
            body: None,
        }
    }

    pub fn absolute_url(&self, request: &DevinRequest) -> Result<Url, DevinClientError> {
        let joined = format!("{}{}", self.base_url.trim_end_matches('/'), request.path);
        Url::parse(&joined).map_err(|source| DevinClientError::InvalidUrl {
            base_url: self.base_url.clone(),
            path: request.path.clone(),
            source,
        })
    }
}

/// Builds the documented create-session payload for an issue run.
pub fn session_create_request(
    task_prompt: &str,
    binding: &DevinRemoteWorkspaceBinding,
    options: &DevinSessionOptions,
) -> SessionCreateRequest {
    let mut tags = options.tags.clone();
    let correlation = binding.correlation_tag();
    if !tags.iter().any(|tag| tag == &correlation) {
        tags.push(correlation);
    }
    tags.truncate(DEVIN_MAX_SESSION_TAGS);

    SessionCreateRequest {
        prompt: binding.compose_prompt(task_prompt),
        repos: binding.repo_name().map(|repo| vec![repo]),
        tags: Some(tags),
        title: options.title.clone(),
        playbook_id: options.playbook_id.clone(),
        knowledge_ids: options.knowledge_ids.clone(),
        secret_ids: options.secret_ids.clone(),
        max_acu_limit: options.max_acu_limit,
        devin_mode: options.devin_mode,
        platform: options.platform.clone(),
        resumable: Some(options.resumable),
        structured_output_schema: None,
    }
}

fn encode_path_segment(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            encoded.push(char::from(byte));
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

fn encode_query_value(value: &str) -> String {
    encode_path_segment(value)
}

// ---------------------------------------------------------------------------
// Client
// ---------------------------------------------------------------------------

/// Verified identity of the credential driving a Devin run.
///
/// Produced by [`DevinCloudClient::bind_tenant`], which is the only path that
/// marks a client as usable for session traffic.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DevinTenancy {
    pub org_id: String,
    #[serde(default)]
    pub principal_type: Option<String>,
    #[serde(default)]
    pub service_user_id: Option<String>,
    #[serde(default)]
    pub service_user_name: Option<String>,
}

/// HTTPS client for the Devin cloud API.
#[derive(Debug, Clone)]
pub struct DevinCloudClient {
    http: reqwest::Client,
    base_url: String,
    org_id: Option<String>,
    token: DevinApiToken,
    poll_interval: Duration,
    tenancy: Option<DevinTenancy>,
}

impl DevinCloudClient {
    /// Builds a client, reading the API token from the configured environment
    /// variable. The token is never stored in configuration or manifests.
    pub fn from_environment(
        config: &DevinCloudConfig,
        environment: impl Fn(&str) -> Option<String>,
    ) -> Result<Self, DevinClientError> {
        config
            .validate()
            .map_err(|error| DevinClientError::InvalidConfig(error.to_string()))?;
        let token = environment(&config.api_key_env)
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| DevinClientError::MissingCredential {
                env: config.api_key_env.clone(),
            })?;
        // The organization identifier is not a credential; it is read from the
        // environment only so operators can keep it beside the token.
        let org_id = match config.org_id.clone() {
            Some(org_id) => Some(org_id),
            None if config.org_id_env.is_empty() => None,
            None => environment(&config.org_id_env)
                .map(|value| value.trim().to_owned())
                .filter(|value| !value.is_empty()),
        };
        if let Some(org_id) = org_id.as_deref()
            && !is_org_id(org_id)
        {
            return Err(DevinClientError::InvalidConfig(
                DevinConfigError::InvalidOrgIdFromEnvironment.to_string(),
            ));
        }

        let http = reqwest::Client::builder()
            .timeout(config.request_timeout)
            .https_only(true)
            .min_tls_version(reqwest::tls::Version::TLS_1_2)
            // The bearer token is attached to every request, so a redirect to
            // another host would hand the credential to that host.
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|error| DevinClientError::Transport(error.to_string()))?;

        Ok(Self {
            http,
            base_url: config.base_url.clone(),
            org_id,
            token: DevinApiToken::new(token.trim()),
            poll_interval: config.poll_interval,
            tenancy: None,
        })
    }

    pub fn org_id(&self) -> Option<&str> {
        self.org_id.as_deref()
    }

    pub fn poll_interval(&self) -> Duration {
        self.poll_interval
    }

    /// Request builder bound to the resolved organization.
    pub fn requests(&self) -> Result<DevinRequestBuilder, DevinClientError> {
        let org_id = self
            .org_id
            .as_deref()
            .ok_or(DevinClientError::MissingOrgId)?;
        Ok(DevinRequestBuilder::new(self.base_url.clone(), org_id))
    }

    /// Verified tenancy, once [`Self::bind_tenant`] has run.
    pub fn tenancy(&self) -> Option<&DevinTenancy> {
        self.tenancy.as_ref()
    }

    /// Binds the client to exactly one organization.
    ///
    /// The credential's own organization is read from `GET /v3/self`. When an
    /// organization was configured, the two must agree: a credential minted for
    /// another tenant is refused rather than silently driving sessions there.
    /// When none was configured, the credential's organization is adopted.
    pub async fn bind_tenant(mut self) -> Result<Self, DevinClientError> {
        let identity = self.identity().await?;
        let tenancy = reconcile_tenant(self.org_id.as_deref(), identity)?;
        self.org_id = Some(tenancy.org_id.clone());
        self.tenancy = Some(tenancy);

        Ok(self)
    }

    /// Resolves the organization from the credential when it was not
    /// configured, so operators only need to supply the service user key.
    pub async fn resolve_org_id(self) -> Result<Self, DevinClientError> {
        self.bind_tenant().await
    }

    /// Rejects a session payload owned by a different organization.
    fn ensure_tenant(&self, session: &SessionResponse) -> Result<(), DevinClientError> {
        let expected = self
            .org_id
            .as_deref()
            .ok_or(DevinClientError::MissingOrgId)?;
        ensure_session_tenant(expected, session)
    }

    pub async fn identity(&self) -> Result<DevinSelfResponse, DevinClientError> {
        self.send_typed(&DevinRequestBuilder::new(self.base_url.clone(), "").get_self())
            .await
    }

    pub async fn send(&self, request: &DevinRequest) -> Result<Value, DevinClientError> {
        let url = DevinRequestBuilder::new(
            self.base_url.clone(),
            self.org_id.clone().unwrap_or_default(),
        )
        .absolute_url(request)?;
        let mut builder = match request.method {
            DevinHttpMethod::Get => self.http.get(url),
            DevinHttpMethod::Post => self.http.post(url),
            DevinHttpMethod::Put => self.http.put(url),
            DevinHttpMethod::Delete => self.http.delete(url),
        }
        .bearer_auth(self.token.expose());
        if let Some(body) = request.body.as_ref() {
            builder = builder.json(body);
        }

        let response = builder
            .send()
            .await
            .map_err(|error| DevinClientError::Transport(error.to_string()))?;
        let status = response.status();
        let body = response
            .text()
            .await
            .map_err(|error| DevinClientError::Transport(error.to_string()))?;
        if !status.is_success() {
            return Err(DevinClientError::Status {
                status: status.as_u16(),
                problem: describe_problem(&body),
            });
        }
        if body.trim().is_empty() {
            return Ok(Value::Null);
        }

        serde_json::from_str(&body).map_err(|error| DevinClientError::Decode(error.to_string()))
    }

    async fn send_typed<T: serde::de::DeserializeOwned>(
        &self,
        request: &DevinRequest,
    ) -> Result<T, DevinClientError> {
        let value = self.send(request).await?;
        serde_json::from_value(value).map_err(|error| DevinClientError::Decode(error.to_string()))
    }

    pub async fn create_session(
        &self,
        request: &SessionCreateRequest,
    ) -> Result<SessionResponse, DevinClientError> {
        let session: SessionResponse = self
            .send_typed(&self.requests()?.create_session(request))
            .await?;
        self.ensure_tenant(&session)?;
        Ok(session)
    }

    pub async fn get_session(&self, devin_id: &str) -> Result<SessionResponse, DevinClientError> {
        let session: SessionResponse = self
            .send_typed(&self.requests()?.get_session(devin_id))
            .await?;
        self.ensure_tenant(&session)?;
        Ok(session)
    }

    pub async fn list_sessions(
        &self,
        query: &SessionsQueryParams,
    ) -> Result<PaginatedResponse<SessionResponse>, DevinClientError> {
        let page: PaginatedResponse<SessionResponse> = self
            .send_typed(&self.requests()?.list_sessions(query))
            .await?;
        for session in &page.items {
            self.ensure_tenant(session)?;
        }
        Ok(page)
    }

    /// Fetches one page of session messages. `after` is the `end_cursor` of the
    /// previous page.
    pub async fn list_messages(
        &self,
        devin_id: &str,
        after: Option<&str>,
        first: Option<u32>,
    ) -> Result<PaginatedResponse<SessionMessage>, DevinClientError> {
        self.send_typed(&self.requests()?.list_messages(devin_id, after, first))
            .await
    }

    pub async fn send_message(
        &self,
        devin_id: &str,
        message: impl Into<String>,
    ) -> Result<(), DevinClientError> {
        self.send(&self.requests()?.send_message(devin_id, message))
            .await
            .map(|_| ())
    }

    /// Stops a session, optionally archiving it.
    pub async fn delete_session(
        &self,
        devin_id: &str,
        archive: bool,
    ) -> Result<(), DevinClientError> {
        self.send(&self.requests()?.delete_session(devin_id, archive))
            .await
            .map(|_| ())
    }

    pub async fn list_attachments(
        &self,
        devin_id: &str,
    ) -> Result<Vec<SessionAttachment>, DevinClientError> {
        self.send_typed(&self.requests()?.list_attachments(devin_id))
            .await
    }

    /// Every secret the bound organization exposes, walked to the last page.
    pub async fn list_secrets(&self) -> Result<Vec<SecretResponse>, DevinClientError> {
        let requests = self.requests()?;
        let mut secrets = Vec::new();
        let mut after: Option<String> = None;
        loop {
            let page: PaginatedResponse<SecretResponse> = self
                .send_typed(&requests.list_secrets(after.as_deref(), Some(DEVIN_MAX_PAGE_SIZE)))
                .await?;
            let advanced = page
                .end_cursor
                .clone()
                .filter(|cursor| Some(cursor) != after.as_ref());
            secrets.extend(page.items);
            match (page.has_next_page, advanced) {
                (true, Some(cursor)) => after = Some(cursor),
                // A page that cannot advance its cursor would loop forever.
                _ => return Ok(secrets),
            }
        }
    }

    /// Resolves configured secret references to organization secret ids.
    ///
    /// A reference may be a secret id or a secret key, which keeps workflow
    /// files readable. Anything the bound organization does not own is
    /// rejected, so a workflow cannot inject another tenant's secret into a
    /// session, and secret values never enter OpenSymphony.
    pub async fn resolve_secret_ids(
        &self,
        references: &[String],
    ) -> Result<Vec<String>, DevinClientError> {
        if references.is_empty() {
            return Ok(Vec::new());
        }
        let org_id = self.org_id.clone().ok_or(DevinClientError::MissingOrgId)?;
        let available = self.list_secrets().await?;
        resolve_secret_references(&org_id, &available, references)
    }

    /// Downloads one session attachment for local evidence, bounded by `limit`.
    ///
    /// Attachments are only fetched from the API origin the client is already
    /// authenticated against: another origin would either receive the bearer
    /// token or contribute unauthenticated bytes to run evidence.
    pub async fn download_attachment(
        &self,
        attachment: &SessionAttachment,
        limit: u64,
    ) -> Result<Vec<u8>, DevinClientError> {
        let url = attachment_download_url(&self.base_url, attachment)?;

        let response = self
            .http
            .get(url)
            .bearer_auth(self.token.expose())
            .send()
            .await
            .map_err(|error| DevinClientError::Transport(error.to_string()))?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(DevinClientError::Status {
                status: status.as_u16(),
                problem: describe_problem(&body),
            });
        }
        if response
            .content_length()
            .is_some_and(|length| length > limit)
        {
            return Err(DevinClientError::AttachmentTooLarge {
                name: attachment.name.clone(),
                limit,
            });
        }
        // `Content-Length` is advisory and absent for chunked responses, so the
        // body is read incrementally and abandoned as soon as it exceeds the
        // limit rather than being buffered in full first.
        let mut response = response;
        let mut body = BoundedBody::new(response.content_length(), limit);
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|error| DevinClientError::Transport(error.to_string()))?
        {
            body.push(&chunk)
                .map_err(|()| DevinClientError::AttachmentTooLarge {
                    name: attachment.name.clone(),
                    limit,
                })?;
        }
        let body = body.into_inner();

        Ok(body)
    }
}

/// Accumulates a response body without ever holding more than `limit` bytes.
///
/// A chunked attachment reports no `Content-Length`, so the ceiling can only be
/// enforced while the body streams in.
struct BoundedBody {
    buffer: Vec<u8>,
    limit: u64,
}

impl BoundedBody {
    fn new(content_length: Option<u64>, limit: u64) -> Self {
        Self {
            buffer: Vec::with_capacity(
                usize::try_from(content_length.unwrap_or(0).min(limit)).unwrap_or(0),
            ),
            limit,
        }
    }

    fn push(&mut self, chunk: &[u8]) -> Result<(), ()> {
        if self.buffer.len() as u64 + chunk.len() as u64 > self.limit {
            return Err(());
        }
        self.buffer.extend_from_slice(chunk);
        Ok(())
    }

    fn into_inner(self) -> Vec<u8> {
        self.buffer
    }
}

/// Binds a credential identity to exactly one organization.
///
/// A configured organization must match the credential's own organization: a
/// credential minted for another tenant is refused rather than silently driving
/// sessions there. When none was configured, the credential's organization is
/// adopted.
pub fn reconcile_tenant(
    configured: Option<&str>,
    identity: DevinSelfResponse,
) -> Result<DevinTenancy, DevinClientError> {
    let credential_org = identity.org_id.ok_or(DevinClientError::MissingOrgId)?;
    if let Some(configured) = configured
        && configured != credential_org
    {
        return Err(DevinClientError::TenantMismatch {
            configured: configured.to_owned(),
            credential: credential_org,
        });
    }

    Ok(DevinTenancy {
        org_id: credential_org,
        principal_type: identity.principal_type,
        service_user_id: identity.service_user_id,
        service_user_name: identity.service_user_name,
    })
}

/// Rejects a session payload owned by a different organization.
pub fn ensure_session_tenant(
    expected: &str,
    session: &SessionResponse,
) -> Result<(), DevinClientError> {
    if session.org_id != expected {
        return Err(DevinClientError::CrossTenantPayload {
            expected: expected.to_owned(),
            observed: session.org_id.clone(),
        });
    }

    Ok(())
}

/// Maps secret references onto ids owned by the bound organization.
///
/// A reference may be a secret id or a secret key, which keeps workflow files
/// readable. Anything the organization does not own is rejected, so a workflow
/// cannot inject another tenant's secret into a session.
pub fn resolve_secret_references(
    org_id: &str,
    available: &[SecretResponse],
    references: &[String],
) -> Result<Vec<String>, DevinClientError> {
    let mut resolved: Vec<String> = Vec::with_capacity(references.len());
    for reference in references {
        let reference = reference.trim();
        let matched = available
            .iter()
            .find(|secret| secret.secret_id == reference)
            .or_else(|| {
                available
                    .iter()
                    .find(|secret| secret.key.as_deref() == Some(reference))
            })
            .ok_or_else(|| DevinClientError::UnknownSecret {
                reference: reference.to_owned(),
                org_id: org_id.to_owned(),
            })?;
        if !resolved.contains(&matched.secret_id) {
            resolved.push(matched.secret_id.clone());
        }
    }

    Ok(resolved)
}

/// Validates that an attachment lives on the authenticated API origin.
///
/// Another origin would either receive the bearer token or contribute
/// unauthenticated bytes to run evidence.
pub fn attachment_download_url(
    base_url: &str,
    attachment: &SessionAttachment,
) -> Result<Url, DevinClientError> {
    let url = Url::parse(&attachment.url).map_err(|source| DevinClientError::InvalidUrl {
        base_url: base_url.to_owned(),
        path: attachment.attachment_id.clone(),
        source,
    })?;
    let base = Url::parse(base_url).map_err(|source| DevinClientError::InvalidUrl {
        base_url: base_url.to_owned(),
        path: String::new(),
        source,
    })?;
    // Ports are part of the origin: `https://api.devin.ai:8443` is a different
    // service from `https://api.devin.ai`, and must never receive the token.
    if url.scheme() != base.scheme()
        || url.host_str() != base.host_str()
        || url.port_or_known_default() != base.port_or_known_default()
    {
        return Err(DevinClientError::ForeignAttachmentOrigin {
            name: attachment.name.clone(),
            origin: url.origin().ascii_serialization(),
        });
    }

    Ok(url)
}

/// Renders an error body as a problem description, falling back to the raw body
/// when Devin returns something other than `application/problem+json`.
fn describe_problem(body: &str) -> String {
    serde_json::from_str::<DevinProblemDetail>(body)
        .map(|problem| problem.to_string())
        .unwrap_or_else(|_| body.trim().to_owned())
}

// ---------------------------------------------------------------------------
// Event normalization
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NormalizedDevinEventKind {
    SessionCreated,
    DevinMessage,
    UserMessage,
    StatusChanged,
    SessionFinished,
    SessionBlocked,
    SessionFailed,
    SessionSuspended,
    SessionTerminated,
    Unknown,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NormalizedDevinEvent {
    pub kind: NormalizedDevinEventKind,
    pub event_type: String,
    pub event_id: Option<String>,
    pub session_id: Option<String>,
    pub timestamp: Option<String>,
    pub message: Option<String>,
    pub status: Option<String>,
    pub raw: Value,
}

/// Normalizes one documented `SessionMessage`.
///
/// Unrecognized sources are retained as [`NormalizedDevinEventKind::Unknown`]
/// with the full raw JSON so future Devin schema additions replay unchanged.
pub fn normalize_session_message(
    session_id: &str,
    message: &SessionMessage,
) -> NormalizedDevinEvent {
    let kind = match message.source.as_str() {
        "devin" => NormalizedDevinEventKind::DevinMessage,
        "user" => NormalizedDevinEventKind::UserMessage,
        _ => NormalizedDevinEventKind::Unknown,
    };
    let raw = serde_json::to_value(message).unwrap_or_else(|_| json!({}));

    NormalizedDevinEvent {
        kind,
        event_type: format!("session.message.{}", message.source),
        event_id: Some(message.event_id.clone()),
        session_id: Some(session_id.to_owned()),
        timestamp: Some(message.created_at.to_string()),
        message: Some(message.message.clone()),
        status: None,
        raw,
    }
}

/// Normalizes a raw JSON payload that could not be decoded into a
/// [`SessionMessage`], preserving it verbatim.
pub fn normalize_devin_event(raw: Value) -> NormalizedDevinEvent {
    let source = first_string(&raw, &["source", "type", "event_type", "kind"]);
    let kind = match source.as_deref() {
        Some("devin") => NormalizedDevinEventKind::DevinMessage,
        Some("user") => NormalizedDevinEventKind::UserMessage,
        _ => NormalizedDevinEventKind::Unknown,
    };

    NormalizedDevinEvent {
        kind,
        event_type: source.unwrap_or_else(|| DEVIN_UNDISCRIMINATED_EVENT_KIND.to_owned()),
        event_id: first_string(&raw, &["event_id", "eventId"]),
        session_id: first_string(&raw, &["session_id", "sessionId", "devin_id"]),
        timestamp: first_string(&raw, &["created_at", "timestamp"]),
        message: first_string(&raw, &["message", "text", "content"]),
        status: first_string(&raw, &["status", "status_detail"]),
        raw,
    }
}

/// Synthesizes the status event Devin does not send: status is session state,
/// not a message, so transitions are derived while polling.
pub fn status_event(session: &SessionResponse) -> NormalizedDevinEvent {
    let detail = session.status_detail.as_ref();
    let kind = match &session.status {
        DevinSessionStatus::Error => NormalizedDevinEventKind::SessionFailed,
        DevinSessionStatus::Suspended => NormalizedDevinEventKind::SessionSuspended,
        DevinSessionStatus::Exit => NormalizedDevinEventKind::SessionFinished,
        _ if detail.is_some_and(DevinStatusDetail::is_finished) => {
            NormalizedDevinEventKind::SessionFinished
        }
        _ if detail.is_some_and(DevinStatusDetail::is_waiting_on_operator) => {
            NormalizedDevinEventKind::SessionBlocked
        }
        _ => NormalizedDevinEventKind::StatusChanged,
    };
    let status = match detail {
        Some(detail) => format!("{}:{}", session.status.as_str(), detail.as_str()),
        None => session.status.as_str().to_owned(),
    };

    NormalizedDevinEvent {
        kind,
        event_type: format!("session.status.{status}"),
        event_id: None,
        session_id: Some(session.session_id.clone()),
        timestamp: Some(session.updated_at.to_string()),
        message: None,
        status: Some(status),
        raw: json!({
            "session_id": session.session_id,
            "status": session.status.as_str(),
            "status_detail": detail.map(DevinStatusDetail::as_str),
        }),
    }
}

pub fn session_created_event(session: &SessionResponse) -> NormalizedDevinEvent {
    NormalizedDevinEvent {
        kind: NormalizedDevinEventKind::SessionCreated,
        event_type: "session.created".into(),
        event_id: None,
        session_id: Some(session.session_id.clone()),
        timestamp: Some(session.created_at.to_string()),
        message: Some(session.url.clone()),
        status: Some(session.status.as_str().to_owned()),
        raw: serde_json::to_value(session).unwrap_or_else(|_| json!({})),
    }
}

/// Tracks the message cursor and last observed status for one polled session.
///
/// Messages are paginated by `end_cursor`, but the cursor is only advanced
/// after the page is consumed and every `event_id` is also remembered, so a
/// replayed or overlapping page cannot emit an event twice.
#[derive(Debug, Clone, Default)]
pub struct DevinMessageCursor {
    seen: HashSet<String>,
    cursor: Option<String>,
    last_status: Option<String>,
}

impl DevinMessageCursor {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cursor(&self) -> Option<&str> {
        self.cursor.as_deref()
    }

    /// Consumes one message page, returning only messages not seen before.
    pub fn ingest_messages(
        &mut self,
        session_id: &str,
        page: &PaginatedResponse<SessionMessage>,
    ) -> Vec<NormalizedDevinEvent> {
        let mut fresh = Vec::new();
        for message in &page.items {
            if self.seen.insert(message.event_id.clone()) {
                fresh.push(normalize_session_message(session_id, message));
            }
        }
        if let Some(end_cursor) = page.end_cursor.as_ref() {
            self.cursor = Some(end_cursor.clone());
        }

        fresh
    }

    /// Emits a status event when the session's status or status detail changed.
    pub fn ingest_status(&mut self, session: &SessionResponse) -> Option<NormalizedDevinEvent> {
        let current = match session.status_detail.as_ref() {
            Some(detail) => format!("{}:{}", session.status.as_str(), detail.as_str()),
            None => session.status.as_str().to_owned(),
        };
        if self.last_status.as_deref() == Some(current.as_str()) {
            return None;
        }
        self.last_status = Some(current);

        Some(status_event(session))
    }

    pub fn last_status(&self) -> Option<&str> {
        self.last_status.as_deref()
    }
}

pub fn devin_event_summary(event: &NormalizedDevinEvent) -> String {
    match event.kind {
        NormalizedDevinEventKind::SessionCreated => match event.message.as_deref() {
            Some(url) => format!("Devin session created: {url}"),
            None => "Devin session created".into(),
        },
        NormalizedDevinEventKind::DevinMessage => event
            .message
            .clone()
            .unwrap_or_else(|| "Devin message".into()),
        NormalizedDevinEventKind::UserMessage => "Operator message forwarded to Devin".into(),
        NormalizedDevinEventKind::StatusChanged => match event.status.as_deref() {
            Some(status) => format!("Devin status changed to {status}"),
            None => "Devin status changed".into(),
        },
        NormalizedDevinEventKind::SessionFinished => "Devin session finished".into(),
        NormalizedDevinEventKind::SessionBlocked => {
            "Devin session is waiting on the operator".into()
        }
        NormalizedDevinEventKind::SessionFailed => "Devin session errored".into(),
        NormalizedDevinEventKind::SessionSuspended => "Devin session was suspended".into(),
        NormalizedDevinEventKind::SessionTerminated => "Devin session terminated".into(),
        NormalizedDevinEventKind::Unknown => {
            format!("Unknown Devin event `{}`", event.event_type)
        }
    }
}

pub fn devin_event_payload(event: &NormalizedDevinEvent) -> Value {
    json!({
        "source_kind": event.event_type,
        "session_id": event.session_id,
        "event_id": event.event_id,
        "timestamp": event.timestamp,
        "status": event.status,
        "message": event.message,
        "raw_payload": event.raw,
    })
}

pub fn normalized_event_to_journal_record(
    run_id: impl Into<String>,
    sequence: u64,
    event: &NormalizedDevinEvent,
) -> EventRecord {
    let run_id = run_id.into();
    let kind = match event.kind {
        NormalizedDevinEventKind::SessionCreated => EventKind::RunStarted,
        NormalizedDevinEventKind::SessionFinished => EventKind::RunCompleted,
        NormalizedDevinEventKind::SessionFailed => EventKind::RunFailed,
        NormalizedDevinEventKind::SessionTerminated => EventKind::RunCancelled,
        NormalizedDevinEventKind::Unknown => EventKind::Unknown {
            raw_kind: event.event_type.clone(),
        },
        _ => EventKind::HarnessEventNormalized {
            source_kind: event.event_type.clone(),
        },
    };

    EventRecord::builder()
        .sequence(sequence)
        .actor(EventActor::harness(DEVIN_CLOUD_AGENT_KIND))
        .entity_ref(EntityRef::run(run_id.clone()))
        .summary(devin_event_summary(event))
        .kind(kind)
        .payload(devin_event_payload(event))
        .raw_payload_ref(format!("devin:{run_id}:{sequence}"))
        .build()
}

fn first_string(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(Value::as_str))
        .map(str::to_owned)
}

// ---------------------------------------------------------------------------
// Session lifecycle
// ---------------------------------------------------------------------------

/// How a polled Devin session ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DevinRunOutcome {
    Finished,
    WaitingOnOperator,
    Suspended,
    Failed,
    Terminated,
}

impl DevinRunOutcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Finished => "finished",
            Self::WaitingOnOperator => "waiting_on_operator",
            Self::Suspended => "suspended",
            Self::Failed => "failed",
            Self::Terminated => "terminated",
        }
    }
}

/// Evidence collected from a Devin session, stored in the local issue workspace.
#[derive(Debug, Clone, PartialEq)]
pub struct DevinRunReport {
    pub session_id: String,
    pub session_url: String,
    pub outcome: DevinRunOutcome,
    pub status: DevinSessionStatus,
    pub status_detail: Option<DevinStatusDetail>,
    pub acus_consumed: f64,
    pub pull_requests: Vec<SessionPullRequest>,
    pub structured_output: Option<Value>,
    pub attachments: Vec<SessionAttachment>,
}

/// Options for [`DevinSessionRunner::run`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevinRunOptions {
    /// Stop polling once Devin asks for operator input instead of waiting.
    pub stop_when_waiting_on_operator: bool,
    /// Upper bound on total polling time.
    pub max_duration: Option<Duration>,
    /// Fetch the session attachment list once the session settles.
    pub collect_attachments: bool,
    /// Page size for message polling.
    pub message_page_size: u32,
}

impl Default for DevinRunOptions {
    fn default() -> Self {
        Self {
            stop_when_waiting_on_operator: true,
            max_duration: None,
            collect_attachments: true,
            message_page_size: 100,
        }
    }
}

/// Drives one Devin session: create, poll, normalize, settle.
///
/// The API exposes no event stream, so progress is polled from the session
/// resource and its cursor-paginated message collection at the configured
/// interval.
#[derive(Debug, Clone)]
pub struct DevinSessionRunner {
    client: DevinCloudClient,
    options: DevinRunOptions,
}

impl DevinSessionRunner {
    pub fn new(client: DevinCloudClient, options: DevinRunOptions) -> Self {
        Self { client, options }
    }

    pub fn client(&self) -> &DevinCloudClient {
        &self.client
    }

    /// Creates a session and polls it to a settled state, handing every fresh
    /// normalized event to `observer` as it is observed.
    pub async fn run(
        &self,
        request: &SessionCreateRequest,
        mut observer: impl FnMut(NormalizedDevinEvent),
    ) -> Result<DevinRunReport, DevinClientError> {
        let created = self.client.create_session(request).await?;
        observer(session_created_event(&created));
        self.follow(&created.session_id, observer).await
    }

    /// Polls an existing session to a settled state.
    pub async fn follow(
        &self,
        devin_id: &str,
        mut observer: impl FnMut(NormalizedDevinEvent),
    ) -> Result<DevinRunReport, DevinClientError> {
        let started = Instant::now();
        let mut cursor = DevinMessageCursor::new();

        loop {
            let session = self.client.get_session(devin_id).await?;
            self.drain_messages(devin_id, &mut cursor, &mut observer)
                .await?;
            if let Some(event) = cursor.ingest_status(&session) {
                observer(event);
            }

            if let Some(outcome) = self.settled_outcome(&session) {
                return self.report(session, outcome).await;
            }

            if let Some(max_duration) = self.options.max_duration
                && started.elapsed() >= max_duration
            {
                return Err(DevinClientError::PollTimeout {
                    session_id: devin_id.to_owned(),
                    elapsed: started.elapsed(),
                });
            }

            tokio::time::sleep(self.client.poll_interval()).await;
        }
    }

    /// Sends operator input to a running session.
    pub async fn send_message(
        &self,
        devin_id: &str,
        message: impl Into<String>,
    ) -> Result<(), DevinClientError> {
        self.client.send_message(devin_id, message).await
    }

    /// Stops a session and reports it as terminated.
    pub async fn terminate(
        &self,
        devin_id: &str,
        archive: bool,
    ) -> Result<DevinRunReport, DevinClientError> {
        self.client.delete_session(devin_id, archive).await?;
        let session = self.client.get_session(devin_id).await?;
        self.report(session, DevinRunOutcome::Terminated).await
    }

    fn settled_outcome(&self, session: &SessionResponse) -> Option<DevinRunOutcome> {
        match &session.status {
            DevinSessionStatus::Error => Some(DevinRunOutcome::Failed),
            DevinSessionStatus::Suspended => Some(DevinRunOutcome::Suspended),
            DevinSessionStatus::Exit => Some(DevinRunOutcome::Finished),
            _ if session
                .status_detail
                .as_ref()
                .is_some_and(DevinStatusDetail::is_finished) =>
            {
                Some(DevinRunOutcome::Finished)
            }
            _ if self.options.stop_when_waiting_on_operator && session.is_waiting_on_operator() => {
                Some(DevinRunOutcome::WaitingOnOperator)
            }
            _ => None,
        }
    }

    /// Reads every message page produced since the last poll.
    async fn drain_messages(
        &self,
        devin_id: &str,
        cursor: &mut DevinMessageCursor,
        observer: &mut impl FnMut(NormalizedDevinEvent),
    ) -> Result<(), DevinClientError> {
        loop {
            let page = self
                .client
                .list_messages(
                    devin_id,
                    cursor.cursor(),
                    Some(self.options.message_page_size),
                )
                .await?;
            let previous = cursor.cursor().map(str::to_owned);
            for event in cursor.ingest_messages(devin_id, &page) {
                observer(event);
            }
            // A page that reports more data but does not advance the cursor
            // would otherwise be requested forever.
            if !page.has_next_page || cursor.cursor().map(str::to_owned) == previous {
                return Ok(());
            }
        }
    }

    async fn report(
        &self,
        session: SessionResponse,
        outcome: DevinRunOutcome,
    ) -> Result<DevinRunReport, DevinClientError> {
        let attachments = if self.options.collect_attachments {
            self.client.list_attachments(&session.session_id).await?
        } else {
            Vec::new()
        };

        Ok(DevinRunReport {
            session_id: session.session_id,
            session_url: session.url,
            outcome,
            status: session.status,
            status_detail: session.status_detail,
            acus_consumed: session.acus_consumed,
            pull_requests: session.pull_requests,
            structured_output: session.structured_output,
            attachments,
        })
    }
}

// ---------------------------------------------------------------------------
// Remote evidence retrieval
// ---------------------------------------------------------------------------

/// Directory, relative to the issue workspace metadata directory, that holds
/// evidence imported from Devin-owned remote workspaces.
pub const DEVIN_EVIDENCE_DIR_NAME: &str = "devin";
/// Per-attachment ceiling for evidence downloads.
pub const DEVIN_MAX_ATTACHMENT_BYTES: u64 = 25 * 1024 * 1024;
/// Ceiling on how many attachments one session contributes to evidence.
pub const DEVIN_MAX_EVIDENCE_ATTACHMENTS: usize = 50;

/// Bounds applied while importing remote artifacts into the local workspace.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DevinEvidenceLimits {
    pub max_attachment_bytes: u64,
    pub max_attachments: usize,
    /// Download attachment bytes, not just their metadata.
    pub download_attachments: bool,
}

impl Default for DevinEvidenceLimits {
    fn default() -> Self {
        Self {
            max_attachment_bytes: DEVIN_MAX_ATTACHMENT_BYTES,
            max_attachments: DEVIN_MAX_EVIDENCE_ATTACHMENTS,
            download_attachments: true,
        }
    }
}

/// One attachment that was imported into the local evidence directory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DevinStoredAttachment {
    pub attachment_id: String,
    pub name: String,
    pub source: String,
    #[serde(default)]
    pub content_type: Option<String>,
    /// Path relative to the evidence root.
    pub stored_path: String,
    pub bytes: u64,
}

/// One attachment that was deliberately not imported, with the reason.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DevinSkippedAttachment {
    pub attachment_id: String,
    pub name: String,
    pub reason: String,
}

/// Index of everything written into the local evidence directory for a run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DevinEvidenceManifest {
    pub session_id: String,
    pub session_url: String,
    pub outcome: String,
    pub status: String,
    #[serde(default)]
    pub status_detail: Option<String>,
    pub acus_consumed: f64,
    pub pull_request_urls: Vec<String>,
    pub event_count: usize,
    pub attachments: Vec<DevinStoredAttachment>,
    pub skipped_attachments: Vec<DevinSkippedAttachment>,
    /// Absolute path of the evidence root inside the local issue workspace.
    pub evidence_root: PathBuf,
    pub containment: String,
}

/// Collects normalized events during a run and imports remote artifacts into
/// the local issue workspace when the session settles.
///
/// The evidence root is orchestrator-owned storage: Devin never executes there,
/// and nothing Devin returns is allowed to escape it (attachment names are
/// sanitized and the resolved path is re-checked against the root).
#[derive(Debug, Clone)]
pub struct DevinEvidenceCollector {
    root: PathBuf,
    events: Vec<Value>,
    limits: DevinEvidenceLimits,
}

impl DevinEvidenceCollector {
    pub fn new(root: impl Into<PathBuf>, limits: DevinEvidenceLimits) -> Self {
        Self {
            root: root.into(),
            events: Vec::new(),
            limits,
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn event_count(&self) -> usize {
        self.events.len()
    }

    /// Records one normalized event for the journal written at settle time.
    pub fn record(&mut self, event: &NormalizedDevinEvent) {
        self.events.push(devin_event_payload(event));
    }

    /// Writes the session summary, event journal, and attachments, returning
    /// the manifest that was also written to disk.
    pub async fn persist(
        &self,
        client: &DevinCloudClient,
        report: &DevinRunReport,
    ) -> Result<DevinEvidenceManifest, DevinClientError> {
        create_dir(&self.root).await?;

        let summary = json!({
            "session_id": report.session_id,
            "session_url": report.session_url,
            "outcome": report.outcome.as_str(),
            "status": report.status.as_str(),
            "status_detail": report.status_detail.as_ref().map(DevinStatusDetail::as_str),
            "acus_consumed": report.acus_consumed,
            "pull_requests": report.pull_requests,
            "structured_output": report.structured_output,
            "attachments": report.attachments,
            "containment": DEVIN_REMOTE_CONTAINMENT,
        });
        write_file(&self.root.join("session.json"), pretty(&summary).as_bytes()).await?;

        let mut journal = String::new();
        for event in &self.events {
            journal.push_str(&serde_json::to_string(event).unwrap_or_else(|_| "{}".to_owned()));
            journal.push('\n');
        }
        write_file(&self.root.join("events.jsonl"), journal.as_bytes()).await?;

        let (attachments, skipped) = self.import_attachments(client, report).await?;

        let manifest = DevinEvidenceManifest {
            session_id: report.session_id.clone(),
            session_url: report.session_url.clone(),
            outcome: report.outcome.as_str().to_owned(),
            status: report.status.as_str().to_owned(),
            status_detail: report
                .status_detail
                .as_ref()
                .map(|detail| detail.as_str().to_owned()),
            acus_consumed: report.acus_consumed,
            pull_request_urls: report
                .pull_requests
                .iter()
                .map(|pull_request| pull_request.pr_url.clone())
                .collect(),
            event_count: self.events.len(),
            attachments,
            skipped_attachments: skipped,
            evidence_root: self.root.clone(),
            containment: DEVIN_REMOTE_CONTAINMENT.to_owned(),
        };
        let rendered = serde_json::to_value(&manifest).unwrap_or_else(|_| json!({}));
        write_file(
            &self.root.join("evidence.json"),
            pretty(&rendered).as_bytes(),
        )
        .await?;

        Ok(manifest)
    }

    /// Downloads attachments into `attachments/`. A single failed download is
    /// recorded as skipped rather than failing the run: evidence import must
    /// not turn a completed Devin session into a failed OpenSymphony run.
    async fn import_attachments(
        &self,
        client: &DevinCloudClient,
        report: &DevinRunReport,
    ) -> Result<(Vec<DevinStoredAttachment>, Vec<DevinSkippedAttachment>), DevinClientError> {
        let mut stored = Vec::new();
        let mut skipped = Vec::new();
        if report.attachments.is_empty() {
            return Ok((stored, skipped));
        }

        let directory = self.root.join("attachments");
        if self.limits.download_attachments {
            create_dir(&directory).await?;
        }

        for (index, attachment) in report.attachments.iter().enumerate() {
            if !self.limits.download_attachments {
                skipped.push(DevinSkippedAttachment {
                    attachment_id: attachment.attachment_id.clone(),
                    name: attachment.name.clone(),
                    reason: "attachment download is disabled for this run".to_owned(),
                });
                continue;
            }
            if index >= self.limits.max_attachments {
                skipped.push(DevinSkippedAttachment {
                    attachment_id: attachment.attachment_id.clone(),
                    name: attachment.name.clone(),
                    reason: format!(
                        "exceeds the {} attachment evidence limit",
                        self.limits.max_attachments
                    ),
                });
                continue;
            }

            let file_name = evidence_file_name(index, attachment);
            let path = directory.join(&file_name);
            // Sanitization already removes separators; this rejects anything a
            // future change could let through.
            if path.parent() != Some(directory.as_path()) {
                skipped.push(DevinSkippedAttachment {
                    attachment_id: attachment.attachment_id.clone(),
                    name: attachment.name.clone(),
                    reason: "resolved outside the evidence directory".to_owned(),
                });
                continue;
            }

            match client
                .download_attachment(attachment, self.limits.max_attachment_bytes)
                .await
            {
                Ok(bytes) => {
                    write_file(&path, &bytes).await?;
                    stored.push(DevinStoredAttachment {
                        attachment_id: attachment.attachment_id.clone(),
                        name: attachment.name.clone(),
                        source: attachment.source.clone(),
                        content_type: attachment.content_type.clone(),
                        stored_path: format!("attachments/{file_name}"),
                        bytes: bytes.len() as u64,
                    });
                }
                Err(error) => skipped.push(DevinSkippedAttachment {
                    attachment_id: attachment.attachment_id.clone(),
                    name: attachment.name.clone(),
                    reason: error.to_string(),
                }),
            }
        }

        Ok((stored, skipped))
    }
}

/// Local file name for a remote attachment.
///
/// Remote names are untrusted: only `[A-Za-z0-9._-]` survives, consecutive dots
/// are collapsed so no `..` segment can appear, the name is length-bounded, and
/// the ordinal prefix keeps duplicates distinct.
pub fn evidence_file_name(index: usize, attachment: &SessionAttachment) -> String {
    let mut previous_dot = false;
    let mut sanitized: String = attachment
        .name
        .chars()
        .map(|character| {
            let mapped =
                if character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-') {
                    character
                } else {
                    '_'
                };
            let mapped = if mapped == '.' && previous_dot {
                '_'
            } else {
                mapped
            };
            previous_dot = mapped == '.';
            mapped
        })
        .collect();
    sanitized = sanitized.trim_matches('.').to_owned();
    sanitized.truncate(96);
    if sanitized.is_empty() {
        sanitized = "attachment".to_owned();
    }

    format!("{index:03}-{sanitized}")
}

fn pretty(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| "{}".to_owned())
}

async fn create_dir(path: &Path) -> Result<(), DevinClientError> {
    tokio::fs::create_dir_all(path)
        .await
        .map_err(|source| DevinClientError::Evidence {
            path: path.to_path_buf(),
            source,
        })
}

async fn write_file(path: &Path, contents: &[u8]) -> Result<(), DevinClientError> {
    tokio::fs::write(path, contents)
        .await
        .map_err(|source| DevinClientError::Evidence {
            path: path.to_path_buf(),
            source,
        })
}

#[cfg(test)]
mod tests {
    use super::BoundedBody;

    #[test]
    fn bounded_body_stops_before_buffering_past_the_limit() {
        let mut body = BoundedBody::new(None, 8);
        assert!(body.push(&[0u8; 5]).is_ok());
        assert!(body.push(&[0u8; 5]).is_err());
        assert_eq!(body.into_inner().len(), 5);
    }

    #[test]
    fn bounded_body_preserves_a_body_within_the_limit() {
        let mut body = BoundedBody::new(Some(6), 8);
        assert!(body.push(b"abc").is_ok());
        assert!(body.push(b"def").is_ok());
        assert_eq!(body.into_inner(), b"abcdef".to_vec());
    }

    #[test]
    fn bounded_body_never_preallocates_past_the_limit() {
        let body = BoundedBody::new(Some(u64::MAX), 16);
        assert!(body.into_inner().capacity() <= 16);
    }
}

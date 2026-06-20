use serde::{Deserialize, Serialize};

use super::version::SchemaVersion;

/// Public model and credential settings shape exposed for client rendering.
///
/// References intentionally point at secret storage locations or environment
/// variables. They never carry API key values, OAuth refresh tokens, or access
/// tokens.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelSettingsResponse {
    pub schema_version: SchemaVersion,
    pub profiles: Vec<ModelSettingsProfile>,
    pub credential_statuses: Vec<CredentialStatus>,
    pub supported_credential_statuses: Vec<CredentialStatusKind>,
    pub codex_local_readiness: CodexLocalReadiness,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CredentialStatusResponse {
    pub schema_version: SchemaVersion,
    pub statuses: Vec<CredentialStatus>,
    pub supported_statuses: Vec<CredentialStatusKind>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelSettingsProfile {
    pub id: String,
    pub display_name: String,
    pub owner_scope: OwnerScope,
    pub provider: CredentialProvider,
    pub credential_mode: CredentialMode,
    pub storage_mode: CredentialStorageMode,
    pub model: ConfiguredValueReference,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_url: Option<ConfiguredValueReference>,
    pub credential_reference: CredentialReference,
    pub compatible_harnesses: Vec<String>,
    pub status: CredentialStatusKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfiguredValueReference {
    pub source: ConfiguredValueSource,
    pub reference: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CredentialReference {
    pub id: String,
    pub kind: CredentialReferenceKind,
    pub provider: CredentialProvider,
    pub storage_mode: CredentialStorageMode,
    pub reference: String,
    pub redacted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CredentialStatus {
    pub credential_reference_id: String,
    pub provider: CredentialProvider,
    pub status: CredentialStatusKind,
    pub checked_by: String,
    pub detail: String,
}

/// Local Codex CLI readiness summary for client and operator rendering.
///
/// This shape intentionally records only command/status metadata. It must not
/// contain OAuth access tokens, refresh material, or parsed private Codex
/// credential files.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodexLocalReadiness {
    pub command: String,
    pub version: Option<String>,
    pub cli_status: CredentialStatusKind,
    pub app_server_status: CredentialStatusKind,
    pub login_status: CredentialStatusKind,
    pub subscription_status: CredentialStatusKind,
    pub checked_by: String,
    pub detail: String,
    pub login_command: String,
    pub status_command: String,
    pub logout_command: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodexCliProbe {
    pub command: String,
    pub version: ProbeCommandResult,
    pub app_server_help: ProbeCommandResult,
    pub login_status: ProbeCommandResult,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProbeCommandResult {
    Success { stdout: String, stderr: String },
    Failure { stdout: String, stderr: String },
    NotFound,
    PermissionDenied { detail: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OwnerScope {
    LocalUser,
    Workspace,
    Project,
    Organization,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CredentialProvider {
    OpenAiCompatibleApi,
    OpenAiChatGptCodex,
    HostedCredentialBroker,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CredentialMode {
    ApiKey,
    Subscription,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CredentialStorageMode {
    Environment,
    LocalKeychain,
    CodexCliHome,
    OpenHandsAuthDirectory,
    HostedBroker,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfiguredValueSource {
    EnvironmentVariable,
    Literal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CredentialReferenceKind {
    EnvironmentVariable,
    LocalKeychainServiceAccount,
    CodexCliLogin,
    OpenHandsAuthDirectory,
    HostedBrokerReference,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CredentialStatusKind {
    Installed,
    LoggedOut,
    Expired,
    Unsupported,
    PermissionDenied,
    Unknown,
}

impl ModelSettingsResponse {
    pub fn local_default(llm_api_key_installed: bool) -> Self {
        Self::local_with_codex_readiness(llm_api_key_installed, CodexLocalReadiness::not_checked())
    }

    pub fn local_with_codex_readiness(
        llm_api_key_installed: bool,
        codex_local_readiness: CodexLocalReadiness,
    ) -> Self {
        let api_key_status = if llm_api_key_installed {
            CredentialStatusKind::Installed
        } else {
            CredentialStatusKind::LoggedOut
        };
        let codex_status = codex_local_readiness.subscription_status;
        let profiles = vec![
            openhands_api_key_profile(api_key_status),
            codex_cli_login_profile(codex_status),
            openhands_auth_directory_profile(),
            hosted_broker_future_profile(),
        ];
        let credential_statuses = profiles
            .iter()
            .map(|profile| {
                let mut status = CredentialStatus::from_profile(profile);
                if profile.credential_reference.kind == CredentialReferenceKind::CodexCliLogin {
                    status.checked_by = codex_local_readiness.checked_by.clone();
                }
                status
            })
            .collect::<Vec<_>>();

        Self {
            schema_version: SchemaVersion::v1(),
            profiles,
            credential_statuses,
            supported_credential_statuses: supported_credential_statuses(),
            codex_local_readiness,
            notes: vec![
                "API-key profiles preserve OpenHands-compatible LLM_MODEL, LLM_API_KEY, and LLM_BASE_URL environment wiring.".into(),
                "Subscription profiles expose credential references only; raw OAuth and API-key material stays in the selected storage backend.".into(),
                "Codex subscription readiness is detected through supported Codex CLI commands, not by reading private credential files.".into(),
                "Hosted broker references are shape-compatible placeholders for the follow-up hosted secret-store implementation.".into(),
            ],
        }
    }
}

impl CredentialStatusResponse {
    pub fn from_model_settings(settings: &ModelSettingsResponse) -> Self {
        Self {
            schema_version: settings.schema_version.clone(),
            statuses: settings.credential_statuses.clone(),
            supported_statuses: settings.supported_credential_statuses.clone(),
        }
    }
}

impl CredentialStatus {
    pub fn from_profile(profile: &ModelSettingsProfile) -> Self {
        Self {
            credential_reference_id: profile.credential_reference.id.clone(),
            provider: profile.provider,
            status: profile.status,
            checked_by: "gateway_static_settings".into(),
            detail: status_detail(profile),
        }
    }
}

impl CodexLocalReadiness {
    pub fn not_checked() -> Self {
        Self {
            command: "codex".into(),
            version: None,
            cli_status: CredentialStatusKind::Unknown,
            app_server_status: CredentialStatusKind::Unknown,
            login_status: CredentialStatusKind::Unknown,
            subscription_status: CredentialStatusKind::Unknown,
            checked_by: "gateway_static_settings".into(),
            detail: "Codex CLI readiness has not been checked in this context.".into(),
            login_command: "codex login --device-auth".into(),
            status_command: "codex login status".into(),
            logout_command: "codex logout".into(),
        }
    }

    pub fn from_probe(probe: CodexCliProbe) -> Self {
        let version = probe
            .version
            .success_stdout()
            .and_then(first_non_empty_line);
        let cli_status = command_status(&probe.version);
        let app_server_status = command_status(&probe.app_server_help);
        let login_status = login_status_from_probe(&probe.login_status);
        let subscription_status = subscription_status(cli_status, app_server_status, login_status);
        let detail = codex_readiness_detail(
            &probe,
            cli_status,
            app_server_status,
            login_status,
            subscription_status,
        );

        Self {
            command: probe.command,
            version,
            cli_status,
            app_server_status,
            login_status,
            subscription_status,
            checked_by: "codex_cli_supported_commands".into(),
            detail,
            login_command: "codex login --device-auth".into(),
            status_command: "codex login status".into(),
            logout_command: "codex logout".into(),
        }
    }
}

impl ProbeCommandResult {
    pub fn success(stdout: impl Into<String>) -> Self {
        Self::Success {
            stdout: stdout.into(),
            stderr: String::new(),
        }
    }

    pub fn failure(stderr: impl Into<String>) -> Self {
        Self::Failure {
            stdout: String::new(),
            stderr: stderr.into(),
        }
    }

    fn success_stdout(&self) -> Option<&str> {
        match self {
            Self::Success { stdout, .. } => Some(stdout),
            Self::Failure { .. } | Self::NotFound | Self::PermissionDenied { .. } => None,
        }
    }
}

pub fn supported_credential_statuses() -> Vec<CredentialStatusKind> {
    vec![
        CredentialStatusKind::Installed,
        CredentialStatusKind::LoggedOut,
        CredentialStatusKind::Expired,
        CredentialStatusKind::Unsupported,
        CredentialStatusKind::PermissionDenied,
        CredentialStatusKind::Unknown,
    ]
}

fn openhands_api_key_profile(status: CredentialStatusKind) -> ModelSettingsProfile {
    ModelSettingsProfile {
        id: "openhands-env-api-key".into(),
        display_name: "OpenHands API-compatible environment profile".into(),
        owner_scope: OwnerScope::LocalUser,
        provider: CredentialProvider::OpenAiCompatibleApi,
        credential_mode: CredentialMode::ApiKey,
        storage_mode: CredentialStorageMode::Environment,
        model: env_ref("LLM_MODEL"),
        base_url: Some(env_ref("LLM_BASE_URL")),
        credential_reference: CredentialReference {
            id: "credential:env:LLM_API_KEY".into(),
            kind: CredentialReferenceKind::EnvironmentVariable,
            provider: CredentialProvider::OpenAiCompatibleApi,
            storage_mode: CredentialStorageMode::Environment,
            reference: "LLM_API_KEY".into(),
            redacted: true,
        },
        compatible_harnesses: vec!["openhands_agent_server".into()],
        status,
    }
}

fn codex_cli_login_profile(status: CredentialStatusKind) -> ModelSettingsProfile {
    ModelSettingsProfile {
        id: "codex-chatgpt-local-keychain".into(),
        display_name: "Codex ChatGPT CLI login reference".into(),
        owner_scope: OwnerScope::LocalUser,
        provider: CredentialProvider::OpenAiChatGptCodex,
        credential_mode: CredentialMode::Subscription,
        storage_mode: CredentialStorageMode::CodexCliHome,
        model: literal_ref("openai/chatgpt-codex-subscription"),
        base_url: None,
        credential_reference: CredentialReference {
            id: "credential:codex-cli:chatgpt-login".into(),
            kind: CredentialReferenceKind::CodexCliLogin,
            provider: CredentialProvider::OpenAiChatGptCodex,
            storage_mode: CredentialStorageMode::CodexCliHome,
            reference: "codex-cli:chatgpt-login".into(),
            redacted: true,
        },
        compatible_harnesses: vec!["codex_app_server".into()],
        status,
    }
}

fn openhands_auth_directory_profile() -> ModelSettingsProfile {
    ModelSettingsProfile {
        id: "openhands-chatgpt-auth-directory".into(),
        display_name: "OpenHands ChatGPT subscription auth-directory reference".into(),
        owner_scope: OwnerScope::LocalUser,
        provider: CredentialProvider::OpenAiChatGptCodex,
        credential_mode: CredentialMode::Subscription,
        storage_mode: CredentialStorageMode::OpenHandsAuthDirectory,
        model: literal_ref("openai/chatgpt-codex-subscription"),
        base_url: None,
        credential_reference: CredentialReference {
            id: "credential:openhands-auth-dir:openai".into(),
            kind: CredentialReferenceKind::OpenHandsAuthDirectory,
            provider: CredentialProvider::OpenAiChatGptCodex,
            storage_mode: CredentialStorageMode::OpenHandsAuthDirectory,
            reference: "OPENHANDS_AUTH_DIR/openai".into(),
            redacted: true,
        },
        compatible_harnesses: vec!["openhands_agent_server".into()],
        status: CredentialStatusKind::Unsupported,
    }
}

fn hosted_broker_future_profile() -> ModelSettingsProfile {
    ModelSettingsProfile {
        id: "hosted-openai-subscription-broker".into(),
        display_name: "Hosted OpenAI subscription broker reference".into(),
        owner_scope: OwnerScope::Organization,
        provider: CredentialProvider::HostedCredentialBroker,
        credential_mode: CredentialMode::Subscription,
        storage_mode: CredentialStorageMode::HostedBroker,
        model: literal_ref("openai/chatgpt-codex-subscription"),
        base_url: None,
        credential_reference: CredentialReference {
            id: "credential:hosted-broker:openai-chatgpt-codex".into(),
            kind: CredentialReferenceKind::HostedBrokerReference,
            provider: CredentialProvider::HostedCredentialBroker,
            storage_mode: CredentialStorageMode::HostedBroker,
            reference: "hosted://credentials/openai-chatgpt-codex".into(),
            redacted: true,
        },
        compatible_harnesses: vec!["codex_app_server".into(), "openhands_agent_server".into()],
        status: CredentialStatusKind::Unsupported,
    }
}

fn env_ref(reference: impl Into<String>) -> ConfiguredValueReference {
    ConfiguredValueReference {
        source: ConfiguredValueSource::EnvironmentVariable,
        reference: reference.into(),
    }
}

fn literal_ref(reference: impl Into<String>) -> ConfiguredValueReference {
    ConfiguredValueReference {
        source: ConfiguredValueSource::Literal,
        reference: reference.into(),
    }
}

fn status_detail(profile: &ModelSettingsProfile) -> String {
    match profile.status {
        CredentialStatusKind::Installed => "Credential reference is available.".into(),
        CredentialStatusKind::LoggedOut => "Credential reference exists, but no login or secret material is installed.".into(),
        CredentialStatusKind::Expired => "Credential reference exists, but stored credentials need refresh.".into(),
        CredentialStatusKind::Unsupported => {
            "Credential reference shape is advertised for compatibility; runtime adapter support is not implemented in this milestone.".into()
        }
        CredentialStatusKind::PermissionDenied => {
            "Credential reference exists, but this process cannot read the backing storage.".into()
        }
        CredentialStatusKind::Unknown => "Credential reference has not been checked yet.".into(),
    }
}

fn command_status(result: &ProbeCommandResult) -> CredentialStatusKind {
    match result {
        ProbeCommandResult::Success { .. } => CredentialStatusKind::Installed,
        ProbeCommandResult::NotFound => CredentialStatusKind::Unsupported,
        ProbeCommandResult::PermissionDenied { .. } => CredentialStatusKind::PermissionDenied,
        ProbeCommandResult::Failure { stdout, stderr } => {
            let text = format!("{stdout}\n{stderr}").to_ascii_lowercase();
            if text.contains("permission denied") || text.contains("access denied") {
                CredentialStatusKind::PermissionDenied
            } else if text.contains("unsupported") || text.contains("unknown command") {
                CredentialStatusKind::Unsupported
            } else {
                CredentialStatusKind::Unknown
            }
        }
    }
}

fn login_status_from_probe(result: &ProbeCommandResult) -> CredentialStatusKind {
    match result {
        ProbeCommandResult::Success { stdout, stderr } => {
            // Recognized English status strings were verified with
            // `codex-cli 0.138.0`; update these fixtures when bumping Codex.
            let text = format!("{stdout}\n{stderr}").to_ascii_lowercase();
            if text.contains("logged in using chatgpt") || text.contains("logged in with chatgpt") {
                CredentialStatusKind::Installed
            } else if text.contains("logged out")
                || text.contains("not logged in")
                || text.contains("login required")
            {
                CredentialStatusKind::LoggedOut
            } else if text.contains("expired") {
                CredentialStatusKind::Expired
            } else if text.contains("permission denied") || text.contains("access denied") {
                CredentialStatusKind::PermissionDenied
            } else {
                CredentialStatusKind::Unknown
            }
        }
        ProbeCommandResult::Failure { stdout, stderr } => {
            let text = format!("{stdout}\n{stderr}").to_ascii_lowercase();
            if text.contains("logged out")
                || text.contains("not logged in")
                || text.contains("login required")
            {
                CredentialStatusKind::LoggedOut
            } else if text.contains("expired") {
                CredentialStatusKind::Expired
            } else {
                command_status(result)
            }
        }
        ProbeCommandResult::NotFound | ProbeCommandResult::PermissionDenied { .. } => {
            command_status(result)
        }
    }
}

fn subscription_status(
    cli_status: CredentialStatusKind,
    app_server_status: CredentialStatusKind,
    login_status: CredentialStatusKind,
) -> CredentialStatusKind {
    if cli_status == CredentialStatusKind::PermissionDenied
        || app_server_status == CredentialStatusKind::PermissionDenied
        || login_status == CredentialStatusKind::PermissionDenied
    {
        return CredentialStatusKind::PermissionDenied;
    }
    if cli_status == CredentialStatusKind::Unsupported
        || app_server_status == CredentialStatusKind::Unsupported
    {
        return CredentialStatusKind::Unsupported;
    }
    login_status
}

fn codex_readiness_detail(
    probe: &CodexCliProbe,
    cli_status: CredentialStatusKind,
    app_server_status: CredentialStatusKind,
    login_status: CredentialStatusKind,
    subscription_status: CredentialStatusKind,
) -> String {
    match subscription_status {
        CredentialStatusKind::Installed => {
            "Codex CLI, app-server surface, and ChatGPT login are ready.".into()
        }
        CredentialStatusKind::LoggedOut => {
            "Codex CLI is available, but ChatGPT login is not active. Run `codex login --device-auth` and then `codex login status`.".into()
        }
        CredentialStatusKind::Expired => {
            "Codex ChatGPT login appears expired. Re-run `codex login --device-auth`.".into()
        }
        CredentialStatusKind::Unsupported => {
            if cli_status == CredentialStatusKind::Unsupported {
                format!("Codex command `{}` is not installed or not on PATH.", probe.command)
            } else if app_server_status == CredentialStatusKind::Unsupported {
                "Installed Codex CLI does not expose `codex app-server --help`.".into()
            } else {
                "Codex ChatGPT subscription login is not supported by this CLI output.".into()
            }
        }
        CredentialStatusKind::PermissionDenied => {
            "Codex readiness check was blocked by local permission or policy denial.".into()
        }
        CredentialStatusKind::Unknown => {
            if login_status == CredentialStatusKind::Unknown {
                "Codex CLI was detected, but `codex login status` did not report a recognized ChatGPT login state.".into()
            } else {
                "Codex readiness could not be classified from supported command output.".into()
            }
        }
    }
}

fn first_non_empty_line(text: &str) -> Option<String> {
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(str::to_owned)
}

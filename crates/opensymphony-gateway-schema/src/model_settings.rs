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
        let api_key_status = if llm_api_key_installed {
            CredentialStatusKind::Installed
        } else {
            CredentialStatusKind::LoggedOut
        };
        let profiles = vec![
            openhands_api_key_profile(api_key_status),
            codex_subscription_keychain_profile(),
            openhands_auth_directory_profile(),
            hosted_broker_future_profile(),
        ];
        let credential_statuses = profiles
            .iter()
            .map(CredentialStatus::from_profile)
            .collect::<Vec<_>>();

        Self {
            schema_version: SchemaVersion::v1(),
            profiles,
            credential_statuses,
            supported_credential_statuses: supported_credential_statuses(),
            notes: vec![
                "API-key profiles preserve OpenHands-compatible LLM_MODEL, LLM_API_KEY, and LLM_BASE_URL environment wiring.".into(),
                "Subscription profiles expose credential references only; raw OAuth and API-key material stays in the selected storage backend.".into(),
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

fn codex_subscription_keychain_profile() -> ModelSettingsProfile {
    ModelSettingsProfile {
        id: "codex-chatgpt-local-keychain".into(),
        display_name: "Codex ChatGPT subscription keychain reference".into(),
        owner_scope: OwnerScope::LocalUser,
        provider: CredentialProvider::OpenAiChatGptCodex,
        credential_mode: CredentialMode::Subscription,
        storage_mode: CredentialStorageMode::LocalKeychain,
        model: literal_ref("openai/chatgpt-codex-subscription"),
        base_url: None,
        credential_reference: CredentialReference {
            id: "credential:keychain:openai-chatgpt-codex".into(),
            kind: CredentialReferenceKind::LocalKeychainServiceAccount,
            provider: CredentialProvider::OpenAiChatGptCodex,
            storage_mode: CredentialStorageMode::LocalKeychain,
            reference: "service=opensymphony,account=openai-chatgpt-codex".into(),
            redacted: true,
        },
        compatible_harnesses: vec!["codex_app_server".into()],
        status: CredentialStatusKind::LoggedOut,
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

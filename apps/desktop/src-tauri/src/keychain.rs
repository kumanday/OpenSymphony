//! Keychain integration for sensitive credential storage.

use serde::{Deserialize, Serialize};
use tauri::command;

use crate::types::{CommandResult, DesktopError};

const REDACTED: &str = "****";

pub fn get_secret(key: &str) -> Result<Option<String>, DesktopError> {
    let entry = keyring::Entry::new("opensymphony", key).map_err(|e| DesktopError::Keychain {
        message: e.to_string(),
    })?;
    match entry.get_password() {
        Ok(v) => Ok(Some(v)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(DesktopError::Keychain {
            message: e.to_string(),
        }),
    }
}

pub fn set_secret(key: &str, value: &str) -> Result<(), DesktopError> {
    let entry = keyring::Entry::new("opensymphony", key).map_err(|e| DesktopError::Keychain {
        message: e.to_string(),
    })?;
    entry
        .set_password(value)
        .map_err(|e| DesktopError::Keychain {
            message: e.to_string(),
        })
}

pub fn delete_secret(key: &str) -> Result<(), DesktopError> {
    let entry = keyring::Entry::new("opensymphony", key).map_err(|e| DesktopError::Keychain {
        message: e.to_string(),
    })?;
    match entry.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(DesktopError::Keychain {
            message: e.to_string(),
        }),
    }
}

pub fn redact_value(key: &str) -> Option<String> {
    match get_secret(key) {
        Ok(Some(_)) => Some(REDACTED.into()),
        Ok(None) => None,
        Err(_) => None, // Don't leak error vs missing-key distinction
    }
}

#[derive(Debug, Deserialize)]
pub struct GetCredentialRequest {
    pub key: String,
}

#[derive(Debug, Serialize)]
pub struct GetCredentialResponse {
    pub redacted_value: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct SetCredentialRequest {
    pub key: String,
    pub value: String,
}

#[derive(Debug, Serialize)]
pub struct SetCredentialResponse {
    pub stored: bool,
}

#[derive(Debug, Deserialize)]
pub struct DeleteCredentialRequest {
    pub key: String,
}

#[derive(Debug, Deserialize)]
pub struct CredentialStatusRequest {
    pub key: String,
}

#[derive(Debug, Serialize)]
pub struct CredentialStatusResponse {
    pub configured: bool,
    pub display: String,
}

#[command]
pub async fn get_credential(req: GetCredentialRequest) -> CommandResult<GetCredentialResponse> {
    Ok(GetCredentialResponse {
        redacted_value: redact_value(&req.key),
    })
}

#[command]
pub async fn set_credential(req: SetCredentialRequest) -> CommandResult<SetCredentialResponse> {
    set_secret(&req.key, &req.value)?;
    Ok(SetCredentialResponse { stored: true })
}

#[command]
pub async fn delete_credential(req: DeleteCredentialRequest) -> CommandResult<()> {
    delete_secret(&req.key)?;
    Ok(())
}

#[command]
pub async fn credential_status(
    req: CredentialStatusRequest,
) -> CommandResult<CredentialStatusResponse> {
    let configured = get_secret(&req.key).map(|v| v.is_some()).unwrap_or(false);
    Ok(CredentialStatusResponse {
        configured,
        display: if configured {
            REDACTED.into()
        } else {
            "none".into()
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_redact_value_does_not_leak_errors() {
        // Missing key should return None (not error information)
        let result = redact_value("nonexistent_key_for_testing_12345");
        assert!(
            result.is_none(),
            "redact_value should not leak error state for missing keys"
        );
    }

    #[tokio::test]
    async fn test_credential_status_hides_missing_or_unavailable_secret() {
        let resp = credential_status(CredentialStatusRequest {
            key: format!("opensymphony-missing-test-{}", std::process::id()),
        })
        .await
        .unwrap();
        assert!(!resp.configured);
        assert_eq!(resp.display, "none");
    }
}

//! Secret-free ACP launch configuration. Values are resolved only at process launch.
use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::{RoutingConfig, WorkflowConfigError};

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AcpConfig {
    #[serde(default)]
    pub profiles: BTreeMap<String, AcpProfile>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AcpProfile {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default = "stdio")]
    pub transport: String,
    #[serde(default = "v1")]
    pub protocol_versions: Vec<u16>,
    #[serde(default)]
    pub env_refs: BTreeMap<String, String>,
    pub auth: Option<AcpAuth>,
    #[serde(default)]
    pub required_capabilities: Vec<String>,
    #[serde(default)]
    pub extensions: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AcpAuth {
    pub method_id: String,
}

fn stdio() -> String {
    "stdio".into()
}
fn v1() -> Vec<u16> {
    vec![1]
}

pub(crate) fn valid_env_name(value: &str) -> bool {
    let mut chars = value.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
}

fn invalid(message: &str) -> WorkflowConfigError {
    WorkflowConfigError::InvalidField {
        field: "acp",
        message: message.into(),
    }
}

impl AcpProfile {
    pub fn validate(&self) -> Result<(), WorkflowConfigError> {
        if self.command.trim().is_empty() || self.command.contains(['\0', '\n', '\r', '$']) {
            return Err(invalid(
                "command must be a nonempty executable, without environment interpolation",
            ));
        }
        if self.args.len() > 128
            || self.args.iter().any(|a| {
                a.len() > 8192
                    || a.contains(['\0', '\n', '\r', '$'])
                    || ["--api-key", "--token", "--password", "--secret"]
                        .iter()
                        .any(|flag| {
                            a.split('=')
                                .next()
                                .is_some_and(|name| name.eq_ignore_ascii_case(flag))
                        })
            })
        {
            return Err(invalid(
                "args must be bounded literal argv entries; credentials belong in env_refs",
            ));
        }
        if self.transport != "stdio" || self.protocol_versions != [1] {
            return Err(invalid(
                "only ACP protocol_versions: [1] with transport: stdio is implemented",
            ));
        }
        if self.env_refs.len() > 128
            || self
                .env_refs
                .iter()
                .any(|(k, v)| !valid_env_name(k) || !valid_env_name(v))
        {
            return Err(invalid(
                "env_refs must map environment variable names to variable names",
            ));
        }
        if self.auth.as_ref().is_some_and(|a| {
            a.method_id.is_empty()
                || a.method_id.len() > 1024
                || a.method_id.chars().any(char::is_control)
        }) {
            return Err(invalid(
                "auth.method_id must be a nonempty bounded opaque identifier without control characters",
            ));
        }
        if !self.extensions.is_empty() {
            return Err(invalid(
                "no extension handlers are implemented; extensions must be empty",
            ));
        }
        if self.required_capabilities.iter().any(|c| {
            !["prompt.image", "prompt.audio", "prompt.embedded_context"].contains(&c.as_str())
        }) {
            return Err(invalid(
                "unknown required capability; supported names: prompt.image, prompt.audio, prompt.embedded_context",
            ));
        }
        Ok(())
    }
}

impl AcpConfig {
    pub fn validate_profiles(&self) -> Result<(), WorkflowConfigError> {
        for (id, profile) in &self.profiles {
            if !valid_id(id) {
                return Err(invalid("profile IDs must be nonempty stable identifiers"));
            }
            profile
                .validate()
                .map_err(|error| invalid(&format!("profile `{id}`: {error}")))?;
        }
        Ok(())
    }

    pub fn validate_selection(
        &self,
        harness: &str,
        profile: Option<&str>,
        model_override: bool,
    ) -> Result<(), WorkflowConfigError> {
        self.validate_profiles()?;
        match (harness, profile) {
            ("acp", Some(id)) if self.profiles.contains_key(id) => {}
            ("acp", _) => {
                return Err(invalid(
                    "routing.harness_profile must name a configured ACP profile",
                ));
            }
            (_, Some(_)) => {
                return Err(invalid(
                    "routing.harness_profile requires routing.harness: acp",
                ));
            }
            _ => {}
        }
        if harness == "acp" && model_override {
            return Err(invalid(
                "ACP model overrides require session configuration support, which is not implemented",
            ));
        }
        Ok(())
    }
}

impl AcpConfig {
    pub fn validate_route(&self, routing: &RoutingConfig) -> Result<(), WorkflowConfigError> {
        self.validate_selection(
            &routing.harness,
            routing.harness_profile.as_deref(),
            routing.model.is_some() || routing.model_profile.is_some(),
        )
    }
}

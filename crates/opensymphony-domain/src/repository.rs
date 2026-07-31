use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

const MANAGED_LABEL_PREFIX: &str = "repo:";

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CanonicalRepositoryId(String);

impl CanonicalRepositoryId {
    pub fn new(value: impl Into<String>) -> Result<Self, RepositoryIdentityError> {
        let value = value.into().trim().to_owned();
        if value.is_empty() || !value.contains(':') {
            return Err(RepositoryIdentityError::InvalidCanonicalId(value));
        }
        Ok(Self(value))
    }

    pub fn from_remote(
        provider: impl AsRef<str>,
        provider_id: Option<&str>,
        locator: impl AsRef<str>,
    ) -> Result<Self, RepositoryIdentityError> {
        let provider = provider_identity(provider.as_ref(), locator.as_ref());
        let durable_key = provider_id
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| normalize_locator_for_provider(provider.as_str(), locator.as_ref()));
        if provider.is_empty() || durable_key.is_empty() {
            return Err(RepositoryIdentityError::MissingRemoteIdentity);
        }
        Self::new(format!("{provider}:repository:{durable_key}"))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for CanonicalRepositoryId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SafeRemoteFingerprint(String);

impl SafeRemoteFingerprint {
    pub fn from_remote(
        provider: impl AsRef<str>,
        provider_id: Option<&str>,
        locator: impl AsRef<str>,
    ) -> Result<Self, RepositoryIdentityError> {
        let provider = provider_identity(provider.as_ref(), locator.as_ref());
        let provider_id = provider_id
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or_default();
        let locator = normalize_locator_for_provider(provider.as_str(), locator.as_ref());
        if provider.is_empty() || (provider_id.is_empty() && locator.is_empty()) {
            return Err(RepositoryIdentityError::MissingRemoteIdentity);
        }
        // Provider-native IDs survive repository renames and transfers. Only
        // fall back to the normalized locator when the provider has no
        // durable native identity to expose.
        let material = if provider_id.is_empty() {
            format!("provider={provider}\u{1f}locator={locator}")
        } else {
            format!("provider={provider}\u{1f}provider_id={provider_id}")
        };
        let mut hasher = Sha256::new();
        hasher.update(material.as_bytes());
        Ok(Self(format!("sha256:{:x}", hasher.finalize())))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepositoryIdentity {
    pub id: CanonicalRepositoryId,
    pub safe_remote_fingerprint: SafeRemoteFingerprint,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepositoryBinding {
    pub alias: String,
    pub repository: RepositoryIdentity,
    pub config_generation: String,
    pub inventory_generation: String,
}

impl RepositoryBinding {
    pub fn repository_id(&self) -> &CanonicalRepositoryId {
        &self.repository.id
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RepositoryRoutingMode {
    LegacySingle,
    ProjectSet,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepositoryInventoryEntry {
    pub alias: String,
    pub identity: RepositoryIdentity,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepositoryRouting {
    pub mode: RepositoryRoutingMode,
    pub inventory: BTreeMap<String, RepositoryInventoryEntry>,
    pub project_repositories: BTreeMap<String, BTreeSet<CanonicalRepositoryId>>,
    pub active_projects: BTreeSet<String>,
    pub legacy_repository: Option<String>,
    pub config_generation: String,
    pub inventory_generation: String,
}

impl RepositoryRouting {
    pub fn resolve(
        &self,
        labels: &[String],
        project_id: Option<&str>,
        project_slug: Option<&str>,
        is_parent: bool,
    ) -> RepositoryBindingOutcome {
        let aliases = managed_repository_aliases(labels);
        if is_parent && !aliases.is_empty() {
            return RepositoryBindingOutcome::ParentBindingNotAllowed;
        }

        let project_keys = [project_id, project_slug]
            .into_iter()
            .flatten()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
            .collect::<Vec<_>>();
        let project_key = project_keys
            .iter()
            .find(|key| self.active_projects.contains(*key))
            .cloned()
            .or_else(|| project_keys.first().cloned());

        if self.mode == RepositoryRoutingMode::ProjectSet {
            let Some(project_key) = project_key.as_deref() else {
                return RepositoryBindingOutcome::ProjectOutsideActiveSet(String::new());
            };
            if !self.active_projects.contains(project_key) {
                return RepositoryBindingOutcome::ProjectOutsideActiveSet(project_key.to_owned());
            }
        }

        let alias = match aliases.as_slice() {
            [] if self.mode == RepositoryRoutingMode::LegacySingle => {
                self.legacy_repository.as_deref().unwrap_or_default()
            }
            [] => return RepositoryBindingOutcome::MissingBinding,
            [alias] => alias.as_str(),
            aliases => return RepositoryBindingOutcome::MultipleBindings(aliases.to_vec()),
        };

        let Some(entry) = self.inventory.get(alias) else {
            return RepositoryBindingOutcome::UnknownAlias(alias.to_owned());
        };

        if self.mode == RepositoryRoutingMode::ProjectSet
            && !project_key
                .as_deref()
                .and_then(|key| self.project_repositories.get(key))
                .is_some_and(|allowed| allowed.contains(&entry.identity.id))
        {
            return RepositoryBindingOutcome::RepositoryNotAllowedForProject(
                entry.identity.id.clone(),
                project_key.unwrap_or_default(),
            );
        }

        RepositoryBindingOutcome::Resolved(RepositoryBinding {
            alias: alias.to_owned(),
            repository: entry.identity.clone(),
            config_generation: self.config_generation.clone(),
            inventory_generation: self.inventory_generation.clone(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RepositoryBindingOutcome {
    Resolved(RepositoryBinding),
    MissingBinding,
    UnknownAlias(String),
    MultipleBindings(Vec<String>),
    RepositoryNotAllowedForProject(CanonicalRepositoryId, String),
    ParentBindingNotAllowed,
    ProjectOutsideActiveSet(String),
}

impl RepositoryBindingOutcome {
    pub fn resolved_binding(&self) -> Option<&RepositoryBinding> {
        match self {
            Self::Resolved(binding) => Some(binding),
            _ => None,
        }
    }

    pub fn repository_id(&self) -> Option<&CanonicalRepositoryId> {
        self.resolved_binding()
            .map(RepositoryBinding::repository_id)
    }

    pub fn canonical_identity_changed(&self, other: &Self) -> bool {
        match (self.repository_id(), other.repository_id()) {
            (Some(left), Some(right)) => left != right,
            _ => self != other,
        }
    }

    pub fn canonical_identity_changed_opt(left: Option<&Self>, right: Option<&Self>) -> bool {
        match (left, right) {
            (Some(left), Some(right)) => left.canonical_identity_changed(right),
            (None, None) => false,
            _ => true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RepositoryIdentityError {
    #[error("invalid canonical repository id `{0}`")]
    InvalidCanonicalId(String),
    #[error("repository remote does not expose a provider-qualified identity")]
    MissingRemoteIdentity,
}

pub fn managed_repository_aliases(labels: &[String]) -> Vec<String> {
    labels
        .iter()
        .filter_map(|label| {
            let alias = label.strip_prefix(MANAGED_LABEL_PREFIX).or_else(|| {
                label
                    .get(..MANAGED_LABEL_PREFIX.len())
                    .filter(|prefix| prefix.eq_ignore_ascii_case(MANAGED_LABEL_PREFIX))
                    .and_then(|_| label.get(MANAGED_LABEL_PREFIX.len()..))
            })?;
            Some(alias.trim().to_owned())
        })
        .collect()
}

fn normalize_component(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

fn provider_identity(provider: &str, locator: &str) -> String {
    let provider = normalize_component(provider);
    let Some(authority) =
        remote_authority(locator).or_else(|| default_provider_authority(&provider))
    else {
        return provider;
    };
    format!("{provider}:{authority}")
}

fn default_provider_authority(provider: &str) -> Option<String> {
    match provider {
        "github" => Some("github.com".to_owned()),
        "gitlab" => Some("gitlab.com".to_owned()),
        "bitbucket" => Some("bitbucket.org".to_owned()),
        _ => None,
    }
}

fn remote_authority(locator: &str) -> Option<String> {
    let raw = locator.trim();
    if raw.is_empty() {
        return None;
    }

    let (has_scheme, without_scheme) = raw
        .split_once("://")
        .map_or((false, raw), |(_, remainder)| (true, remainder));
    let authority = if has_scheme {
        without_scheme.split('/').next().unwrap_or_default()
    } else if let Some(remainder) = raw
        .strip_prefix("git@")
        .or_else(|| raw.strip_prefix("ssh@"))
    {
        remainder.split(':').next().unwrap_or_default()
    } else {
        without_scheme.split('/').next().unwrap_or_default()
    };
    let authority = authority
        .rsplit_once('@')
        .map_or(authority, |(_, authority)| authority)
        .trim_matches(['[', ']']);
    if authority.is_empty()
        || (!has_scheme
            && !authority.contains('.')
            && !authority.contains(':')
            && authority != "localhost")
    {
        return None;
    }
    Some(normalize_component(authority))
}

fn normalize_locator(value: &str) -> String {
    let mut value = value.trim().to_owned();
    if let Some((scheme, without_scheme)) = value.split_once("://")
        && matches!(
            scheme.to_ascii_lowercase().as_str(),
            "http" | "https" | "ssh" | "git"
        )
    {
        value = without_scheme.to_owned();
    }
    value = value.trim_end_matches('/').to_owned();
    value.strip_suffix(".git").unwrap_or(&value).to_owned()
}

fn normalize_locator_for_provider(provider: &str, locator: &str) -> String {
    let normalized = normalize_locator(locator);
    let authority = remote_authority(locator)
        .or_else(|| {
            provider
                .rsplit_once(':')
                .map(|(_, authority)| authority.to_owned())
        })
        .or_else(|| default_provider_authority(&normalize_component(provider)));
    let Some(authority) = authority else {
        return normalized;
    };

    let slash_prefix = format!("{authority}/");
    if let Some(path) =
        strip_case_insensitive_prefix(&normalized, &slash_prefix).filter(|path| !path.is_empty())
    {
        return path.to_owned();
    }

    let colon_prefix = format!("{authority}:");
    if let Some(path) =
        strip_case_insensitive_prefix(&normalized, &colon_prefix).filter(|path| !path.is_empty())
    {
        return path.to_owned();
    }

    if let Some((user, remainder)) = normalized.split_once('@')
        && !user.is_empty()
        && let Some(path) = remainder
            .strip_prefix(&slash_prefix)
            .or_else(|| remainder.strip_prefix(&colon_prefix))
    {
        return path.to_owned();
    }

    normalized
}

fn strip_case_insensitive_prefix<'a>(value: &'a str, prefix: &str) -> Option<&'a str> {
    value
        .get(..prefix.len())
        .filter(|candidate| candidate.eq_ignore_ascii_case(prefix))
        .and_then(|_| value.get(prefix.len()..))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn routing(mode: RepositoryRoutingMode) -> RepositoryRouting {
        let identity = RepositoryIdentity {
            id: CanonicalRepositoryId::from_remote("github", Some("123"), "owner/repo")
                .expect("id"),
            safe_remote_fingerprint: SafeRemoteFingerprint::from_remote(
                "github",
                Some("123"),
                "owner/repo",
            )
            .expect("fingerprint"),
        };
        RepositoryRouting {
            mode,
            inventory: BTreeMap::from([(
                "one".to_string(),
                RepositoryInventoryEntry {
                    alias: "one".to_string(),
                    identity: identity.clone(),
                },
            )]),
            project_repositories: BTreeMap::from([(
                "project-id".to_string(),
                BTreeSet::from([identity.id]),
            )]),
            active_projects: BTreeSet::from(["project-id".to_string()]),
            legacy_repository: Some("one".to_string()),
            config_generation: "config-1".to_string(),
            inventory_generation: "inventory-1".to_string(),
        }
    }

    fn labels(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_string()).collect()
    }

    #[test]
    fn canonical_identity_prefers_provider_native_id() {
        let id = CanonicalRepositoryId::from_remote("GitHub", Some("Repo-42"), "owner/renamed")
            .expect("identity");
        assert_eq!(id.as_str(), "github:github.com:repository:Repo-42");
    }

    #[test]
    fn equivalent_public_provider_locators_share_identity() {
        let shorthand = CanonicalRepositoryId::from_remote("github", Some("42"), "owner/repo")
            .expect("shorthand identity should be valid");
        let url = CanonicalRepositoryId::from_remote(
            "github",
            Some("42"),
            "https://github.com/owner/repo",
        )
        .expect("URL identity should be valid");
        let shorthand_fingerprint =
            SafeRemoteFingerprint::from_remote("github", Some("42"), "owner/repo")
                .expect("shorthand fingerprint should be valid");
        let url_fingerprint = SafeRemoteFingerprint::from_remote(
            "github",
            Some("42"),
            "https://github.com/owner/repo",
        )
        .expect("URL fingerprint should be valid");

        assert_eq!(shorthand, url);
        assert_eq!(shorthand_fingerprint, url_fingerprint);
    }

    #[test]
    fn equivalent_public_provider_locators_share_fallback_identity() {
        let shorthand = CanonicalRepositoryId::from_remote("github", None, "owner/repo")
            .expect("shorthand identity should be valid");
        let url =
            CanonicalRepositoryId::from_remote("github", None, "https://github.com/owner/repo")
                .expect("URL identity should be valid");
        let shorthand_fingerprint =
            SafeRemoteFingerprint::from_remote("github", None, "owner/repo")
                .expect("shorthand fingerprint should be valid");
        let url_fingerprint =
            SafeRemoteFingerprint::from_remote("github", None, "https://github.com/owner/repo")
                .expect("URL fingerprint should be valid");

        assert_eq!(shorthand, url);
        assert_eq!(shorthand_fingerprint, url_fingerprint);
    }

    #[test]
    fn remote_fingerprint_does_not_include_clone_credentials() {
        let first = SafeRemoteFingerprint::from_remote("github", Some("42"), "Owner/Repo")
            .expect("fingerprint");
        let second = SafeRemoteFingerprint::from_remote("github", Some("42"), "owner/repo")
            .expect("fingerprint");
        assert_eq!(first, second);
        assert!(!first.as_str().contains("git@"));
    }

    #[test]
    fn provider_identity_keeps_fingerprint_stable_across_renames() {
        let before = SafeRemoteFingerprint::from_remote("github", Some("42"), "old-owner/old-name")
            .expect("fingerprint");
        let after = SafeRemoteFingerprint::from_remote("github", Some("42"), "new-owner/new-name")
            .expect("fingerprint");
        assert_eq!(before, after);
    }

    #[test]
    fn provider_native_repository_ids_preserve_case() {
        let upper = CanonicalRepositoryId::from_remote("github", Some("RepoA"), "owner/repo")
            .expect("upper-case provider id should be valid");
        let lower = CanonicalRepositoryId::from_remote("github", Some("repoa"), "owner/repo")
            .expect("lower-case provider id should be valid");
        let upper_fingerprint =
            SafeRemoteFingerprint::from_remote("github", Some("RepoA"), "owner/repo")
                .expect("upper-case provider id should fingerprint");
        let lower_fingerprint =
            SafeRemoteFingerprint::from_remote("github", Some("repoa"), "owner/repo")
                .expect("lower-case provider id should fingerprint");

        assert_ne!(upper, lower);
        assert_ne!(upper_fingerprint, lower_fingerprint);
    }

    #[test]
    fn provider_native_identity_is_qualified_by_remote_authority() {
        let github = CanonicalRepositoryId::from_remote(
            "github",
            Some("42"),
            "https://github.com/owner/repository",
        )
        .expect("github identity should be valid");
        let enterprise = CanonicalRepositoryId::from_remote(
            "github",
            Some("42"),
            "https://git.example.test/owner/repository",
        )
        .expect("enterprise identity should be valid");
        let github_fingerprint = SafeRemoteFingerprint::from_remote(
            "github",
            Some("42"),
            "https://github.com/owner/repository",
        )
        .expect("github fingerprint should be valid");
        let enterprise_fingerprint = SafeRemoteFingerprint::from_remote(
            "github",
            Some("42"),
            "https://git.example.test/owner/repository",
        )
        .expect("enterprise fingerprint should be valid");

        assert_ne!(github, enterprise);
        assert_ne!(github_fingerprint, enterprise_fingerprint);
        assert_eq!(github.as_str(), "github:github.com:repository:42");
    }

    #[test]
    fn binding_alias_changes_do_not_change_canonical_identity() {
        let identity = RepositoryIdentity {
            id: CanonicalRepositoryId::new("github:repository:42").expect("identity"),
            safe_remote_fingerprint: SafeRemoteFingerprint::from_remote(
                "github",
                Some("42"),
                "owner/repository",
            )
            .expect("fingerprint"),
        };
        let binding = |alias: &str| {
            RepositoryBindingOutcome::Resolved(RepositoryBinding {
                alias: alias.to_owned(),
                repository: identity.clone(),
                config_generation: format!("config-{alias}"),
                inventory_generation: format!("inventory-{alias}"),
            })
        };

        assert!(!binding("one").canonical_identity_changed(&binding("two")));
    }

    #[test]
    fn locator_derived_repository_ids_preserve_locator_case() {
        let upper = CanonicalRepositoryId::from_remote("github", None, "Owner/Repo")
            .expect("upper-case locator should be valid");
        let lower = CanonicalRepositoryId::from_remote("github", None, "owner/repo")
            .expect("lower-case locator should be valid");
        let upper_fingerprint = SafeRemoteFingerprint::from_remote("github", None, "Owner/Repo")
            .expect("upper-case locator should fingerprint");
        let lower_fingerprint = SafeRemoteFingerprint::from_remote("github", None, "owner/repo")
            .expect("lower-case locator should fingerprint");

        assert_ne!(upper, lower);
        assert_ne!(upper_fingerprint, lower_fingerprint);
    }

    #[test]
    fn unicode_locator_fallback_does_not_panic_on_authority_prefix_probe() {
        let result = std::panic::catch_unwind(|| {
            CanonicalRepositoryId::from_remote("github", None, "éééééé")
        });

        assert!(result.is_ok(), "locator normalization must not panic");
        assert!(
            result
                .expect("locator normalization must not panic")
                .is_ok()
        );
    }

    #[test]
    fn strict_resolution_distinguishes_each_binding_outcome() {
        let cases = [
            (
                labels(&[]),
                Some("project-id"),
                false,
                RepositoryBindingOutcome::MissingBinding,
            ),
            (
                labels(&["repo:missing"]),
                Some("project-id"),
                false,
                RepositoryBindingOutcome::UnknownAlias("missing".to_string()),
            ),
            (
                labels(&["repo:one", "repo:missing"]),
                Some("project-id"),
                false,
                RepositoryBindingOutcome::MultipleBindings(vec![
                    "one".to_string(),
                    "missing".to_string(),
                ]),
            ),
            (
                labels(&["repo:"]),
                Some("project-id"),
                false,
                RepositoryBindingOutcome::UnknownAlias(String::new()),
            ),
            (
                labels(&["repo:", "repo:one"]),
                Some("project-id"),
                false,
                RepositoryBindingOutcome::MultipleBindings(vec!["".to_string(), "one".to_string()]),
            ),
            (
                labels(&["repo:one"]),
                Some("other-project"),
                false,
                RepositoryBindingOutcome::ProjectOutsideActiveSet("other-project".to_string()),
            ),
            (
                labels(&["repo:one"]),
                Some("project-id"),
                true,
                RepositoryBindingOutcome::ParentBindingNotAllowed,
            ),
        ];
        for (labels, project, is_parent, expected) in cases {
            assert_eq!(
                routing(RepositoryRoutingMode::ProjectSet)
                    .resolve(&labels, project, None, is_parent),
                expected
            );
        }
    }

    #[test]
    fn association_never_selects_a_default_and_legacy_mode_does() {
        let strict = routing(RepositoryRoutingMode::ProjectSet);
        assert_eq!(
            strict.resolve(&labels(&[]), Some("project-id"), None, false),
            RepositoryBindingOutcome::MissingBinding
        );
        let legacy = routing(RepositoryRoutingMode::LegacySingle);
        assert!(matches!(
            legacy.resolve(&labels(&[]), None, None, false),
            RepositoryBindingOutcome::Resolved(_)
        ));
    }

    #[test]
    fn project_association_rejects_an_inventory_repository_without_defaulting() {
        let mut strict = routing(RepositoryRoutingMode::ProjectSet);
        strict
            .project_repositories
            .insert("project-id".to_string(), BTreeSet::new());

        assert!(matches!(
            strict.resolve(&labels(&["repo:one"]), Some("project-id"), None, false),
            RepositoryBindingOutcome::RepositoryNotAllowedForProject(_, ref project)
                if project == "project-id"
        ));
    }

    #[test]
    fn project_routing_falls_back_to_an_active_provider_slug() {
        let strict = routing(RepositoryRoutingMode::ProjectSet);

        assert!(matches!(
            strict.resolve(
                &labels(&["repo:one"]),
                Some("stale-provider-id"),
                Some("project-id"),
                false,
            ),
            RepositoryBindingOutcome::Resolved(_)
        ));
    }

    #[test]
    fn managed_empty_labels_remain_visible_to_routing() {
        assert_eq!(
            managed_repository_aliases(&labels(&["repo:"])),
            vec![String::new()]
        );
    }
}

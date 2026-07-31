use std::path::{Component, Path, PathBuf};

use sha2::{Digest, Sha256};

use super::WorkspaceError;

pub fn sanitize_workspace_key(identifier: &str) -> Result<String, WorkspaceError> {
    if identifier.trim().is_empty() {
        return Err(WorkspaceError::EmptyIdentifier);
    }

    let key = identifier
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-') {
                character
            } else {
                '_'
            }
        })
        .collect::<String>();

    if key.is_empty()
        || !matches!(
            Path::new(&key).components().next(),
            Some(Component::Normal(_))
        )
    {
        return Err(WorkspaceError::InvalidWorkspaceKey { key });
    }

    if Path::new(&key).components().nth(1).is_some() {
        return Err(WorkspaceError::InvalidWorkspaceKey { key });
    }

    Ok(key)
}

pub fn workspace_path_for_root(
    root: impl AsRef<Path>,
    issue_identifier: &str,
) -> Result<PathBuf, WorkspaceError> {
    let root = normalize_absolute_path(root.as_ref())?;
    let key = sanitize_workspace_key(issue_identifier)?;
    resolve_path_within_root(root, key)
}

/// Derive a collision-resistant key for a repository-scoped checkout.
pub fn checkout_workspace_key(
    issue_identifier: &str,
    issue_id: &str,
    repository_id: &str,
) -> Result<String, WorkspaceError> {
    let display = sanitize_workspace_key(issue_identifier)?;
    let mut hasher = Sha256::new();
    hasher.update(issue_identifier.as_bytes());
    hasher.update([0]);
    hasher.update(issue_id.as_bytes());
    hasher.update([0]);
    hasher.update(repository_id.as_bytes());
    let digest = format!("{:x}", hasher.finalize());
    Ok(format!("{display}-{}", &digest[..16]))
}

pub fn resolve_path_within_root(
    root: impl AsRef<Path>,
    candidate: impl AsRef<Path>,
) -> Result<PathBuf, WorkspaceError> {
    let root = normalize_absolute_path(root.as_ref())?;
    let candidate = if candidate.as_ref().is_absolute() {
        normalize_absolute_path(candidate.as_ref())?
    } else {
        normalize_absolute_path(&root.join(candidate.as_ref()))?
    };

    if candidate.starts_with(&root) {
        Ok(candidate)
    } else {
        Err(WorkspaceError::PathEscape {
            root,
            path: candidate,
        })
    }
}

pub(crate) fn normalize_absolute_path(path: &Path) -> Result<PathBuf, WorkspaceError> {
    if !path.is_absolute() {
        return Err(WorkspaceError::RootNotAbsolute {
            path: path.to_path_buf(),
        });
    }

    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            Component::RootDir => normalized.push(Path::new(std::path::MAIN_SEPARATOR_STR)),
            Component::CurDir => {}
            Component::ParentDir => {
                let _ = normalized.pop();
            }
            Component::Normal(segment) => normalized.push(segment),
        }
    }

    Ok(normalized)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::WorkspaceError;
    use super::{checkout_workspace_key, resolve_path_within_root, sanitize_workspace_key};

    #[test]
    fn sanitizes_documented_examples() {
        assert_eq!(
            sanitize_workspace_key("ABC-123").expect("documented ticket key should be valid"),
            "ABC-123"
        );
        assert_eq!(
            sanitize_workspace_key("feature/42").expect("slashes should be normalized"),
            "feature_42"
        );
        assert_eq!(
            sanitize_workspace_key("Bug: weird path")
                .expect("spaces and punctuation should be normalized"),
            "Bug__weird_path"
        );
    }

    #[test]
    fn rejects_empty_and_reserved_workspace_keys() {
        assert!(matches!(
            sanitize_workspace_key(""),
            Err(WorkspaceError::EmptyIdentifier)
        ));
        assert!(matches!(
            sanitize_workspace_key("."),
            Err(WorkspaceError::InvalidWorkspaceKey { .. })
        ));
        assert!(matches!(
            sanitize_workspace_key(".."),
            Err(WorkspaceError::InvalidWorkspaceKey { .. })
        ));
    }

    #[test]
    fn containment_helper_rejects_parent_escape() {
        let root = std::env::temp_dir().join("opensymphony-workspace-root");
        let error = resolve_path_within_root(&root, PathBuf::from("../escape"))
            .expect_err("parent-relative candidate should be rejected");

        assert!(matches!(error, WorkspaceError::PathEscape { .. }));
    }

    #[test]
    fn containment_helper_allows_descendants() {
        let root = std::env::temp_dir().join("opensymphony-workspace-root");
        let candidate = resolve_path_within_root(&root, PathBuf::from("child/.opensymphony"))
            .expect("descendant path should remain within root");

        assert!(candidate.ends_with(PathBuf::from("child/.opensymphony")));
    }

    #[test]
    fn checkout_keys_keep_unicode_and_sanitized_collisions_distinct() {
        let slash = checkout_workspace_key("feature/42", "one", "github:repository:1")
            .expect("checkout key should be valid");
        let colon = checkout_workspace_key("feature:42", "two", "github:repository:1")
            .expect("checkout key should be valid");
        let unicode = checkout_workspace_key("feature-é", "three", "github:repository:1")
            .expect("checkout key should be valid");

        assert_ne!(slash, colon);
        assert_ne!(slash, unicode);
        assert_ne!(colon, unicode);
    }
}

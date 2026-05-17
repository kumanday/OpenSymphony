use std::{
    fmt, fs, io,
    path::{Path, PathBuf},
};

use sha2::{Digest, Sha256};
use thiserror::Error;
use uuid::Uuid;

pub const OPENHANDS_CONVERSATIONS_PATH_ENV: &str = "OH_CONVERSATIONS_PATH";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConversationStoreKind {
    Active,
    Archived,
    Legacy,
}

impl ConversationStoreKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Archived => "archived",
            Self::Legacy => "legacy",
        }
    }
}

impl fmt::Display for ConversationStoreKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocatedConversation {
    pub kind: ConversationStoreKind,
    pub path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConversationMoveOutcome {
    Moved {
        from: ConversationStoreKind,
        from_path: PathBuf,
        to: ConversationStoreKind,
        to_path: PathBuf,
    },
    AlreadyInTarget {
        kind: ConversationStoreKind,
        path: PathBuf,
    },
    Missing,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenHandsConversationStorePaths {
    pub repo_key: String,
    pub legacy_root: PathBuf,
    pub repo_root: PathBuf,
    pub active: PathBuf,
    pub archived: PathBuf,
}

impl OpenHandsConversationStorePaths {
    pub fn for_tool_dir(
        tool_dir: impl AsRef<Path>,
        target_repo: impl AsRef<Path>,
    ) -> Result<Self, ConversationStoreError> {
        let target_repo = canonicalize_repo_path(target_repo.as_ref())?;
        let repo_key = repo_store_key(&target_repo);
        let legacy_root = tool_dir.as_ref().join("workspace").join("conversations");
        let repo_root = legacy_root.join("repos").join(&repo_key);
        Ok(Self {
            repo_key,
            active: repo_root.join("active"),
            archived: repo_root.join("archived"),
            repo_root,
            legacy_root,
        })
    }

    pub fn ensure_active_and_archived(&self) -> Result<(), ConversationStoreError> {
        self.create_store_dir(&self.active)?;
        self.create_store_dir(&self.archived)
    }

    pub fn path_for(&self, kind: ConversationStoreKind) -> &Path {
        match kind {
            ConversationStoreKind::Active => &self.active,
            ConversationStoreKind::Archived => &self.archived,
            ConversationStoreKind::Legacy => &self.legacy_root,
        }
    }

    pub fn locate_conversation(
        &self,
        conversation_id: &str,
    ) -> Result<Option<LocatedConversation>, ConversationStoreError> {
        let names = conversation_dir_names(conversation_id)?;
        for kind in [
            ConversationStoreKind::Active,
            ConversationStoreKind::Archived,
            ConversationStoreKind::Legacy,
        ] {
            for name in &names {
                let path = self.path_for(kind).join(name);
                if path.is_dir() {
                    return Ok(Some(LocatedConversation { kind, path }));
                }
            }
        }
        Ok(None)
    }

    pub fn move_conversation_to(
        &self,
        conversation_id: &str,
        target: ConversationStoreKind,
    ) -> Result<ConversationMoveOutcome, ConversationStoreError> {
        if target == ConversationStoreKind::Legacy {
            return Err(ConversationStoreError::InvalidTarget { target });
        }

        self.ensure_active_and_archived()?;
        let Some(located) = self.locate_conversation(conversation_id)? else {
            return Ok(ConversationMoveOutcome::Missing);
        };
        if located.kind == target {
            self.remove_duplicate_conversations(conversation_id, target, &located.path)?;
            return Ok(ConversationMoveOutcome::AlreadyInTarget {
                kind: located.kind,
                path: located.path,
            });
        }

        let destination = self
            .path_for(target)
            .join(conversation_dir_name(conversation_id)?);
        if destination.exists() {
            if destination.is_dir() {
                remove_conversation_dir(&located.path)?;
                self.remove_duplicate_conversations(conversation_id, target, &destination)?;
                return Ok(ConversationMoveOutcome::AlreadyInTarget {
                    kind: target,
                    path: destination,
                });
            }
            return Err(ConversationStoreError::DestinationExists { destination });
        }
        move_conversation_dir(&located.path, &destination)?;

        Ok(ConversationMoveOutcome::Moved {
            from: located.kind,
            from_path: located.path,
            to: target,
            to_path: destination,
        })
    }

    fn create_store_dir(&self, path: &Path) -> Result<(), ConversationStoreError> {
        fs::create_dir_all(path).map_err(|source| ConversationStoreError::CreateDirectory {
            path: path.to_path_buf(),
            source,
        })
    }

    fn remove_duplicate_conversations(
        &self,
        conversation_id: &str,
        target: ConversationStoreKind,
        target_path: &Path,
    ) -> Result<(), ConversationStoreError> {
        let names = conversation_dir_names(conversation_id)?;
        for kind in [
            ConversationStoreKind::Active,
            ConversationStoreKind::Archived,
            ConversationStoreKind::Legacy,
        ] {
            if kind == target {
                continue;
            }
            for name in &names {
                let path = self.path_for(kind).join(name);
                if path == target_path {
                    continue;
                }
                if path.is_dir() {
                    remove_conversation_dir(&path)?;
                }
            }
        }
        Ok(())
    }
}

#[derive(Debug, Error)]
pub enum ConversationStoreError {
    #[error("failed to resolve target repository path {path}: {source}")]
    ResolveRepoPath {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("conversation id `{value}` is not a UUID: {source}")]
    InvalidConversationId {
        value: String,
        #[source]
        source: uuid::Error,
    },
    #[error("failed to create OpenHands conversation store {path}: {source}")]
    CreateDirectory {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("cannot move conversations into the {target} OpenHands store")]
    InvalidTarget { target: ConversationStoreKind },
    #[error("OpenHands conversation archive destination already exists: {destination}")]
    DestinationExists { destination: PathBuf },
    #[error("failed to move OpenHands conversation from {from} to {to}: {source}")]
    MoveConversation {
        from: PathBuf,
        to: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("failed to copy OpenHands conversation from {from} to {to}: {source}")]
    CopyConversation {
        from: PathBuf,
        to: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("failed to remove OpenHands conversation {path}: {source}")]
    RemoveConversation {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

fn canonicalize_repo_path(path: &Path) -> Result<PathBuf, ConversationStoreError> {
    fs::canonicalize(path).map_err(|source| ConversationStoreError::ResolveRepoPath {
        path: path.to_path_buf(),
        source,
    })
}

fn repo_store_key(target_repo: &Path) -> String {
    let slug_source = target_repo
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("repo");
    let slug = sanitize_repo_slug(slug_source);
    let digest = Sha256::digest(target_repo.to_string_lossy().as_bytes());
    let hash = digest[..6]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("{slug}-{hash}")
}

fn sanitize_repo_slug(value: &str) -> String {
    let mut output = String::new();
    let mut last_was_separator = false;
    for character in value.chars() {
        let next = if character.is_ascii_alphanumeric() {
            last_was_separator = false;
            Some(character.to_ascii_lowercase())
        } else if matches!(character, '-' | '_' | '.') {
            if last_was_separator {
                None
            } else {
                last_was_separator = true;
                Some('-')
            }
        } else if last_was_separator {
            None
        } else {
            last_was_separator = true;
            Some('-')
        };
        if let Some(next) = next
            && output.len() < 48
        {
            output.push(next);
        }
    }
    let trimmed = output.trim_matches('-').to_string();
    if trimmed.is_empty() {
        "repo".to_string()
    } else {
        trimmed
    }
}

fn conversation_dir_names(conversation_id: &str) -> Result<Vec<String>, ConversationStoreError> {
    let compact = conversation_dir_name(conversation_id)?;
    let raw = conversation_id.trim().to_string();
    // Current OpenHands local stores use compact UUID directory names. The raw
    // UUID fallback keeps debug/archive tolerant of manually restored stores or
    // older exported snapshots named after the API-facing conversation UUID.
    if raw == compact {
        Ok(vec![compact])
    } else {
        Ok(vec![compact, raw])
    }
}

fn conversation_dir_name(conversation_id: &str) -> Result<String, ConversationStoreError> {
    Uuid::parse_str(conversation_id.trim())
        .map(|uuid| uuid.simple().to_string())
        .map_err(|source| ConversationStoreError::InvalidConversationId {
            value: conversation_id.to_string(),
            source,
        })
}

fn move_conversation_dir(from: &Path, to: &Path) -> Result<(), ConversationStoreError> {
    move_conversation_dir_with_ops(
        from,
        to,
        |from, to| fs::rename(from, to),
        copy_conversation_dir,
    )
}

fn move_conversation_dir_with_ops<R, C>(
    from: &Path,
    to: &Path,
    rename: R,
    copy: C,
) -> Result<(), ConversationStoreError>
where
    R: FnOnce(&Path, &Path) -> io::Result<()>,
    C: FnOnce(&Path, &Path) -> Result<(), ConversationStoreError>,
{
    match rename(from, to) {
        Ok(()) => Ok(()),
        Err(source) if source.kind() == io::ErrorKind::CrossesDevices => {
            if let Err(error) = copy(from, to) {
                let _ = fs::remove_dir_all(to);
                return Err(error);
            }
            remove_conversation_dir(from)
        }
        Err(source) => Err(ConversationStoreError::MoveConversation {
            from: from.to_path_buf(),
            to: to.to_path_buf(),
            source,
        }),
    }
}

fn remove_conversation_dir(path: &Path) -> Result<(), ConversationStoreError> {
    fs::remove_dir_all(path).map_err(|source| ConversationStoreError::RemoveConversation {
        path: path.to_path_buf(),
        source,
    })
}

fn copy_conversation_dir(from: &Path, to: &Path) -> Result<(), ConversationStoreError> {
    fs::create_dir(to).map_err(|source| ConversationStoreError::CopyConversation {
        from: from.to_path_buf(),
        to: to.to_path_buf(),
        source,
    })?;
    for entry in fs::read_dir(from).map_err(|source| ConversationStoreError::CopyConversation {
        from: from.to_path_buf(),
        to: to.to_path_buf(),
        source,
    })? {
        let entry = entry.map_err(|source| ConversationStoreError::CopyConversation {
            from: from.to_path_buf(),
            to: to.to_path_buf(),
            source,
        })?;
        let source_path = entry.path();
        let destination_path = to.join(entry.file_name());
        let file_type =
            entry
                .file_type()
                .map_err(|source| ConversationStoreError::CopyConversation {
                    from: source_path.clone(),
                    to: destination_path.clone(),
                    source,
                })?;
        if file_type.is_dir() {
            copy_conversation_dir(&source_path, &destination_path)?;
        } else if file_type.is_file() {
            fs::copy(&source_path, &destination_path).map_err(|source| {
                ConversationStoreError::CopyConversation {
                    from: source_path.clone(),
                    to: destination_path.clone(),
                    source,
                }
            })?;
        } else if file_type.is_symlink() {
            copy_symlink(&source_path, &destination_path).map_err(|source| {
                ConversationStoreError::CopyConversation {
                    from: source_path.clone(),
                    to: destination_path.clone(),
                    source,
                }
            })?;
        } else {
            return Err(ConversationStoreError::CopyConversation {
                from: source_path.clone(),
                to: destination_path.clone(),
                source: io::Error::other("unsupported file type in conversation directory"),
            });
        }
    }
    Ok(())
}

#[cfg(unix)]
fn copy_symlink(from: &Path, to: &Path) -> io::Result<()> {
    std::os::unix::fs::symlink(fs::read_link(from)?, to)
}

#[cfg(not(unix))]
fn copy_symlink(_from: &Path, _to: &Path) -> io::Result<()> {
    Err(io::Error::other(
        "copying symlinks is unsupported on this platform",
    ))
}

#[cfg(test)]
mod tests {
    use super::{ConversationMoveOutcome, ConversationStoreKind, OpenHandsConversationStorePaths};

    #[test]
    fn store_paths_are_repo_scoped_under_managed_conversations() {
        let tool_dir = tempfile::tempdir().expect("tool dir");
        let repo = tempfile::tempdir().expect("repo");

        let paths = OpenHandsConversationStorePaths::for_tool_dir(tool_dir.path(), repo.path())
            .expect("paths should resolve");

        assert!(paths.legacy_root.ends_with("workspace/conversations"));
        assert!(paths.repo_root.starts_with(&paths.legacy_root));
        assert!(paths.repo_root.ends_with(&paths.repo_key));
        assert_eq!(paths.active, paths.repo_root.join("active"));
        assert_eq!(paths.archived, paths.repo_root.join("archived"));
    }

    #[test]
    fn locate_checks_active_archived_then_legacy_compact_uuid_dirs() {
        let tool_dir = tempfile::tempdir().expect("tool dir");
        let repo = tempfile::tempdir().expect("repo");
        let paths = OpenHandsConversationStorePaths::for_tool_dir(tool_dir.path(), repo.path())
            .expect("paths should resolve");
        let conversation_id = "dd258bb7-cc1b-415c-9892-e19af34a2e66";
        let compact_id = "dd258bb7cc1b415c9892e19af34a2e66";
        std::fs::create_dir_all(paths.archived.join(compact_id))
            .expect("archived conversation should be created");
        std::fs::create_dir_all(paths.legacy_root.join(compact_id))
            .expect("legacy duplicate should be created");

        let located = paths
            .locate_conversation(conversation_id)
            .expect("lookup should succeed")
            .expect("conversation should be found");

        assert_eq!(located.kind, ConversationStoreKind::Archived);
        assert_eq!(located.path, paths.archived.join(compact_id));
    }

    #[test]
    fn move_conversation_to_archive_moves_from_active_store() {
        let tool_dir = tempfile::tempdir().expect("tool dir");
        let repo = tempfile::tempdir().expect("repo");
        let paths = OpenHandsConversationStorePaths::for_tool_dir(tool_dir.path(), repo.path())
            .expect("paths should resolve");
        let conversation_id = "dd258bb7-cc1b-415c-9892-e19af34a2e66";
        let compact_id = "dd258bb7cc1b415c9892e19af34a2e66";
        let active = paths.active.join(compact_id);
        std::fs::create_dir_all(&active).expect("active conversation should be created");

        let outcome = paths
            .move_conversation_to(conversation_id, ConversationStoreKind::Archived)
            .expect("move should succeed");

        assert!(matches!(
            outcome,
            ConversationMoveOutcome::Moved {
                from: ConversationStoreKind::Active,
                to: ConversationStoreKind::Archived,
                ..
            }
        ));
        assert!(!active.exists());
        assert!(paths.archived.join(compact_id).is_dir());
    }

    #[test]
    fn move_conversation_to_archive_migrates_legacy_flat_store() {
        let tool_dir = tempfile::tempdir().expect("tool dir");
        let repo = tempfile::tempdir().expect("repo");
        let paths = OpenHandsConversationStorePaths::for_tool_dir(tool_dir.path(), repo.path())
            .expect("paths should resolve");
        let conversation_id = "dd258bb7-cc1b-415c-9892-e19af34a2e66";
        let compact_id = "dd258bb7cc1b415c9892e19af34a2e66";
        let legacy = paths.legacy_root.join(compact_id);
        std::fs::create_dir_all(&legacy).expect("legacy conversation should be created");

        let outcome = paths
            .move_conversation_to(conversation_id, ConversationStoreKind::Archived)
            .expect("move should succeed");

        assert!(matches!(
            outcome,
            ConversationMoveOutcome::Moved {
                from: ConversationStoreKind::Legacy,
                to: ConversationStoreKind::Archived,
                ..
            }
        ));
        assert!(!legacy.exists());
        assert!(paths.archived.join(compact_id).is_dir());
    }

    #[test]
    fn move_conversation_treats_existing_target_directory_as_idempotent() {
        let tool_dir = tempfile::tempdir().expect("tool dir");
        let repo = tempfile::tempdir().expect("repo");
        let paths = OpenHandsConversationStorePaths::for_tool_dir(tool_dir.path(), repo.path())
            .expect("paths should resolve");
        let conversation_id = "dd258bb7-cc1b-415c-9892-e19af34a2e66";
        let compact_id = "dd258bb7cc1b415c9892e19af34a2e66";
        let active = paths.active.join(compact_id);
        let legacy = paths.legacy_root.join(compact_id);
        std::fs::create_dir_all(&active).expect("active conversation should be created");
        std::fs::create_dir_all(&legacy).expect("legacy duplicate should be created");

        let outcome = paths
            .move_conversation_to(conversation_id, ConversationStoreKind::Active)
            .expect("move should reconcile duplicate");

        assert!(matches!(
            outcome,
            ConversationMoveOutcome::AlreadyInTarget {
                kind: ConversationStoreKind::Active,
                ..
            }
        ));
        assert!(active.is_dir());
        assert!(!legacy.exists());
    }

    #[test]
    fn locate_supports_raw_uuid_directory_names() {
        let tool_dir = tempfile::tempdir().expect("tool dir");
        let repo = tempfile::tempdir().expect("repo");
        let paths = OpenHandsConversationStorePaths::for_tool_dir(tool_dir.path(), repo.path())
            .expect("paths should resolve");
        let conversation_id = "dd258bb7-cc1b-415c-9892-e19af34a2e66";
        let raw_path = paths.active.join(conversation_id);
        std::fs::create_dir_all(&raw_path).expect("raw conversation should be created");

        let located = paths
            .locate_conversation(conversation_id)
            .expect("lookup should succeed")
            .expect("conversation should be found");

        assert_eq!(located.kind, ConversationStoreKind::Active);
        assert_eq!(located.path, raw_path);
    }

    #[test]
    fn copy_conversation_dir_copies_nested_files() {
        let temp = tempfile::tempdir().expect("temp dir");
        let source = temp.path().join("source");
        let destination = temp.path().join("destination");
        std::fs::create_dir_all(source.join("events")).expect("source should be created");
        std::fs::write(source.join("meta.json"), "{}").expect("meta should write");
        std::fs::write(source.join("events").join("1.json"), "{\"id\":\"1\"}")
            .expect("event should write");

        super::copy_conversation_dir(&source, &destination).expect("copy should succeed");

        assert_eq!(
            std::fs::read_to_string(destination.join("meta.json")).expect("meta should read"),
            "{}"
        );
        assert_eq!(
            std::fs::read_to_string(destination.join("events").join("1.json"))
                .expect("event should read"),
            "{\"id\":\"1\"}"
        );
    }

    #[test]
    fn move_conversation_dir_falls_back_to_copy_on_cross_device_rename() {
        let temp = tempfile::tempdir().expect("temp dir");
        let source = temp.path().join("source");
        let destination = temp.path().join("destination");
        std::fs::create_dir_all(source.join("events")).expect("source should be created");
        std::fs::write(source.join("events").join("1.json"), "{\"id\":\"1\"}")
            .expect("event should write");

        super::move_conversation_dir_with_ops(
            &source,
            &destination,
            |_from, _to| {
                Err(std::io::Error::new(
                    std::io::ErrorKind::CrossesDevices,
                    "exdev",
                ))
            },
            super::copy_conversation_dir,
        )
        .expect("cross-device fallback should copy then remove source");

        assert!(!source.exists());
        assert_eq!(
            std::fs::read_to_string(destination.join("events").join("1.json"))
                .expect("event should read"),
            "{\"id\":\"1\"}"
        );
    }

    #[test]
    fn move_conversation_dir_removes_partial_copy_after_cross_device_failure() {
        let temp = tempfile::tempdir().expect("temp dir");
        let source = temp.path().join("source");
        let destination = temp.path().join("destination");
        std::fs::create_dir_all(&source).expect("source should be created");

        let error = super::move_conversation_dir_with_ops(
            &source,
            &destination,
            |_from, _to| {
                Err(std::io::Error::new(
                    std::io::ErrorKind::CrossesDevices,
                    "exdev",
                ))
            },
            |from, to| {
                std::fs::create_dir_all(to).expect("partial destination should be created");
                std::fs::write(to.join("partial"), "partial").expect("partial should write");
                Err(super::ConversationStoreError::CopyConversation {
                    from: from.to_path_buf(),
                    to: to.to_path_buf(),
                    source: std::io::Error::other("copy failed"),
                })
            },
        )
        .expect_err("copy failure should be returned");

        assert!(matches!(
            error,
            super::ConversationStoreError::CopyConversation { .. }
        ));
        assert!(source.is_dir());
        assert!(!destination.exists());
    }
}

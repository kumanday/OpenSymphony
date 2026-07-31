impl MemoryConfig {
    pub fn with_repository_source(
        mut self,
        source: MemoryRepositorySource,
    ) -> Self {
        self.repository_sources
            .insert(source.repository_id.clone(), source);
        self
    }

    pub fn with_default_repository_id(mut self, repository_id: impl Into<String>) -> Self {
        self.default_repository_id = Some(repository_id.into());
        self
    }
}

pub fn register_memory_source(
    config: &MemoryConfig,
    source: &RegisteredMemorySource,
) -> Result<(), MemoryError> {
    validate_memory_source(source)?;
    let connection = open_index(config)?;
    migrate_index(&connection).map_err(|error| MemoryError::DuckDb {
        path: config.index_path.clone(),
        source: error,
    })?;
    let generation = source.generation.clone();
    connection
        .execute(
            "INSERT INTO registered_memory_sources (source_id, repository_id, commit_sha, source_kind, source_root, status, generation, registered_at) VALUES (?, ?, ?, ?, ?, ?, ?, CAST(current_timestamp AS TEXT)) ON CONFLICT (source_id) DO UPDATE SET repository_id = excluded.repository_id, commit_sha = excluded.commit_sha, source_kind = excluded.source_kind, source_root = excluded.source_root, status = excluded.status, generation = excluded.generation, registered_at = excluded.registered_at",
            duckdb::params![
                source.source_id,
                source.repository_id,
                source.commit_sha,
                source.kind.as_str(),
                source.root.to_string_lossy().to_string(),
                source.status.as_str(),
                generation,
            ],
        )
        .map_err(|error| MemoryError::DuckDb {
            path: config.index_path.clone(),
            source: error,
        })?;
    Ok(())
}

pub fn registered_memory_sources(
    config: &MemoryConfig,
) -> Result<Vec<RegisteredMemorySource>, MemoryError> {
    let Some(connection) = open_existing_index_read_only(config)? else {
        return Ok(Vec::new());
    };
    let exists: bool = connection
        .query_row(
            "SELECT count(*) > 0 FROM information_schema.tables WHERE table_name = 'registered_memory_sources'",
            [],
            |row| row.get(0),
        )
        .map_err(|error| MemoryError::DuckDb {
            path: config.index_path.clone(),
            source: error,
        })?;
    if !exists {
        return Ok(Vec::new());
    }
    let mut statement = connection
        .prepare("SELECT source_id, repository_id, commit_sha, source_kind, source_root, status, generation FROM registered_memory_sources ORDER BY source_id")
        .map_err(|error| MemoryError::DuckDb {
            path: config.index_path.clone(),
            source: error,
        })?;
    let rows = statement
        .query_map([], |row| {
            Ok(RegisteredMemorySource {
                source_id: row.get(0)?,
                repository_id: row.get(1)?,
                commit_sha: row.get(2)?,
                kind: parse_memory_source_kind(&row.get::<_, String>(3)?),
                root: PathBuf::from(row.get::<_, String>(4)?),
                status: parse_memory_source_status(&row.get::<_, String>(5)?),
                generation: row.get(6)?,
            })
        })
        .map_err(|error| MemoryError::DuckDb {
            path: config.index_path.clone(),
            source: error,
        })?;
    rows.map(|row| {
        row.map_err(|error| MemoryError::DuckDb {
            path: config.index_path.clone(),
            source: error,
        })
    })
    .collect()
}

pub fn reconcile_memory_sources(
    config: &MemoryConfig,
    source_ids: &BTreeSet<String>,
) -> Result<(), MemoryError> {
    let connection = open_index(config)?;
    migrate_index(&connection).map_err(|error| MemoryError::DuckDb {
        path: config.index_path.clone(),
        source: error,
    })?;
    if source_ids.is_empty() {
        connection
            .execute("DELETE FROM registered_memory_sources", [])
            .map_err(|error| MemoryError::DuckDb {
                path: config.index_path.clone(),
                source: error,
            })?;
        return Ok(());
    }
    let placeholders = std::iter::repeat_n("?", source_ids.len())
        .collect::<Vec<_>>()
        .join(", ");
    let values = source_ids.iter().cloned().collect::<Vec<_>>();
    connection
        .execute(
            &format!(
                "DELETE FROM registered_memory_sources WHERE source_id NOT IN ({placeholders})"
            ),
            duckdb::params_from_iter(values),
        )
        .map_err(|error| MemoryError::DuckDb {
            path: config.index_path.clone(),
            source: error,
        })?;
    Ok(())
}

pub fn persist_scope_refs(
    config: &MemoryConfig,
    concept_id: &str,
    scope_refs: &[KnowledgeScope],
) -> Result<(), MemoryError> {
    let mut connection = open_index(config)?;
    migrate_index(&connection).map_err(|error| MemoryError::DuckDb {
        path: config.index_path.clone(),
        source: error,
    })?;
    let transaction = connection.transaction().map_err(|error| MemoryError::DuckDb {
        path: config.index_path.clone(),
        source: error,
    })?;
    transaction
        .execute("DELETE FROM scope_refs WHERE concept_id = ?", [concept_id])
        .map_err(|error| MemoryError::DuckDb {
            path: config.index_path.clone(),
            source: error,
        })?;
    for scope_ref in scope_refs {
        let encoded = serde_json::to_string(&scope_ref.kind)?;
        transaction
            .execute(
                "INSERT INTO scope_refs (concept_id, scope_kind, scope_id, label) VALUES (?, ?, ?, ?)",
                duckdb::params![concept_id, encoded.trim_matches('"'), scope_ref.id, scope_ref.label],
            )
            .map_err(|error| MemoryError::DuckDb {
                path: config.index_path.clone(),
                source: error,
            })?;
    }
    transaction.commit().map_err(|error| MemoryError::DuckDb {
        path: config.index_path.clone(),
        source: error,
    })
}

fn validate_memory_source(source: &RegisteredMemorySource) -> Result<(), MemoryError> {
    if !source.repository_id.contains(':') {
        return Err(MemoryError::InvalidInput(
            "memory source repository_id must be a canonical provider-qualified ID".to_string(),
        ));
    }
    if source.commit_sha.trim().is_empty() {
        return Err(MemoryError::InvalidInput(
            "memory source commit_sha must not be empty".to_string(),
        ));
    }
    if source.source_id.trim().is_empty() {
        return Err(MemoryError::InvalidInput(
            "memory source source_id must not be empty".to_string(),
        ));
    }
    Ok(())
}

fn parse_memory_source_kind(value: &str) -> MemorySourceKind {
    match value {
        "public_docs" => MemorySourceKind::PublicDocs,
        "okf_bundle" => MemorySourceKind::OkfBundle,
        "legacy_store" => MemorySourceKind::LegacyStore,
        _ => MemorySourceKind::Policy,
    }
}

fn parse_memory_source_status(value: &str) -> MemorySourceRegistrationStatus {
    match value {
        "registered" => MemorySourceRegistrationStatus::Registered,
        "failed" => MemorySourceRegistrationStatus::Failed,
        _ => MemorySourceRegistrationStatus::Pending,
    }
}

fn scope_kind_name(kind: &KnowledgeScopeKind) -> &'static str {
    match kind {
        KnowledgeScopeKind::LocalInstance => "local_instance",
        KnowledgeScopeKind::Organization => "organization",
        KnowledgeScopeKind::ProjectSet => "project_set",
        KnowledgeScopeKind::Project => "project",
        KnowledgeScopeKind::Milestone => "milestone",
        KnowledgeScopeKind::WorkItem => "work_item",
        KnowledgeScopeKind::Repository => "repository",
        KnowledgeScopeKind::CodePath => "code_path",
        KnowledgeScopeKind::Area => "area",
    }
}

#[cfg(test)]
mod catalog_tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn registers_canonical_sources_and_normalizes_scope_refs() {
        let root = TempDir::new().expect("memory root");
        let config = MemoryConfig::load(root.path(), None).expect("config");
        let source = RegisteredMemorySource {
            source_id: "github:repository:123:legacy_store".to_string(),
            repository_id: "github:repository:123".to_string(),
            commit_sha: "abc123".to_string(),
            kind: MemorySourceKind::LegacyStore,
            root: root.path().join(".opensymphony/memory"),
            status: MemorySourceRegistrationStatus::Registered,
            generation: "sha256:generation".to_string(),
        };

        register_memory_source(&config, &source).expect("source should register");
        persist_scope_refs(
            &config,
            "issues/COE-550",
            &[
                KnowledgeScope {
                    kind: KnowledgeScopeKind::Project,
                    id: "project-a".to_string(),
                    label: None,
                },
                KnowledgeScope {
                    kind: KnowledgeScopeKind::Repository,
                    id: source.repository_id.clone(),
                    label: Some("Repository A".to_string()),
                },
            ],
        )
        .expect("scope refs should persist");

        assert_eq!(registered_memory_sources(&config).expect("sources")[0], source);
        let connection = open_existing_index_read_only(&config)
            .expect("index should open")
            .expect("index should exist");
        let count: i64 = connection
            .query_row(
                "SELECT count(*) FROM scope_refs WHERE concept_id = 'issues/COE-550'",
                [],
                |row| row.get(0),
            )
            .expect("scope refs");
        assert_eq!(count, 2);
    }
}

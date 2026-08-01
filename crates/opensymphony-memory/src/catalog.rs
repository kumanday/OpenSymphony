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

    pub fn with_default_project_set_id(mut self, project_set_id: impl Into<String>) -> Self {
        self.default_project_set_id = Some(project_set_id.into());
        self
    }

    pub fn with_project_scope_ids(
        mut self,
        project_scope_ids: impl IntoIterator<Item = String>,
    ) -> Self {
        self.project_scope_ids.extend(project_scope_ids);
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
    let mut existing = connection
        .prepare("SELECT source_id, repository_id, source_kind FROM registered_memory_sources")
        .map_err(|error| MemoryError::DuckDb {
            path: config.index_path.clone(),
            source: error,
        })?;
    let existing = existing
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(|error| MemoryError::DuckDb {
            path: config.index_path.clone(),
            source: error,
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| MemoryError::DuckDb {
            path: config.index_path.clone(),
            source: error,
        })?;
    let withdrawn_source_ids = existing
        .iter()
        .filter(|(source_id, _, source_kind)| {
            !source_ids.contains(source_id)
                && matches!(
                    source_kind.as_str(),
                    "repository" | "legacy_store" | "okf_bundle"
                )
        })
        .map(|(source_id, repository_id, _)| (source_id.clone(), repository_id.clone()))
        .collect::<Vec<_>>();
    let current_repository_ids = existing
        .iter()
        .filter(|(source_id, _, _)| source_ids.contains(source_id))
        .map(|(_, repository_id, _)| repository_id.clone())
        .collect::<BTreeSet<_>>();
    let withdrawn_repository_ids = withdrawn_source_ids
        .iter()
        .map(|(_, repository_id)| repository_id)
        .filter(|repository_id| !current_repository_ids.contains(*repository_id))
        .cloned()
        .collect::<BTreeSet<_>>();
    drop(connection);
    for repository_id in &withdrawn_repository_ids {
        withdraw_code_repository(config, repository_id)?;
    }
    for (source_id, repository_id) in withdrawn_source_ids {
        withdraw_memory_source_records(config, &source_id, &repository_id)?;
        let connection = open_index(config)?;
        connection
            .execute(
                "DELETE FROM registered_memory_sources WHERE source_id = ?",
                [&source_id],
            )
            .map_err(|error| MemoryError::DuckDb {
                path: config.index_path.clone(),
                source: error,
            })?;
    }
    let placeholders = std::iter::repeat_n("?", source_ids.len())
        .collect::<Vec<_>>()
        .join(", ");
    let connection = open_index(config)?;
    if source_ids.is_empty() {
        connection
            .execute("DELETE FROM registered_memory_sources", [])
            .map_err(|error| MemoryError::DuckDb {
                path: config.index_path.clone(),
                source: error,
            })?;
    } else {
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
    }
    Ok(())
}

pub fn withdraw_memory_source_records(
    config: &MemoryConfig,
    source_id: &str,
    repository_id: &str,
) -> Result<(), MemoryError> {
    let mut connection = open_index(config)?;
    migrate_index(&connection).map_err(|error| MemoryError::DuckDb {
        path: config.index_path.clone(),
        source: error,
    })?;
    let mut statement = connection
        .prepare(
            "SELECT issue_key, concept_id, source_ids_json, scope_refs_json, source_refs_json FROM issues",
        )
        .map_err(|error| MemoryError::DuckDb {
            path: config.index_path.clone(),
            source: error,
        })?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
            ))
        })
        .map_err(|error| MemoryError::DuckDb {
            path: config.index_path.clone(),
            source: error,
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| MemoryError::DuckDb {
            path: config.index_path.clone(),
            source: error,
        })?;
    drop(statement);
    let transaction = connection.transaction().map_err(|error| MemoryError::DuckDb {
        path: config.index_path.clone(),
        source: error,
    })?;
    // Relation rows carry source ownership independently of the logical issue.
    // Remove this source's evidence even when another source still owns the
    // shared issue; otherwise withdrawn repositories leak stale areas, PRs,
    // files, checks, and reviews through the surviving row.
    for table in [
        "issue_areas",
        "pull_requests",
        "changed_files",
        "checks",
        "reviews",
    ] {
        transaction
            .execute(&format!("DELETE FROM {table} WHERE source_id = ?"), [source_id])
            .map_err(|error| MemoryError::DuckDb {
                path: config.index_path.clone(),
                source: error,
            })?;
    }
    transaction
        .execute(
            "DELETE FROM source_scope_refs WHERE source_id = ?",
            [source_id],
        )
        .map_err(|error| MemoryError::DuckDb {
            path: config.index_path.clone(),
            source: error,
        })?;
    let registered_source_repositories = {
        let mut statement = transaction
            .prepare("SELECT source_id, repository_id FROM registered_memory_sources")
            .map_err(|error| MemoryError::DuckDb {
                path: config.index_path.clone(),
                source: error,
            })?;
        statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|error| MemoryError::DuckDb {
                path: config.index_path.clone(),
                source: error,
            })?
            .collect::<Result<BTreeMap<_, _>, _>>()
            .map_err(|error| MemoryError::DuckDb {
                path: config.index_path.clone(),
                source: error,
            })?
    };
    for (issue_key, concept_id, encoded_source_ids, encoded_scopes, encoded_sources) in rows {
        let mut source_ids = serde_json::from_str::<Vec<String>>(&encoded_source_ids)
            .unwrap_or_default();
        if !source_ids.iter().any(|value| value == source_id) {
            continue;
        }
        source_ids.retain(|value| value != source_id);
        if source_ids.is_empty() {
            transaction
                .execute("DELETE FROM scope_refs WHERE concept_id = ?", [&concept_id])
                .map_err(|error| MemoryError::DuckDb {
                    path: config.index_path.clone(),
                    source: error,
                })?;
            transaction
                .execute("DELETE FROM issues WHERE issue_key = ?", [&issue_key])
                .map_err(|error| MemoryError::DuckDb {
                    path: config.index_path.clone(),
                    source: error,
                })?;
            for table in [
                "issue_areas",
                "pull_requests",
                "changed_files",
                "checks",
                "reviews",
            ] {
                transaction
                    .execute(&format!("DELETE FROM {table} WHERE issue_key = ?"), [&issue_key])
                    .map_err(|error| MemoryError::DuckDb {
                        path: config.index_path.clone(),
                        source: error,
                    })?;
            }
        } else {
            let mut scopes = serde_json::from_str::<Vec<KnowledgeScope>>(&encoded_scopes)
                .unwrap_or_default();
            let mut sources = serde_json::from_str::<Vec<MemorySourceRef>>(&encoded_sources)
                .unwrap_or_default();
            let has_other_source = source_ids.iter().any(|candidate| {
                candidate != source_id
                    && registered_source_repositories
                        .get(candidate)
                        .is_some_and(|owner| owner == repository_id)
            });
            sources.retain(|source| {
                source.registration_source_id.as_deref() != Some(source_id)
                    && !(source.registration_source_id.is_none() && source.id == source_id)
            });
            if !has_other_source {
                let mut scope_statement = transaction
                    .prepare(
                        "SELECT scope_id FROM source_scope_refs WHERE concept_id = ? AND scope_kind = 'project'",
                    )
                    .map_err(|error| MemoryError::DuckDb {
                        path: config.index_path.clone(),
                        source: error,
                    })?;
                let mut remaining_project_scopes = scope_statement
                    .query_map([&concept_id], |row| row.get(0))
                    .map_err(|error| MemoryError::DuckDb {
                        path: config.index_path.clone(),
                        source: error,
                    })?
                    .collect::<Result<BTreeSet<String>, _>>()
                    .map_err(|error| MemoryError::DuckDb {
                        path: config.index_path.clone(),
                        source: error,
                    })?;
                if remaining_project_scopes.is_empty() {
                    remaining_project_scopes = source_ids
                        .iter()
                        .filter_map(|candidate| registered_source_repositories.get(candidate))
                        .filter_map(|repository_id| config.repository_sources.get(repository_id))
                        .flat_map(|source| source.project_scope_ids.iter())
                        .cloned()
                        .collect::<BTreeSet<_>>();
                }
                scopes.retain(|scope| match &scope.kind {
                    KnowledgeScopeKind::Repository => scope.id != repository_id,
                    KnowledgeScopeKind::Project => remaining_project_scopes.contains(&scope.id),
                    KnowledgeScopeKind::ProjectSet => config
                        .default_project_set_id
                        .as_deref()
                        .is_some_and(|id| scope.id == id),
                    _ => true,
                });
                sources.retain(|source| source.repo_id.as_deref() != Some(repository_id));
                transaction
                    .execute("DELETE FROM scope_refs WHERE concept_id = ?", [&concept_id])
                    .map_err(|error| MemoryError::DuckDb {
                        path: config.index_path.clone(),
                        source: error,
                    })?;
                for scope in &scopes {
                    transaction
                        .execute(
                            "INSERT OR IGNORE INTO scope_refs (concept_id, scope_kind, scope_id, label) VALUES (?, ?, ?, ?)",
                            duckdb::params![concept_id, scope_kind_name(&scope.kind), scope.id, scope.label],
                        )
                        .map_err(|error| MemoryError::DuckDb {
                            path: config.index_path.clone(),
                            source: error,
                        })?;
                }
            }
            transaction
                .execute(
                    "UPDATE issues SET source_ids_json = ?, scope_refs_json = ?, source_refs_json = ? WHERE issue_key = ?",
                    duckdb::params![serde_json::to_string(&source_ids)?, serde_json::to_string(&scopes)?, serde_json::to_string(&sources)?, issue_key],
                )
                .map_err(|error| MemoryError::DuckDb {
                    path: config.index_path.clone(),
                    source: error,
                })?;
        }
    }
    transaction.commit().map_err(|error| MemoryError::DuckDb {
        path: config.index_path.clone(),
        source: error,
    })
}

pub fn withdraw_memory_repository_records(
    config: &MemoryConfig,
    repository_id: &str,
) -> Result<(), MemoryError> {
    let mut connection = open_index(config)?;
    migrate_index(&connection).map_err(|error| MemoryError::DuckDb {
        path: config.index_path.clone(),
        source: error,
    })?;
    let mut statement = connection
        .prepare(
            "SELECT DISTINCT issues.issue_key, issues.concept_id, issues.scope_refs_json, issues.source_refs_json FROM issues JOIN scope_refs ON scope_refs.concept_id = issues.concept_id WHERE scope_refs.scope_kind = 'repository' AND scope_refs.scope_id = ?",
        )
        .map_err(|error| MemoryError::DuckDb {
            path: config.index_path.clone(),
            source: error,
        })?;
    let rows = statement
        .query_map(duckdb::params![repository_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        })
        .map_err(|error| MemoryError::DuckDb {
            path: config.index_path.clone(),
            source: error,
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| MemoryError::DuckDb {
            path: config.index_path.clone(),
            source: error,
        })?;
    drop(statement);
    let transaction = connection.transaction().map_err(|error| MemoryError::DuckDb {
        path: config.index_path.clone(),
        source: error,
    })?;
    let registered_source_repositories = {
        let mut statement = transaction
            .prepare("SELECT source_id, repository_id FROM registered_memory_sources")
            .map_err(|error| MemoryError::DuckDb {
                path: config.index_path.clone(),
                source: error,
            })?;
        statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|error| MemoryError::DuckDb {
                path: config.index_path.clone(),
                source: error,
            })?
            .collect::<Result<BTreeMap<_, _>, _>>()
            .map_err(|error| MemoryError::DuckDb {
                path: config.index_path.clone(),
                source: error,
            })?
    };
    let remaining_project_scopes = registered_source_repositories
        .values()
        .filter(|candidate| candidate.as_str() != repository_id)
        .filter_map(|candidate| config.repository_sources.get(candidate))
        .flat_map(|source| source.project_scope_ids.iter())
        .cloned()
        .collect::<BTreeSet<_>>();
    for (issue_key, concept_id, scopes_json, sources_json) in rows {
        let scopes = serde_json::from_str::<Vec<KnowledgeScope>>(&scopes_json)
            .unwrap_or_default()
            .into_iter()
            .filter(|scope| match &scope.kind {
                KnowledgeScopeKind::Repository => scope.id != repository_id,
                KnowledgeScopeKind::Project => remaining_project_scopes.contains(&scope.id),
                KnowledgeScopeKind::ProjectSet => config
                    .default_project_set_id
                    .as_deref()
                    .is_some_and(|id| scope.id == id),
                _ => true,
            })
            .collect::<Vec<_>>();
        let sources = serde_json::from_str::<Vec<MemorySourceRef>>(&sources_json)
            .unwrap_or_default()
            .into_iter()
            .filter(|source| source.repo_id.as_deref() != Some(repository_id))
            .collect::<Vec<_>>();
        transaction
            .execute("DELETE FROM scope_refs WHERE concept_id = ?", [&concept_id])
            .map_err(|error| MemoryError::DuckDb {
                path: config.index_path.clone(),
                source: error,
            })?;
        if scopes.is_empty() {
            transaction
                .execute("DELETE FROM issues WHERE issue_key = ?", [&issue_key])
                .map_err(|error| MemoryError::DuckDb {
                    path: config.index_path.clone(),
                    source: error,
                })?;
            for table in [
                "issue_areas",
                "pull_requests",
                "changed_files",
                "checks",
                "reviews",
            ] {
                transaction
                    .execute(
                        &format!("DELETE FROM {table} WHERE issue_key = ?"),
                        [&issue_key],
                    )
                    .map_err(|error| MemoryError::DuckDb {
                        path: config.index_path.clone(),
                        source: error,
                    })?;
            }
        } else {
            transaction
                .execute(
                    "UPDATE issues SET scope_refs_json = ?, source_refs_json = ? WHERE issue_key = ?",
                    duckdb::params![serde_json::to_string(&scopes)?, serde_json::to_string(&sources)?, issue_key],
                )
                .map_err(|error| MemoryError::DuckDb {
                    path: config.index_path.clone(),
                    source: error,
                })?;
            for scope in scopes {
                transaction
                    .execute(
                        "INSERT OR IGNORE INTO scope_refs (concept_id, scope_kind, scope_id, label) VALUES (?, ?, ?, ?)",
                        duckdb::params![concept_id, scope_kind_name(&scope.kind), scope.id, scope.label],
                    )
                    .map_err(|error| MemoryError::DuckDb {
                        path: config.index_path.clone(),
                        source: error,
                    })?;
            }
        }
    }
    transaction.commit().map_err(|error| MemoryError::DuckDb {
        path: config.index_path.clone(),
        source: error,
    })
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
        "repository" => MemorySourceKind::Repository,
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

    #[test]
    fn withdraws_only_source_refs_owned_by_the_registration() {
        let root = TempDir::new().expect("memory root");
        let config = MemoryConfig::load(root.path(), None).expect("config");
        let public_source = RegisteredMemorySource {
            source_id: "github:repository:123:okf-public".to_string(),
            repository_id: "github:repository:123".to_string(),
            commit_sha: "abc123".to_string(),
            kind: MemorySourceKind::OkfBundle,
            root: root.path().join("okf-public"),
            status: MemorySourceRegistrationStatus::Registered,
            generation: "sha256:public".to_string(),
        };
        let private_source = RegisteredMemorySource {
            source_id: "github:repository:123:okf-private".to_string(),
            generation: "sha256:private".to_string(),
            root: root.path().join("okf-private"),
            ..public_source.clone()
        };
        register_memory_source(&config, &public_source).expect("public source");
        register_memory_source(&config, &private_source).expect("private source");
        let connection = open_index(&config).expect("index");
        connection
            .execute(
                "INSERT INTO issues (issue_key, title, labels_json, archive_status, capsule_path, visibility, source_hash, warning_count, docs_sync_status, body, captured_at, concept_id, scope_refs_json, source_refs_json, source_ids_json) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                duckdb::params![
                    "COE-550",
                    "Shared concept",
                    "[]",
                    "not_archived",
                    "issues/COE-550.md",
                    "private",
                    "hash",
                    0_i64,
                    "pending",
                    "body",
                    "2026-07-31T00:00:00Z",
                    "issues/COE-550",
                    r#"[{"kind":"repository","id":"github:repository:123"}]"#,
                    r#"[{"kind":"github_pr","id":"42","repo_id":"github:repository:123","registration_source_id":"github:repository:123:okf-public"},{"kind":"github_pr","id":"42","repo_id":"github:repository:123","registration_source_id":"github:repository:123:okf-private"}]"#,
                    r#"["github:repository:123:okf-public","github:repository:123:okf-private"]"#,
                ],
            )
            .expect("shared issue");
        connection
            .execute(
                "INSERT INTO scope_refs (concept_id, scope_kind, scope_id, label) VALUES ('issues/COE-550', 'repository', 'github:repository:123', NULL)",
                [],
            )
            .expect("normalized scope");

        withdraw_memory_source_records(
            &config,
            &public_source.source_id,
            "github:repository:123",
        )
            .expect("source withdrawal");

        let issue = load_indexed_issues(&config)
            .expect("issues")
            .into_iter()
            .find(|issue| issue.issue_key == "COE-550")
            .expect("shared issue remains");
        assert_eq!(issue.source_refs.len(), 1);
        assert_eq!(
            issue.source_refs[0].registration_source_id.as_deref(),
            Some(private_source.source_id.as_str())
        );
    }
}

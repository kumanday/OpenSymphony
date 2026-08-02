fn source_repository_id(
    config: &MemoryConfig,
    registered_source_repositories: &BTreeMap<String, String>,
    source_id: &str,
) -> Option<String> {
    registered_source_repositories
        .get(source_id)
        .cloned()
        .or_else(|| source_id.strip_prefix("__live_capture__:").map(str::to_string))
        .or_else(|| {
            (source_id == LIVE_CAPTURE_OWNER)
                .then(|| config.default_repository_id.clone())
                .flatten()
        })
}

impl MemoryConfig {
    pub fn with_repository_source(
        mut self,
        source: MemoryRepositorySource,
    ) -> Self {
        self.repository_sources
            .insert(source.repository_id.clone(), source);
        self
    }

    pub fn with_repository_remote_locator(
        mut self,
        repository_id: impl Into<String>,
        remote_locator: impl Into<String>,
    ) -> Self {
        self.repository_remote_locators
            .insert(repository_id.into(), remote_locator.into());
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
        withdraw_memory_repository_records(config, repository_id)?;
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
    let (registered_source_repositories, registered_source_kinds) = {
        let mut statement = transaction
            .prepare("SELECT source_id, repository_id, source_kind FROM registered_memory_sources")
            .map_err(|error| MemoryError::DuckDb {
                path: config.index_path.clone(),
                source: error,
            })?;
        statement
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
            })?
            .into_iter()
            .fold(
                (BTreeMap::new(), BTreeMap::new()),
                |(mut repositories, mut kinds), (source_id, repository_id, kind)| {
                    repositories.insert(source_id.clone(), repository_id);
                    kinds.insert(source_id, kind);
                    (repositories, kinds)
                },
            )
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
                    && source_repository_id(config, &registered_source_repositories, candidate)
                        .is_some_and(|owner| owner == repository_id)
            });
            let has_other_project_set_source = config.default_project_set_id.is_some()
                && source_ids.iter().any(|candidate| {
                    candidate != source_id
                        && source_repository_id(config, &registered_source_repositories, candidate)
                            .as_deref()
                            .and_then(|owner| config.repository_sources.get(owner))
                            .is_some_and(|source| {
                                config.project_scope_ids.is_empty()
                                    || !source
                                        .project_scope_ids
                                        .is_disjoint(&config.project_scope_ids)
                            })
                });
            sources.retain(|source| {
                source.registration_source_id.as_deref() != Some(source_id)
                    && !(source.registration_source_id.is_none() && source.id == source_id)
            });
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
                let provenance_count: i64 = transaction
                    .query_row(
                        "SELECT COUNT(*) FROM source_scope_refs WHERE concept_id = ?",
                        [&concept_id],
                        |row| row.get(0),
                    )
                    .map_err(|error| MemoryError::DuckDb {
                        path: config.index_path.clone(),
                        source: error,
                    })?;
                if provenance_count == 0 {
                    remaining_project_scopes = source_ids
                        .iter()
                        .filter_map(|candidate| {
                            source_repository_id(config, &registered_source_repositories, candidate)
                        })
                        .filter_map(|repository_id| config.repository_sources.get(&repository_id))
                        .flat_map(|source| source.project_scope_ids.iter())
                        .cloned()
                    .collect::<BTreeSet<_>>();
                }
            }
            drop(scope_statement);
            let surviving_source_scopes = {
                let mut statement = transaction
                    .prepare(
                        "SELECT source_id, scope_kind, scope_id, label FROM source_scope_refs WHERE concept_id = ?",
                    )
                    .map_err(|error| MemoryError::DuckDb {
                        path: config.index_path.clone(),
                        source: error,
                    })?;
                statement
                    .query_map([&concept_id], |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, Option<String>>(3)?,
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
                    })?
            };
            for (surviving_source_id, kind, id, label) in surviving_source_scopes {
                if source_ids.contains(&surviving_source_id)
                    && let Some(kind) = parse_scope_kind(&kind)
                {
                    let scope = KnowledgeScope { kind, id, label };
                    if !scopes.contains(&scope) {
                        scopes.push(scope);
                    }
                }
            }
            scopes.retain(|scope| match &scope.kind {
                KnowledgeScopeKind::Repository => has_other_source || scope.id != repository_id,
                KnowledgeScopeKind::Project => remaining_project_scopes.contains(&scope.id),
                KnowledgeScopeKind::ProjectSet => {
                    (has_other_source || has_other_project_set_source)
                        && config
                            .default_project_set_id
                            .as_deref()
                            .is_some_and(|id| scope.id == id)
                }
                _ => true,
            });
            if !has_other_source {
                sources.retain(|source| {
                    source.registration_source_id.is_none()
                        || source.repo_id.as_deref() != Some(repository_id)
                });
            }
            let surviving_reimport_sources = source_ids
                .iter()
                .filter(|candidate| {
                    matches!(
                        registered_source_kinds.get(*candidate).map(String::as_str),
                        Some("legacy_store" | "okf_bundle")
                    )
                })
                .cloned()
                .collect::<Vec<_>>();
            let surviving_live_owner = source_ids.iter().any(|candidate| is_live_capture_owner(candidate));
            if surviving_live_owner {
                for surviving_source_id in &surviving_reimport_sources {
                    transaction
                        .execute(
                            "UPDATE registered_memory_sources SET status = 'pending' WHERE source_id = ?",
                            [&surviving_source_id],
                        )
                        .map_err(|error| MemoryError::DuckDb {
                            path: config.index_path.clone(),
                            source: error,
                        })?;
                }
            }
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
            "SELECT DISTINCT issues.issue_key, issues.concept_id, issues.scope_refs_json, issues.source_refs_json, issues.source_ids_json FROM issues JOIN scope_refs ON scope_refs.concept_id = issues.concept_id WHERE scope_refs.scope_kind = 'repository' AND scope_refs.scope_id = ?",
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
    let (registered_source_repositories, registered_source_kinds) = {
        let mut statement = transaction
            .prepare("SELECT source_id, repository_id, source_kind FROM registered_memory_sources")
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
        (
            rows.iter()
                .map(|(source_id, repository_id, _)| (source_id.clone(), repository_id.clone()))
                .collect::<BTreeMap<_, _>>(),
            rows.into_iter()
                .map(|(source_id, _, source_kind)| (source_id, source_kind))
                .collect::<BTreeMap<_, _>>(),
        )
    };
    let withdrawn_source_ids = registered_source_repositories
        .iter()
        .filter(|(_, owner)| owner.as_str() == repository_id)
        .map(|(source_id, _)| source_id.clone())
        .collect::<Vec<_>>();
    transaction
        .execute(
            "DELETE FROM source_scope_refs WHERE source_id IN (SELECT source_id FROM registered_memory_sources WHERE repository_id = ?)",
            [repository_id],
        )
        .map_err(|error| MemoryError::DuckDb {
            path: config.index_path.clone(),
            source: error,
        })?;
    let withdrawn_live_owner = live_capture_owner(Some(repository_id));
    transaction
        .execute(
            "DELETE FROM source_scope_refs WHERE source_id = ?",
            [&withdrawn_live_owner],
        )
        .map_err(|error| MemoryError::DuckDb {
            path: config.index_path.clone(),
            source: error,
        })?;
    for (issue_key, concept_id, scopes_json, sources_json, source_ids_json) in rows {
        let mut source_ids = serde_json::from_str::<Vec<String>>(&source_ids_json).unwrap_or_default();
        let scopes = serde_json::from_str::<Vec<KnowledgeScope>>(&scopes_json).unwrap_or_default();
        let has_surviving_registered_owner = source_ids.iter().any(|source_id| {
            !is_live_capture_owner(source_id)
                && source_repository_id(config, &registered_source_repositories, source_id)
                    .is_some_and(|owner| owner != repository_id)
        });
        let legacy_live_owner_attributable = source_ids.iter().any(|source_id| source_id == LIVE_CAPTURE_OWNER)
            && scopes.iter().any(|scope| {
                scope.kind == KnowledgeScopeKind::Repository && scope.id == repository_id
            })
            && (!scopes.iter().any(|scope| {
                scope.kind == KnowledgeScopeKind::Repository && scope.id != repository_id
            }) || has_surviving_registered_owner);
        let had_legacy_live_owner = legacy_live_owner_attributable;
        source_ids.retain(|source_id| {
            !(live_capture_owner_matches_repository(source_id, repository_id)
                && (source_id != LIVE_CAPTURE_OWNER || legacy_live_owner_attributable))
                && source_repository_id(config, &registered_source_repositories, source_id)
                    .as_deref()
                    != Some(repository_id)
        });
        if legacy_live_owner_attributable {
            transaction
                .execute(
                    "DELETE FROM source_scope_refs WHERE concept_id = ? AND source_id = ?",
                    duckdb::params![concept_id, LIVE_CAPTURE_OWNER],
                )
                .map_err(|error| MemoryError::DuckDb {
                    path: config.index_path.clone(),
                    source: error,
                })?;
        } else {
            transaction
                .execute(
                    "DELETE FROM source_scope_refs WHERE concept_id = ? AND source_id = ? AND scope_kind = 'repository' AND scope_id = ?",
                    duckdb::params![concept_id, LIVE_CAPTURE_OWNER, repository_id],
                )
                .map_err(|error| MemoryError::DuckDb {
                    path: config.index_path.clone(),
                    source: error,
                })?;
        }
        for table in [
            "issue_areas",
            "pull_requests",
            "changed_files",
            "checks",
            "reviews",
        ] {
            for withdrawn_source_id in &withdrawn_source_ids {
                transaction
                    .execute(
                        &format!("DELETE FROM {table} WHERE issue_key = ? AND source_id = ?"),
                        duckdb::params![issue_key, withdrawn_source_id],
                    )
                    .map_err(|error| MemoryError::DuckDb {
                        path: config.index_path.clone(),
                        source: error,
                    })?;
            }
            transaction
                .execute(
                    &format!("DELETE FROM {table} WHERE issue_key = ? AND source_id = ?"),
                    duckdb::params![issue_key, withdrawn_live_owner.clone()],
                )
                .map_err(|error| MemoryError::DuckDb {
                    path: config.index_path.clone(),
                    source: error,
                })?;
            if had_legacy_live_owner {
                transaction
                    .execute(
                        &format!("DELETE FROM {table} WHERE issue_key = ? AND source_id IS NULL"),
                        [&issue_key],
                    )
                    .map_err(|error| MemoryError::DuckDb {
                        path: config.index_path.clone(),
                        source: error,
                    })?;
            }
        }
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
            continue;
        }
        let surviving_reimport_sources = source_ids
            .iter()
            .filter(|candidate| {
                matches!(
                    registered_source_kinds.get(*candidate).map(String::as_str),
                    Some("legacy_store" | "okf_bundle")
                )
            })
            .cloned()
            .collect::<Vec<_>>();
        if !surviving_reimport_sources.is_empty() {
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
            for surviving_source_id in surviving_reimport_sources {
                transaction
                    .execute(
                        "UPDATE registered_memory_sources SET status = 'pending' WHERE source_id = ?",
                        [&surviving_source_id],
                    )
                    .map_err(|error| MemoryError::DuckDb {
                        path: config.index_path.clone(),
                        source: error,
                    })?;
            }
            continue;
        }
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
            let provenance_count: i64 = transaction
                .query_row(
                    "SELECT COUNT(*) FROM source_scope_refs WHERE concept_id = ?",
                    [&concept_id],
                    |row| row.get(0),
                )
                .map_err(|error| MemoryError::DuckDb {
                    path: config.index_path.clone(),
                    source: error,
                })?;
            if provenance_count == 0 {
                remaining_project_scopes = source_ids
                    .iter()
                    .filter_map(|candidate| {
                        source_repository_id(config, &registered_source_repositories, candidate)
                    })
                    .filter_map(|repository_id| config.repository_sources.get(&repository_id))
                    .flat_map(|source| source.project_scope_ids.iter())
                    .cloned()
                    .collect::<BTreeSet<_>>();
            }
        }
        let mut scopes = serde_json::from_str::<Vec<KnowledgeScope>>(&scopes_json)
            .unwrap_or_default()
            .into_iter()
            .filter(|scope| {
                scope.kind != KnowledgeScopeKind::ProjectSet
                    || source_ids.iter().any(|candidate| {
                        source_repository_id(config, &registered_source_repositories, candidate)
                            .as_deref()
                            .and_then(|owner| config.repository_sources.get(owner))
                            .is_some_and(|source| {
                                config.default_project_set_id.as_deref()
                                    == Some(scope.id.as_str())
                                    && (config.project_scope_ids.is_empty()
                                        || !source
                                            .project_scope_ids
                                            .is_disjoint(&config.project_scope_ids))
                            })
                    })
            })
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
        let mut provenance_scope_statement = transaction
            .prepare(
                "SELECT source_id, scope_kind, scope_id, label FROM source_scope_refs WHERE concept_id = ?",
            )
            .map_err(|error| MemoryError::DuckDb {
                path: config.index_path.clone(),
                source: error,
            })?;
        let provenance_scopes = provenance_scope_statement
            .query_map([&concept_id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
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
        for (source_id, kind, id, label) in provenance_scopes {
            if source_ids.contains(&source_id)
                && let Some(kind) = parse_scope_kind(&kind)
            {
                let scope = KnowledgeScope { kind, id, label };
                if !scopes.contains(&scope) {
                    scopes.push(scope);
                }
            }
        }
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
                    "UPDATE issues SET source_ids_json = ?, scope_refs_json = ?, source_refs_json = ? WHERE issue_key = ?",
                    duckdb::params![serde_json::to_string(&source_ids)?, serde_json::to_string(&scopes)?, serde_json::to_string(&sources)?, issue_key],
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

    #[test]
    fn withdraws_project_scopes_from_same_repository_source_provenance() {
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
                    "COE-551",
                    "Shared concept",
                    "[]",
                    "not_archived",
                    "issues/COE-551.md",
                    "private",
                    "hash",
                    0_i64,
                    "pending",
                    "body",
                    "2026-07-31T00:00:00Z",
                    "issues/COE-551",
                    r#"[{"kind":"repository","id":"github:repository:123"},{"kind":"repository","id":"github:repository:other"},{"kind":"project","id":"project-public"},{"kind":"project","id":"project-private"}]"#,
                    "[]",
                    r#"["github:repository:123:okf-public","github:repository:123:okf-private"]"#,
                ],
            )
            .expect("shared issue");
        connection
            .execute(
                "INSERT INTO scope_refs (concept_id, scope_kind, scope_id, label) VALUES ('issues/COE-551', 'repository', 'github:repository:123', NULL), ('issues/COE-551', 'repository', 'github:repository:other', NULL), ('issues/COE-551', 'project', 'project-public', NULL), ('issues/COE-551', 'project', 'project-private', NULL)",
                [],
            )
            .expect("normalized scopes");
        connection
            .execute(
                "INSERT INTO source_scope_refs (concept_id, source_id, scope_kind, scope_id, label) VALUES ('issues/COE-551', 'github:repository:123:okf-public', 'project', 'project-public', NULL), ('issues/COE-551', 'github:repository:123:okf-private', 'project', 'project-private', NULL)",
                [],
            )
            .expect("source scopes");

        withdraw_memory_source_records(
            &config,
            &public_source.source_id,
            &public_source.repository_id,
        )
        .expect("source withdrawal");

        let issue = load_indexed_issues(&config)
            .expect("issues")
            .into_iter()
            .find(|issue| issue.issue_key == "COE-551")
            .expect("shared issue remains");
        assert!(!issue
            .scope_refs
            .iter()
            .any(|scope| scope.id == "project-public"));
        assert!(issue.scope_refs.iter().any(|scope| {
            scope.kind == KnowledgeScopeKind::Repository && scope.id == "github:repository:123"
        }));
        assert!(issue.scope_refs.iter().any(|scope| {
            scope.kind == KnowledgeScopeKind::Repository
                && scope.id == "github:repository:other"
        }));
        assert!(issue
            .scope_refs
            .iter()
            .any(|scope| scope.id == "project-private"));
        let private_scope_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM source_scope_refs WHERE concept_id = 'issues/COE-551' AND source_id = 'github:repository:123:okf-private'",
                [],
                |row| row.get(0),
            )
            .expect("remaining source scopes");
        assert_eq!(private_scope_count, 1);
    }

    #[test]
    fn withdraws_repository_using_surviving_source_scope_provenance() {
        let root = TempDir::new().expect("memory root");
        let config = MemoryConfig::load(root.path(), None).expect("config");
        let source_a = RegisteredMemorySource {
            source_id: "github:repository:a:okf".to_string(),
            repository_id: "github:repository:a".to_string(),
            commit_sha: "abc123".to_string(),
            kind: MemorySourceKind::OkfBundle,
            root: root.path().join("okf-a"),
            status: MemorySourceRegistrationStatus::Registered,
            generation: "sha256:a".to_string(),
        };
        let mut source_b = source_a.clone();
        source_b.source_id = "github:repository:b:okf".to_string();
        source_b.repository_id = "github:repository:b".to_string();
        source_b.root = root.path().join("okf-b");
        source_b.generation = "sha256:b".to_string();
        source_b.kind = MemorySourceKind::Repository;
        register_memory_source(&config, &source_a).expect("source a");
        register_memory_source(&config, &source_b).expect("source b");
        let connection = open_index(&config).expect("index");
        connection
            .execute(
                "INSERT INTO issues (issue_key, title, labels_json, archive_status, capsule_path, visibility, source_hash, warning_count, docs_sync_status, body, captured_at, concept_id, scope_refs_json, source_refs_json, source_ids_json) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                duckdb::params![
                    "COE-552",
                    "Cross repository concept",
                    "[]",
                    "not_archived",
                    "issues/COE-552.md",
                    "private",
                    "hash",
                    0_i64,
                    "pending",
                    "body",
                    "2026-07-31T00:00:00Z",
                    "issues/COE-552",
                    r#"[{"kind":"repository","id":"github:repository:a"},{"kind":"repository","id":"github:repository:b"},{"kind":"project","id":"project-a"},{"kind":"project","id":"project-b"}]"#,
                    r#"[{"kind":"legacy_store","id":"a","repo_id":"github:repository:a"},{"kind":"repository","id":"b","repo_id":"github:repository:b"}]"#,
                    r#"["github:repository:a:okf","github:repository:b:okf"]"#,
                ],
            )
            .expect("shared issue");
        connection
            .execute(
                "INSERT INTO scope_refs (concept_id, scope_kind, scope_id, label) VALUES ('issues/COE-552', 'repository', 'github:repository:a', NULL), ('issues/COE-552', 'repository', 'github:repository:b', NULL), ('issues/COE-552', 'project', 'project-a', NULL), ('issues/COE-552', 'project', 'project-b', NULL)",
                [],
            )
            .expect("normalized scopes");
        connection
            .execute(
                "INSERT INTO source_scope_refs (concept_id, source_id, scope_kind, scope_id, label) VALUES ('issues/COE-552', 'github:repository:a:okf', 'project', 'project-a', NULL), ('issues/COE-552', 'github:repository:b:okf', 'project', 'project-b', NULL)",
                [],
            )
            .expect("source scopes");
        withdraw_memory_repository_records(&config, &source_a.repository_id)
            .expect("repository withdrawal");

        let issue = load_indexed_issues(&config)
            .expect("issues")
            .into_iter()
            .find(|issue| issue.issue_key == "COE-552")
            .expect("remaining issue");
        assert!(!issue.scope_refs.iter().any(|scope| {
            scope.kind == KnowledgeScopeKind::Project && scope.id == "project-a"
        }));
        assert!(issue.scope_refs.iter().any(|scope| {
            scope.kind == KnowledgeScopeKind::Project && scope.id == "project-b"
        }));
    }

    #[test]
    fn withdraws_live_capture_without_source_registration_ownership() {
        let root = TempDir::new().expect("memory root");
        let mut config = MemoryConfig::load(root.path(), None).expect("config");
        config.default_project_set_id = Some("set-a".to_string());
        let source = RegisteredMemorySource {
            source_id: "github:repository:a:repository".to_string(),
            repository_id: "github:repository:a".to_string(),
            commit_sha: "abc123".to_string(),
            kind: MemorySourceKind::Repository,
            root: root.path().join("repo-a"),
            status: MemorySourceRegistrationStatus::Registered,
            generation: "sha256:repo".to_string(),
        };
        register_memory_source(&config, &source).expect("repository source");
        let connection = open_index(&config).expect("index");
        connection
            .execute(
                "INSERT INTO issues (issue_key, title, labels_json, archive_status, capsule_path, visibility, source_hash, warning_count, docs_sync_status, body, captured_at, concept_id, scope_refs_json, source_refs_json, source_ids_json) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                duckdb::params![
                    "COE-553",
                    "Live capture",
                    "[]",
                    "not_archived",
                    "issues/COE-553.md",
                    "private",
                    "hash",
                    0_i64,
                    "pending",
                    "body",
                    "2026-07-31T00:00:00Z",
                    "issues/COE-553",
                    r#"[{"kind":"repository","id":"github:repository:a"},{"kind":"project","id":"project-a"},{"kind":"project_set","id":"set-a"}]"#,
                    "[]",
                    r#"["__live_capture__"]"#,
                ],
            )
            .expect("live capture");
        connection
            .execute(
                "INSERT INTO scope_refs (concept_id, scope_kind, scope_id, label) VALUES ('issues/COE-553', 'repository', 'github:repository:a', NULL), ('issues/COE-553', 'project', 'project-a', NULL), ('issues/COE-553', 'project_set', 'set-a', NULL)",
                [],
            )
            .expect("normalized scopes");

        withdraw_memory_repository_records(&config, "github:repository:a")
            .expect("repository withdrawal");

        assert!(load_indexed_issues(&config)
            .expect("issues")
            .into_iter()
            .all(|issue| issue.issue_key != "COE-553"));
    }

    #[test]
    fn withdrawing_registered_source_preserves_surviving_live_capture_payload() {
        let root = TempDir::new().expect("memory root");
        let config = MemoryConfig::load(root.path(), None).expect("config");
        let source = RegisteredMemorySource {
            source_id: "github:repository:a:repository".to_string(),
            repository_id: "github:repository:a".to_string(),
            commit_sha: "abc123".to_string(),
            kind: MemorySourceKind::Repository,
            root: root.path().join("repo-a"),
            status: MemorySourceRegistrationStatus::Registered,
            generation: "sha256:repo".to_string(),
        };
        register_memory_source(&config, &source).expect("repository source");
        let connection = open_index(&config).expect("index");
        connection
            .execute(
                "INSERT INTO issues (issue_key, title, labels_json, archive_status, capsule_path, visibility, source_hash, warning_count, docs_sync_status, body, captured_at, concept_id, scope_refs_json, source_refs_json, source_ids_json) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                duckdb::params![
                    "COE-555",
                    "Live capture refreshed from source",
                    "[]",
                    "not_archived",
                    "issues/COE-555.md",
                    "private",
                    "hash",
                    0_i64,
                    "pending",
                    "body",
                    "2026-07-31T00:00:00Z",
                    "issues/COE-555",
                    r#"[{"kind":"repository","id":"github:repository:a"},{"kind":"repository","id":"github:repository:b"}]"#,
                    r#"[{"kind":"github_pr","id":"42","repo_id":"github:repository:b"},{"kind":"legacy_store","id":"source","repo_id":"github:repository:a","registration_source_id":"github:repository:a:repository"}]"#,
                    r#"["__live_capture__:github:repository:b","github:repository:a:repository"]"#,
                ],
            )
            .expect("live capture");
        connection
            .execute(
                "INSERT INTO scope_refs (concept_id, scope_kind, scope_id, label) VALUES ('issues/COE-555', 'repository', 'github:repository:a', NULL), ('issues/COE-555', 'repository', 'github:repository:b', NULL)",
                [],
            )
            .expect("normalized scope");
        connection
            .execute(
                "INSERT INTO source_scope_refs (concept_id, source_id, scope_kind, scope_id, label) VALUES ('issues/COE-555', '__live_capture__:github:repository:b', 'repository', 'github:repository:b', NULL)",
                [],
            )
            .expect("surviving live scope");
        connection
            .execute(
                "INSERT INTO issue_areas (issue_key, area, source_id) VALUES ('COE-555', 'area-a', 'github:repository:a:repository'), ('COE-555', 'area-b', '__live_capture__:github:repository:b')",
                [],
            )
            .expect("source-owned areas");

        withdraw_memory_source_records(
            &config,
            &source.source_id,
            &source.repository_id,
        )
        .expect("source withdrawal");

        let issue = load_indexed_issues(&config)
            .expect("issues")
            .into_iter()
            .find(|issue| issue.issue_key == "COE-555")
            .expect("surviving live issue");
        assert_eq!(issue.title, "Live capture refreshed from source");
        assert_eq!(issue.body, "body");
        assert_eq!(issue.areas(), vec!["area-b"]);
        assert_eq!(
            issue
                .scope_refs
                .iter()
                .map(|scope| scope.id.as_str())
                .collect::<Vec<_>>(),
            vec!["github:repository:b"]
        );
    }

    #[test]
    fn withdrawing_repository_removes_live_owner_from_surviving_source() {
        let root = TempDir::new().expect("memory root");
        let config = MemoryConfig::load(root.path(), None).expect("config");
        let source_a = RegisteredMemorySource {
            source_id: "github:repository:a:repository".to_string(),
            repository_id: "github:repository:a".to_string(),
            commit_sha: "abc123".to_string(),
            kind: MemorySourceKind::Repository,
            root: root.path().join("repo-a"),
            status: MemorySourceRegistrationStatus::Registered,
            generation: "sha256:repo-a".to_string(),
        };
        let source_b = RegisteredMemorySource {
            source_id: "github:repository:b:repository".to_string(),
            repository_id: "github:repository:b".to_string(),
            root: root.path().join("repo-b"),
            ..source_a.clone()
        };
        register_memory_source(&config, &source_a).expect("source a");
        register_memory_source(&config, &source_b).expect("source b");
        let connection = open_index(&config).expect("index");
        connection
            .execute(
                "INSERT INTO issues (issue_key, title, labels_json, archive_status, capsule_path, visibility, source_hash, warning_count, docs_sync_status, body, captured_at, concept_id, scope_refs_json, source_refs_json, source_ids_json) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                duckdb::params![
                    "COE-556",
                    "Shared live capture",
                    "[]",
                    "not_archived",
                    "issues/COE-556.md",
                    "private",
                    "hash",
                    0_i64,
                    "pending",
                    "body",
                    "2026-08-01T00:00:00Z",
                    "issues/COE-556",
                    r#"[{"kind":"repository","id":"github:repository:a"},{"kind":"repository","id":"github:repository:b"}]"#,
                    "[]",
                    r#"["__live_capture__","github:repository:b:repository"]"#,
                ],
            )
            .expect("shared issue");
        connection
            .execute(
                "INSERT INTO scope_refs (concept_id, scope_kind, scope_id, label) VALUES ('issues/COE-556', 'repository', 'github:repository:a', NULL), ('issues/COE-556', 'repository', 'github:repository:b', NULL)",
                [],
            )
            .expect("normalized scopes");
        drop(connection);

        withdraw_memory_repository_records(&config, &source_a.repository_id)
            .expect("repository withdrawal");

        let connection = open_existing_index_read_only(&config)
            .expect("index should reopen")
            .expect("index exists");
        let source_ids: String = connection
            .query_row(
                "SELECT source_ids_json FROM issues WHERE issue_key = 'COE-556'",
                [],
                |row| row.get(0),
            )
            .expect("surviving issue");
        assert_eq!(source_ids, r#"["github:repository:b:repository"]"#);
        let remaining_a_scopes: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM scope_refs WHERE concept_id = 'issues/COE-556' AND scope_id = 'github:repository:a'",
                [],
                |row| row.get(0),
            )
            .expect("repository a scopes");
        assert_eq!(remaining_a_scopes, 0);
    }

    #[test]
    fn withdrawing_repository_preserves_surviving_live_capture_payload() {
        let root = TempDir::new().expect("memory root");
        let config = MemoryConfig::load(root.path(), None).expect("config");
        let source_a = RegisteredMemorySource {
            source_id: "github:repository:a:repository".to_string(),
            repository_id: "github:repository:a".to_string(),
            commit_sha: "abc123".to_string(),
            kind: MemorySourceKind::Repository,
            root: root.path().join("repo-a"),
            status: MemorySourceRegistrationStatus::Registered,
            generation: "sha256:repo-a".to_string(),
        };
        let source_b = RegisteredMemorySource {
            source_id: "github:repository:b:repository".to_string(),
            repository_id: "github:repository:b".to_string(),
            root: root.path().join("repo-b"),
            ..source_a.clone()
        };
        register_memory_source(&config, &source_a).expect("source a");
        register_memory_source(&config, &source_b).expect("source b");
        let connection = open_index(&config).expect("index");
        connection
            .execute(
                "INSERT INTO issues (issue_key, title, labels_json, archive_status, capsule_path, visibility, source_hash, warning_count, docs_sync_status, body, captured_at, concept_id, scope_refs_json, source_refs_json, source_ids_json) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                duckdb::params![
                    "COE-562",
                    "Live survivor",
                    "[]",
                    "not_archived",
                    "issues/COE-562.md",
                    "private",
                    "hash",
                    0_i64,
                    "pending",
                    "Live capture body",
                    "2026-08-01T00:00:00Z",
                    "issues/COE-562",
                    r#"[{"kind":"repository","id":"github:repository:a"},{"kind":"repository","id":"github:repository:b"}]"#,
                    r#"[{"kind":"legacy_store","id":"source-a","repo_id":"github:repository:a","registration_source_id":"github:repository:a:repository"},{"kind":"linear_issue","id":"COE-562","repo_id":"github:repository:b"}]"#,
                    r#"["github:repository:a:repository","__live_capture__:github:repository:b"]"#,
                ],
            )
            .expect("shared issue");
        connection
            .execute(
                "INSERT INTO scope_refs (concept_id, scope_kind, scope_id, label) VALUES ('issues/COE-562', 'repository', 'github:repository:a', NULL), ('issues/COE-562', 'repository', 'github:repository:b', NULL)",
                [],
            )
            .expect("normalized scopes");
        connection
            .execute(
                "INSERT INTO source_scope_refs (concept_id, source_id, scope_kind, scope_id, label) VALUES ('issues/COE-562', 'github:repository:a:repository', 'repository', 'github:repository:a', NULL), ('issues/COE-562', '__live_capture__:github:repository:b', 'repository', 'github:repository:b', NULL)",
                [],
            )
            .expect("source scopes");
        connection
            .execute(
                "INSERT INTO issue_areas (issue_key, area, source_id) VALUES ('COE-562', 'area-a', 'github:repository:a:repository'), ('COE-562', 'area-b', '__live_capture__:github:repository:b')",
                [],
            )
            .expect("source-owned areas");
        drop(connection);

        withdraw_memory_repository_records(&config, &source_a.repository_id)
            .expect("repository withdrawal");

        let issue = load_indexed_issues(&config)
            .expect("issues")
            .into_iter()
            .find(|issue| issue.issue_key == "COE-562")
            .expect("live-owned issue survives");
        let connection = open_existing_index_read_only(&config)
            .expect("index should reopen")
            .expect("index exists");
        let source_ids: String = connection
            .query_row(
                "SELECT source_ids_json FROM issues WHERE issue_key = 'COE-562'",
                [],
                |row| row.get(0),
            )
            .expect("surviving source ids");
        assert_eq!(issue.title, "Live survivor");
        assert_eq!(issue.body, "Live capture body");
        assert_eq!(source_ids, r#"["__live_capture__:github:repository:b"]"#);
        assert_eq!(
            issue
                .scope_refs
                .iter()
                .map(|scope| scope.id.as_str())
                .collect::<Vec<_>>(),
            vec!["github:repository:b"]
        );
        assert_eq!(issue.areas(), vec!["area-b"]);
    }

    #[test]
    fn withdrawing_repository_preserves_other_live_owner_relations() {
        let root = TempDir::new().expect("memory root");
        let config = MemoryConfig::load(root.path(), None).expect("config");
        let connection = open_index(&config).expect("index");
        migrate_index(&connection).expect("schema");
        connection
            .execute(
                "INSERT INTO issues (issue_key, title, labels_json, archive_status, capsule_path, visibility, source_hash, warning_count, docs_sync_status, body, captured_at, concept_id, scope_refs_json, source_refs_json, source_ids_json) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                duckdb::params![
                    "COE-557",
                    "Cross repository live capture",
                    "[]",
                    "not_archived",
                    "issues/COE-557.md",
                    "private",
                    "hash",
                    0_i64,
                    "pending",
                    "body",
                    "2026-08-01T00:00:00Z",
                    "issues/COE-557",
                    r#"[{"kind":"repository","id":"repo-a"},{"kind":"repository","id":"repo-b"}]"#,
                    "[]",
                    r#"["__live_capture__:repo-a","__live_capture__:repo-b"]"#,
                ],
            )
            .expect("shared issue");
        connection
            .execute(
                "INSERT INTO scope_refs (concept_id, scope_kind, scope_id, label) VALUES ('issues/COE-557', 'repository', 'repo-a', NULL), ('issues/COE-557', 'repository', 'repo-b', NULL)",
                [],
            )
            .expect("normalized scopes");
        connection
            .execute(
                "INSERT INTO issue_areas (issue_key, area, source_id) VALUES ('COE-557', 'area-a', '__live_capture__:repo-a'), ('COE-557', 'area-b', '__live_capture__:repo-b')",
                [],
            )
            .expect("live relation rows");
        drop(connection);

        withdraw_memory_repository_records(&config, "repo-a").expect("repository withdrawal");

        let connection = open_existing_index_read_only(&config)
            .expect("index should reopen")
            .expect("index exists");
        let source_ids: String = connection
            .query_row(
                "SELECT source_ids_json FROM issues WHERE issue_key = 'COE-557'",
                [],
                |row| row.get(0),
            )
            .expect("surviving issue");
        assert_eq!(source_ids, r#"["__live_capture__:repo-b"]"#);
        let remaining_area_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM issue_areas WHERE issue_key = 'COE-557' AND source_id = '__live_capture__:repo-b'",
                [],
                |row| row.get(0),
            )
            .expect("remaining live relation");
        assert_eq!(remaining_area_count, 1);
        let withdrawn_area_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM issue_areas WHERE issue_key = 'COE-557' AND source_id = '__live_capture__:repo-a'",
                [],
                |row| row.get(0),
            )
            .expect("withdrawn live relation");
        assert_eq!(withdrawn_area_count, 0);
    }

    #[test]
    fn withdrawing_repository_preserves_unqualified_sibling_live_owner() {
        let root = TempDir::new().expect("memory root");
        let config = MemoryConfig::load(root.path(), None).expect("config");
        let connection = open_index(&config).expect("index");
        migrate_index(&connection).expect("schema");
        connection
            .execute(
                "INSERT INTO issues (issue_key, title, labels_json, archive_status, capsule_path, visibility, source_hash, warning_count, docs_sync_status, body, captured_at, concept_id, scope_refs_json, source_refs_json, source_ids_json) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                duckdb::params![
                    "COE-558",
                    "Shared legacy live capture",
                    "[]",
                    "not_archived",
                    "issues/COE-558.md",
                    "private",
                    "hash",
                    0_i64,
                    "pending",
                    "body",
                    "2026-08-01T00:00:00Z",
                    "issues/COE-558",
                    r#"[{"kind":"repository","id":"repo-a"},{"kind":"repository","id":"repo-b"}]"#,
                    "[]",
                    r#"["__live_capture__"]"#,
                ],
            )
            .expect("shared issue");
        connection
            .execute(
                "INSERT INTO scope_refs (concept_id, scope_kind, scope_id, label) VALUES ('issues/COE-558', 'repository', 'repo-a', NULL), ('issues/COE-558', 'repository', 'repo-b', NULL)",
                [],
            )
            .expect("normalized scopes");
        connection
            .execute(
                "INSERT INTO source_scope_refs (concept_id, source_id, scope_kind, scope_id, label) VALUES ('issues/COE-558', '__live_capture__', 'repository', 'repo-a', NULL), ('issues/COE-558', '__live_capture__', 'repository', 'repo-b', NULL)",
                [],
            )
            .expect("live source scopes");
        drop(connection);

        withdraw_memory_repository_records(&config, "repo-a").expect("repository withdrawal");

        let connection = open_existing_index_read_only(&config)
            .expect("index should reopen")
            .expect("index exists");
        let source_ids: String = connection
            .query_row(
                "SELECT source_ids_json FROM issues WHERE issue_key = 'COE-558'",
                [],
                |row| row.get(0),
            )
            .expect("surviving issue");
        assert_eq!(source_ids, r#"["__live_capture__"]"#);
        let remaining_b_scope_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM source_scope_refs WHERE concept_id = 'issues/COE-558' AND source_id = '__live_capture__' AND scope_id = 'repo-b'",
                [],
                |row| row.get(0),
            )
            .expect("surviving source scope");
        assert_eq!(remaining_b_scope_count, 1);
        let withdrawn_a_scope_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM source_scope_refs WHERE concept_id = 'issues/COE-558' AND source_id = '__live_capture__' AND scope_id = 'repo-a'",
                [],
                |row| row.get(0),
            )
            .expect("withdrawn source scope");
        assert_eq!(withdrawn_a_scope_count, 0);
    }

    #[test]
    fn withdraws_source_preserves_project_set_scope_for_sibling_source() {
        let root = TempDir::new().expect("memory root");
        let mut config = MemoryConfig::load(root.path(), None).expect("config");
        config.default_project_set_id = Some("set-a".to_string());
        let source_a = RegisteredMemorySource {
            source_id: "github:repository:a:public".to_string(),
            repository_id: "github:repository:a".to_string(),
            commit_sha: "abc123".to_string(),
            kind: MemorySourceKind::OkfBundle,
            root: root.path().join("public"),
            status: MemorySourceRegistrationStatus::Registered,
            generation: "sha256:public".to_string(),
        };
        let source_b = RegisteredMemorySource {
            source_id: "github:repository:a:private".to_string(),
            root: root.path().join("private"),
            generation: "sha256:private".to_string(),
            ..source_a.clone()
        };
        register_memory_source(&config, &source_a).expect("public source");
        register_memory_source(&config, &source_b).expect("private source");
        let connection = open_index(&config).expect("index");
        connection
            .execute(
                "INSERT INTO issues (issue_key, title, labels_json, archive_status, capsule_path, visibility, source_hash, warning_count, docs_sync_status, body, captured_at, concept_id, scope_refs_json, source_refs_json, source_ids_json) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                duckdb::params![
                    "COE-554",
                    "Project set concept",
                    "[]",
                    "not_archived",
                    "issues/COE-554.md",
                    "private",
                    "hash",
                    0_i64,
                    "pending",
                    "body",
                    "2026-07-31T00:00:00Z",
                    "issues/COE-554",
                    r#"[{"kind":"repository","id":"github:repository:a"},{"kind":"project","id":"project-b"},{"kind":"project_set","id":"set-a"}]"#,
                    "[]",
                    r#"["github:repository:a:public","github:repository:a:private"]"#,
                ],
            )
            .expect("shared issue");
        connection
            .execute(
                "INSERT INTO scope_refs (concept_id, scope_kind, scope_id, label) VALUES ('issues/COE-554', 'repository', 'github:repository:a', NULL), ('issues/COE-554', 'project_set', 'set-a', NULL)",
                [],
            )
            .expect("normalized scopes");

        withdraw_memory_source_records(
            &config,
            &source_a.source_id,
            &source_a.repository_id,
        )
        .expect("source withdrawal");

        let issue = load_indexed_issues(&config)
            .expect("issues")
            .into_iter()
            .find(|issue| issue.issue_key == "COE-554")
            .expect("remaining issue");
        assert!(issue.scope_refs.iter().any(|scope| {
            scope.kind == KnowledgeScopeKind::ProjectSet && scope.id == "set-a"
        }));
    }

    #[test]
    fn withdraws_source_preserves_live_payload_for_cross_repository_owner() {
        let root = TempDir::new().expect("memory root");
        let mut config = MemoryConfig::load(root.path(), None).expect("config");
        config.default_project_set_id = Some("set-a".to_string());
        config.project_scope_ids = BTreeSet::from(["project-b".to_string()]);
        for (repository_id, project_id) in [("github:repository:a", "project-a"), ("github:repository:b", "project-b")] {
            config.repository_sources.insert(
                repository_id.to_string(),
                MemoryRepositorySource {
                    repository_id: repository_id.to_string(),
                    root: root.path().join(repository_id.replace(':', "-")),
                    commit_sha: None,
                    project_scope_ids: BTreeSet::from([project_id.to_string()]),
                    target_branch: None,
                },
            );
        }
        let source_a = RegisteredMemorySource {
            source_id: "github:repository:a:public".to_string(),
            repository_id: "github:repository:a".to_string(),
            commit_sha: "abc123".to_string(),
            kind: MemorySourceKind::OkfBundle,
            root: root.path().join("source-a"),
            status: MemorySourceRegistrationStatus::Registered,
            generation: "sha256:source-a".to_string(),
        };
        let source_b = RegisteredMemorySource {
            source_id: "github:repository:b:public".to_string(),
            repository_id: "github:repository:b".to_string(),
            root: root.path().join("source-b"),
            ..source_a.clone()
        };
        register_memory_source(&config, &source_a).expect("source a");
        register_memory_source(&config, &source_b).expect("source b");
        let connection = open_index(&config).expect("index");
        connection
            .execute(
                "INSERT INTO issues (issue_key, title, labels_json, archive_status, capsule_path, visibility, source_hash, warning_count, docs_sync_status, body, captured_at, concept_id, scope_refs_json, source_refs_json, source_ids_json) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                duckdb::params![
                    "COE-555",
                    "Cross repository project set",
                    "[]",
                    "not_archived",
                    "issues/COE-555.md",
                    "private",
                    "hash",
                    0_i64,
                    "pending",
                    "body",
                    "2026-07-31T00:00:00Z",
                    "issues/COE-555",
                    r#"[{"kind":"repository","id":"github:repository:a"},{"kind":"repository","id":"github:repository:b"},{"kind":"project","id":"project-b"},{"kind":"project_set","id":"set-a"}]"#,
                    "[]",
                    r#"["github:repository:a:public","__live_capture__:github:repository:b"]"#,
                ],
            )
            .expect("shared issue");
        connection
            .execute(
                "INSERT INTO scope_refs (concept_id, scope_kind, scope_id, label) VALUES ('issues/COE-555', 'repository', 'github:repository:a', NULL), ('issues/COE-555', 'repository', 'github:repository:b', NULL), ('issues/COE-555', 'project', 'project-b', NULL), ('issues/COE-555', 'project_set', 'set-a', NULL)",
                [],
            )
            .expect("normalized scopes");
        connection
            .execute(
                "INSERT INTO source_scope_refs (concept_id, source_id, scope_kind, scope_id, label) VALUES ('issues/unrelated', 'github:repository:b:public', 'project', 'project-b', NULL), ('issues/COE-555', '__live_capture__:github:repository:b', 'repository', 'github:repository:b', NULL), ('issues/COE-555', '__live_capture__:github:repository:b', 'project', 'project-b', NULL)",
                [],
            )
            .expect("unrelated source scope");
        drop(connection);

        withdraw_memory_source_records(
            &config,
            &source_a.source_id,
            &source_a.repository_id,
        )
        .expect("source withdrawal");

        let issue = load_indexed_issues(&config)
            .expect("issues")
            .into_iter()
            .find(|issue| issue.issue_key == "COE-555")
            .expect("surviving live issue");
        assert_eq!(issue.body, "body");
        assert_eq!(
            issue
                .scope_refs
                .iter()
                .map(|scope| scope.id.as_str())
                .collect::<Vec<_>>(),
            vec!["github:repository:b", "project-b", "set-a"]
        );
    }

    #[test]
    fn withdrawing_live_owner_removes_payload_until_surviving_store_reimports() {
        let root = TempDir::new().expect("memory root");
        let config = MemoryConfig::load(root.path(), None).expect("config");
        let surviving_source = RegisteredMemorySource {
            source_id: "github:repository:b:legacy_store".to_string(),
            repository_id: "github:repository:b".to_string(),
            commit_sha: "def456".to_string(),
            kind: MemorySourceKind::LegacyStore,
            root: root.path().join("legacy-b"),
            status: MemorySourceRegistrationStatus::Registered,
            generation: "sha256:legacy-b".to_string(),
        };
        register_memory_source(&config, &surviving_source).expect("surviving source");
        let connection = open_index(&config).expect("index");
        connection
            .execute(
                "INSERT INTO issues (issue_key, title, labels_json, archive_status, capsule_path, visibility, source_hash, warning_count, docs_sync_status, body, captured_at, concept_id, scope_refs_json, source_refs_json, source_ids_json) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                duckdb::params![
                    "COE-558",
                    "Live payload",
                    "[]",
                    "not_archived",
                    "issues/COE-558.md",
                    "private",
                    "hash",
                    0_i64,
                    "pending",
                    "captured by repository A",
                    "2026-07-31T00:00:00Z",
                    "issues/COE-558",
                    r#"[{"kind":"repository","id":"github:repository:a"},{"kind":"repository","id":"github:repository:b"}]"#,
                    "[]",
                    r#"["__live_capture__:github:repository:a","github:repository:b:legacy_store"]"#,
                ],
            )
            .expect("shared issue");
        connection
            .execute(
                "INSERT INTO scope_refs (concept_id, scope_kind, scope_id, label) VALUES ('issues/COE-558', 'repository', 'github:repository:a', NULL), ('issues/COE-558', 'repository', 'github:repository:b', NULL)",
                [],
            )
            .expect("normalized scopes");
        drop(connection);

        withdraw_memory_repository_records(&config, "github:repository:a")
            .expect("repository withdrawal");

        assert!(
            load_indexed_issues(&config)
                .expect("issues")
                .into_iter()
                .all(|issue| issue.issue_key != "COE-558")
        );
        assert_eq!(
            registered_memory_sources(&config)
                .expect("registered sources")
                .into_iter()
                .find(|source| source.source_id == surviving_source.source_id)
                .expect("surviving source")
                .status,
            MemorySourceRegistrationStatus::Pending
        );
    }
}

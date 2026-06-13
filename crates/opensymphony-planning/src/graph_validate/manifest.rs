//! On-disk validator for `docs/tasks/task-package.yaml`.
//!
//! The validator reads the task package manifest plus each declared task
//! file and emits a [`ManifestValidationResult`] that captures the same
//! five error classes the legacy Python converter already exposes:
//!
//! - missing task files
//! - unknown milestones (declared on a task but absent from the manifest)
//! - unknown dependencies (declared in `blockedBy` but absent from the
//!   manifest's `tasks` list)
//! - creation-order cycles (Kahn-style topological check)
//! - self-blocks (a task declaring itself in `blockedBy`)
//! - duplicate task IDs in the manifest
//!
//! Findings are surfaced as separate vector fields so the planning-session
//! API can render each class in its own section. The result supports
//! `is_ok()` so callers can use it as a fast-fail predicate.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use serde::Deserialize;

use crate::opensymphony_planning::generator::domain::TaskId;

use super::domain::{
    InvalidTaskFile, ManifestValidationResult, MissingTaskFile, SelfBlock, UnknownDependency,
    UnknownMilestone,
};
use super::frontmatter::{TaskFrontmatter, TaskFrontmatterError, parse_task_file};

/// Raw representation of `docs/tasks/task-package.yaml`.
///
/// The on-disk schema uses camelCase keys (`planningWave`, `tasksDir`)
/// matching the existing fixture files. Fields are decoded with explicit
/// `#[serde(rename = ...)]` so the validator works without a custom
/// `serde` adapter.
#[derive(Debug, Clone, Deserialize)]
pub struct TaskPackageManifestFile {
    #[serde(rename = "planningWave")]
    pub planning_wave: String,
    #[serde(rename = "tasksDir", default)]
    pub tasks_dir: String,
    #[serde(default)]
    pub milestones: Vec<String>,
    pub tasks: Vec<ManifestTaskEntry>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ManifestTaskEntry {
    pub id: String,
    pub file: String,
}

/// Loads a task-package manifest from disk.
pub fn load_manifest(path: &Path) -> Result<TaskPackageManifestFile, ManifestValidatorError> {
    let raw = fs::read_to_string(path).map_err(|source| ManifestValidatorError::Io {
        path: path.display().to_string(),
        source,
    })?;
    serde_yaml::from_str(&raw).map_err(|source| ManifestValidatorError::Yaml {
        path: path.display().to_string(),
        source,
    })
}

/// Errors surfaced by manifest loading. Validation paths emit their own
/// non-error `ManifestValidationResult` instead.
#[derive(Debug, thiserror::Error)]
pub enum ManifestValidatorError {
    #[error("failed to read manifest {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse manifest {path}: {source}")]
    Yaml {
        path: String,
        #[source]
        source: serde_yaml::Error,
    },
}

/// Manifest validator.
#[allow(dead_code)]
pub struct ManifestValidator;

impl ManifestValidator {
    /// Validates the supplied manifest file path (the manifest itself) and
    /// each declared task file path against `repo_root`. Missing or
    /// unreadable task files are surfaced via `missing_task_files`, not as
    /// hard errors.
    ///
    /// `repo_root` is supplied explicitly so the validator cannot silently
    /// mis-resolve relative task paths when the manifest's location in the
    /// repository layout changes (see COE-416 review).
    #[allow(dead_code)]
    pub fn validate(
        manifest_path: &Path,
        repo_root: &Path,
    ) -> Result<ManifestValidationResult, ManifestValidatorError> {
        let manifest = load_manifest(manifest_path)?;
        Ok(Self::validate_against_repo_root(&manifest, repo_root))
    }

    /// Same as [`Self::validate`] but takes a pre-parsed manifest and a
    /// repository root to anchor relative task paths. Useful for unit tests
    /// that exercise the validator with temporary fixtures on disk.
    pub fn validate_against_repo_root(
        manifest: &TaskPackageManifestFile,
        repo_root: &Path,
    ) -> ManifestValidationResult {
        let mut result = ManifestValidationResult {
            planning_wave: manifest.planning_wave.clone(),
            declared_task_ids: Vec::new(),
            missing_task_files: Vec::new(),
            invalid_task_files: Vec::new(),
            unknown_milestones: Vec::new(),
            unknown_dependencies: Vec::new(),
            creation_order_cycles: Vec::new(),
            self_blocks: Vec::new(),
            duplicate_task_ids: Vec::new(),
        };

        let mut seen_ids: BTreeSet<TaskId> = BTreeSet::new();
        let mut entries: Vec<(TaskId, TaskFrontmatter)> = Vec::new();
        let milestone_set: BTreeSet<String> = manifest.milestones.iter().cloned().collect();

        for entry in &manifest.tasks {
            let id = TaskId::new(entry.id.clone());
            if !seen_ids.insert(id.clone()) {
                result.duplicate_task_ids.push(id);
                continue;
            }
            result.declared_task_ids.push(id.clone());
            let path = repo_root.join(&entry.file);
            match parse_task_file(&path) {
                Ok(parsed) => entries.push((id.clone(), parsed.frontmatter)),
                Err(TaskFrontmatterError::Io { source, .. })
                    if source.kind() == std::io::ErrorKind::NotFound =>
                {
                    result.missing_task_files.push(MissingTaskFile {
                        task_id: id.clone(),
                        file_path: entry.file.clone(),
                    });
                }
                Err(err) => {
                    // The file exists on disk but is not loadable as a task
                    // file (YAML syntax error, missing frontmatter, IO
                    // permission denied, etc). Surface it as a distinct
                    // `invalid_task_files` finding so users fixing the
                    // manifest see the real cause instead of a phantom
                    // "missing" file.
                    result.invalid_task_files.push(InvalidTaskFile {
                        task_id: id.clone(),
                        file_path: entry.file.clone(),
                        reason: err.to_string(),
                    });
                }
            }
        }

        let id_set: BTreeSet<TaskId> = result.declared_task_ids.iter().cloned().collect();
        let mut adjacency: BTreeMap<TaskId, BTreeSet<TaskId>> = BTreeMap::new();
        for (task_id, frontmatter) in &entries {
            if !milestone_set.contains(frontmatter.milestone.as_deref().unwrap_or_default())
                && let Some(declared) = frontmatter.milestone.clone()
            {
                result.unknown_milestones.push(UnknownMilestone {
                    task_id: task_id.clone(),
                    declared_milestone: declared,
                });
            }
            for dep in &frontmatter.blocked_by {
                if dep == &task_id.0 {
                    result.self_blocks.push(SelfBlock {
                        task_id: task_id.clone(),
                    });
                } else if !id_set.contains(&TaskId::new(dep.clone())) {
                    result.unknown_dependencies.push(UnknownDependency {
                        from_task_id: task_id.clone(),
                        unknown_dependency: TaskId::new(dep.clone()),
                    });
                } else {
                    adjacency
                        .entry(task_id.clone())
                        .or_default()
                        .insert(TaskId::new(dep.clone()));
                }
            }
        }

        result.creation_order_cycles = creation_order_cycles(&adjacency, &id_set);
        // Stable order keeps the artefact diff-friendly for tests.
        result
            .missing_task_files
            .sort_by(|a, b| a.task_id.cmp(&b.task_id));
        result
            .invalid_task_files
            .sort_by(|a, b| a.task_id.cmp(&b.task_id));
        result
            .unknown_milestones
            .sort_by(|a, b| a.task_id.cmp(&b.task_id));
        result
            .unknown_dependencies
            .sort_by(|a, b| a.from_task_id.cmp(&b.from_task_id));
        result.self_blocks.sort_by(|a, b| a.task_id.cmp(&b.task_id));
        result
    }
}

/// Returns the minimal cycles (one representative cycle path per
/// strongly-connected component) in the directed graph implied by
/// `adjacency`. We use a simple DFS for tasks working in BTreeMap order
/// so the output is deterministic.
fn creation_order_cycles(
    adjacency: &BTreeMap<TaskId, BTreeSet<TaskId>>,
    nodes: &BTreeSet<TaskId>,
) -> Vec<Vec<TaskId>> {
    let mut visited: BTreeSet<TaskId> = BTreeSet::new();
    let mut on_stack: BTreeSet<TaskId> = BTreeSet::new();
    let mut stack: Vec<TaskId> = Vec::new();
    let mut seen_cycles: BTreeSet<Vec<TaskId>> = BTreeSet::new();
    let mut collected: Vec<Vec<TaskId>> = Vec::new();
    let mut state = DfsState {
        adjacency,
        visited: &mut visited,
        on_stack: &mut on_stack,
        stack: &mut stack,
        seen_cycles: &mut seen_cycles,
        collected: &mut collected,
    };

    for entry in nodes {
        if !state.visited.contains(entry) {
            dfs_cycle(entry, &mut state);
        }
    }
    collected
}

/// Aggregated mutable DFS bookkeeping shared across recursive calls. Grouped
/// into a struct (rather than passed as separate parameters) so the recursive
/// call sites stay readable as the validator grows (see COE-416 review).
struct DfsState<'a> {
    adjacency: &'a BTreeMap<TaskId, BTreeSet<TaskId>>,
    visited: &'a mut BTreeSet<TaskId>,
    on_stack: &'a mut BTreeSet<TaskId>,
    stack: &'a mut Vec<TaskId>,
    seen_cycles: &'a mut BTreeSet<Vec<TaskId>>,
    collected: &'a mut Vec<Vec<TaskId>>,
}

fn dfs_cycle(node: &TaskId, state: &mut DfsState<'_>) {
    state.visited.insert(node.clone());
    state.on_stack.insert(node.clone());
    state.stack.push(node.clone());
    if let Some(deps) = state.adjacency.get(node) {
        for dep in deps {
            if !state.visited.contains(dep) {
                dfs_cycle(dep, state);
            } else if state.on_stack.contains(dep)
                && let Some(start_idx) = state.stack.iter().position(|n| n == dep)
            {
                // `stack[start_idx..]` already includes `dep` at `start_idx`,
                // so appending it again would duplicate the first entry and
                // produce a malformed cycle vector (see COE-416 review).
                let mut cycle: Vec<TaskId> = state.stack[start_idx..].to_vec();
                cycle.sort();
                if state.seen_cycles.insert(cycle.clone()) {
                    state.collected.push(cycle);
                }
            }
        }
    }
    state.on_stack.remove(node);
    state.stack.pop();
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::collections::BTreeSet;
    use std::fs;
    use std::io::Write;

    fn write(path: &Path, contents: &str) {
        let mut file = fs::File::create(path).expect("create file");
        file.write_all(contents.as_bytes()).expect("write file");
    }

    fn fixture_with_manifest(
        tmp: &Path,
        manifest_text: &str,
        files: Vec<(String, String)>,
    ) -> TaskPackageManifestFile {
        write(&tmp.join("task-package.yaml"), manifest_text);
        for (path, contents) in files {
            let full = tmp.join(&path);
            if let Some(parent) = full.parent() {
                fs::create_dir_all(parent).expect("mkdir");
            }
            write(&full, &contents);
        }
        load_manifest(&tmp.join("task-package.yaml")).expect("manifest loads")
    }

    fn manifest_with_tasks(tasks: &[(&str, &str)]) -> String {
        let mut s = String::from(
            "planningWave: test\ntasksDir: docs/tasks\nmilestones:\n  - \"M1\"\ntasks:\n",
        );
        for (id, file) in tasks {
            s.push_str(&format!("  - id: {}\n    file: {}\n", id, file));
        }
        s
    }

    fn task_file_text(id: &str, milestone: &str, blocked_by: &[&str], blocks: &[&str]) -> String {
        let mut s = format!(
            "---\nid: {}\ntitle: \"{}\"\nmilestone: \"{}\"\nblockedBy: [",
            id, id, milestone
        );
        s.push_str(
            &blocked_by
                .iter()
                .map(|x| format!("\"{}\"", x))
                .collect::<Vec<_>>()
                .join(", "),
        );
        s.push_str("]\nblocks: [");
        s.push_str(
            &blocks
                .iter()
                .map(|x| format!("\"{}\"", x))
                .collect::<Vec<_>>()
                .join(", "),
        );
        s.push_str("]\n---\n# Test\n");
        s
    }

    #[test]
    fn validates_clean_manifest() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let manifest_text =
            manifest_with_tasks(&[("TASK-A", "docs/tasks/a.md"), ("TASK-B", "docs/tasks/b.md")]);
        let files = vec![
            (
                "docs/tasks/a.md".to_string(),
                task_file_text("TASK-A", "M1", &[], &["TASK-B"]),
            ),
            (
                "docs/tasks/b.md".to_string(),
                task_file_text("TASK-B", "M1", &["TASK-A"], &[]),
            ),
        ];
        let manifest = fixture_with_manifest(tmp.path(), &manifest_text, files);
        let result = ManifestValidator::validate_against_repo_root(&manifest, tmp.path());
        assert!(result.is_ok(), "unexpected findings: {result:?}");
        assert_eq!(result.error_count(), 0);
    }

    #[test]
    fn missing_task_file_is_reported() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let manifest_text = manifest_with_tasks(&[
            ("TASK-A", "docs/tasks/a.md"),
            ("TASK-B", "docs/tasks/missing.md"),
        ]);
        let files = vec![(
            "docs/tasks/a.md".to_string(),
            task_file_text("TASK-A", "M1", &[], &[]),
        )];
        let manifest = fixture_with_manifest(tmp.path(), &manifest_text, files);
        let result = ManifestValidator::validate_against_repo_root(&manifest, tmp.path());
        assert!(!result.is_ok());
        assert_eq!(result.missing_task_files.len(), 1);
        assert_eq!(result.missing_task_files[0].task_id, TaskId::new("TASK-B"));
    }

    #[test]
    fn invalid_task_file_is_reported_separately_from_missing() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let manifest_text = manifest_with_tasks(&[
            ("TASK-A", "docs/tasks/a.md"),
            // Declare a task whose frontmatter is malformed YAML. The file
            // *exists* on disk, so this must surface as an `invalid_task_files`
            // finding rather than being bucketed into `missing_task_files`
            // (which would mislead users into chasing a phantom missing file).
            ("TASK-B", "docs/tasks/broken.md"),
        ]);
        let files = vec![
            (
                "docs/tasks/a.md".to_string(),
                task_file_text("TASK-A", "M1", &[], &[]),
            ),
            (
                "docs/tasks/broken.md".to_string(),
                // Open the frontmatter block but never close it; this
                // exercises the brace/quote mismatch path in
                // `parse_task_text`.
                "---\nid: \"TASK-B\"\ntitle: \"Broken\"\n  unclosed_quote: \"\n".to_string(),
            ),
        ];
        let manifest = fixture_with_manifest(tmp.path(), &manifest_text, files);
        let result = ManifestValidator::validate_against_repo_root(&manifest, tmp.path());
        assert!(!result.is_ok());
        assert_eq!(
            result.missing_task_files.len(),
            0,
            "files that exist on disk must not be reported as missing: {result:?}",
        );
        assert_eq!(result.invalid_task_files.len(), 1);
        let invalid = &result.invalid_task_files[0];
        assert_eq!(invalid.task_id, TaskId::new("TASK-B"));
        assert_eq!(invalid.file_path, "docs/tasks/broken.md");
        assert!(
            !invalid.reason.is_empty(),
            "invalid_task_files entries must carry a non-empty reason so users can fix the root cause"
        );

        // The total error count rolls invalid_task_files into the tally so
        // dashboards do not silently lose visibility into malformed
        // manifests.
        assert!(
            result.error_count() >= 1,
            "error_count must include invalid_task_files: {result:?}",
        );
    }

    /// Convenience builder that returns a task file YAML with the closing
    /// `---` marker intentionally omitted. Used by the regression tests
    /// that exercise the missing-frontmatter path.
    fn task_file_with_open_marker(id: &str, milestone: &str) -> String {
        format!(
            "---\nid: \"{}\"\ntitle: \"{}\"\nmilestone: \"{}\"\nblockedBy: []\nblocks: []\n",
            id, id, milestone
        )
    }

    #[test]
    fn missing_frontmatter_is_reported_as_invalid_not_missing() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let manifest_text = manifest_with_tasks(&[("TASK-X", "docs/tasks/x.md")]);
        let files = vec![(
            "docs/tasks/x.md".to_string(),
            task_file_with_open_marker("TASK-X", "M1"),
        )];
        let manifest = fixture_with_manifest(tmp.path(), &manifest_text, files);
        let result = ManifestValidator::validate_against_repo_root(&manifest, tmp.path());
        assert_eq!(result.missing_task_files.len(), 0);
        assert_eq!(result.invalid_task_files.len(), 1);
        assert_eq!(result.invalid_task_files[0].task_id, TaskId::new("TASK-X"));
    }

    #[test]
    fn unknown_milestone_is_reported() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let manifest_text = manifest_with_tasks(&[("TASK-A", "docs/tasks/a.md")]);
        let files = vec![(
            "docs/tasks/a.md".to_string(),
            task_file_text("TASK-A", "M9", &[], &[]),
        )];
        let manifest = fixture_with_manifest(tmp.path(), &manifest_text, files);
        let result = ManifestValidator::validate_against_repo_root(&manifest, tmp.path());
        assert_eq!(result.unknown_milestones.len(), 1);
        assert_eq!(result.unknown_milestones[0].declared_milestone, "M9");
    }

    #[test]
    fn unknown_dependency_is_reported() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let manifest_text = manifest_with_tasks(&[("TASK-A", "docs/tasks/a.md")]);
        let files = vec![(
            "docs/tasks/a.md".to_string(),
            task_file_text("TASK-A", "M1", &["TASK-GHOST"], &[]),
        )];
        let manifest = fixture_with_manifest(tmp.path(), &manifest_text, files);
        let result = ManifestValidator::validate_against_repo_root(&manifest, tmp.path());
        assert_eq!(result.unknown_dependencies.len(), 1);
        assert_eq!(
            result.unknown_dependencies[0].unknown_dependency,
            TaskId::new("TASK-GHOST")
        );
    }

    #[test]
    fn creation_order_cycle_is_reported() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let manifest_text =
            manifest_with_tasks(&[("TASK-A", "docs/tasks/a.md"), ("TASK-B", "docs/tasks/b.md")]);
        let files = vec![
            (
                "docs/tasks/a.md".to_string(),
                task_file_text("TASK-A", "M1", &["TASK-B"], &[]),
            ),
            (
                "docs/tasks/b.md".to_string(),
                task_file_text("TASK-B", "M1", &["TASK-A"], &[]),
            ),
        ];
        let manifest = fixture_with_manifest(tmp.path(), &manifest_text, files);
        let result = ManifestValidator::validate_against_repo_root(&manifest, tmp.path());
        assert_eq!(result.creation_order_cycles.len(), 1);
        // Cycle must list each node exactly once (no duplicate start node
        // appended). After sort the length-2 cycle becomes [A, B].
        let cycle = &result.creation_order_cycles[0];
        assert_eq!(cycle.len(), 2);
        let unique: BTreeSet<&TaskId> = cycle.iter().collect();
        assert_eq!(unique.len(), 2);
        assert_eq!(cycle, &vec![TaskId::new("TASK-A"), TaskId::new("TASK-B")],);
    }

    #[test]
    fn self_block_is_reported() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let manifest_text = manifest_with_tasks(&[("TASK-A", "docs/tasks/a.md")]);
        let files = vec![(
            "docs/tasks/a.md".to_string(),
            task_file_text("TASK-A", "M1", &["TASK-A"], &[]),
        )];
        let manifest = fixture_with_manifest(tmp.path(), &manifest_text, files);
        let result = ManifestValidator::validate_against_repo_root(&manifest, tmp.path());
        assert_eq!(result.self_blocks.len(), 1);
        assert_eq!(result.self_blocks[0].task_id, TaskId::new("TASK-A"));
    }

    #[test]
    fn duplicate_task_id_is_reported() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let manifest_text = manifest_with_tasks(&[
            ("TASK-A", "docs/tasks/a.md"),
            ("TASK-A", "docs/tasks/copy.md"),
        ]);
        let files = vec![
            (
                "docs/tasks/a.md".to_string(),
                task_file_text("TASK-A", "M1", &[], &[]),
            ),
            (
                "docs/tasks/copy.md".to_string(),
                task_file_text("TASK-A", "M1", &[], &[]),
            ),
        ];
        let manifest = fixture_with_manifest(tmp.path(), &manifest_text, files);
        let result = ManifestValidator::validate_against_repo_root(&manifest, tmp.path());
        assert_eq!(result.duplicate_task_ids, vec![TaskId::new("TASK-A")]);
        // error_count must include duplicate ids so dashboards/tests that
        // rely on the count do not silently under-report when only the
        // duplicate id check fires (see COE-416 review).
        assert!(result.error_count() >= 1);
        assert!(!result.is_ok());
    }

    #[test]
    fn validate_takes_explicit_repo_root() {
        // `ManifestValidator::validate` must consume the supplied repo_root
        // so callers can anchor relative task paths regardless of where the
        // manifest lives in the repository layout. This guards against the
        // previously fragile `.parent().and_then(Path::parent)` heuristic.
        let workspace = tempfile::tempdir().expect("workspace");
        let project = workspace.path().join("project");
        fs::create_dir_all(project.join("docs/tasks")).expect("mkdir");
        let manifest_path = project.join("docs/tasks/task-package.yaml");
        fs::write(
            &manifest_path,
            manifest_with_tasks(&[("TASK-A", "docs/tasks/a.md")]),
        )
        .expect("write manifest");
        fs::write(
            project.join("docs/tasks/a.md"),
            task_file_text("TASK-A", "M1", &[], &[]),
        )
        .expect("write task file");

        // Anchoring at the workspace root should fail to find the file
        // (truth path is `project/docs/tasks/a.md`).
        let bad_result = ManifestValidator::validate(&manifest_path, workspace.path())
            .expect("manifest parses")
            .missing_task_files;
        assert_eq!(bad_result.len(), 1);

        // Anchoring at the project root (where the file actually lives)
        // resolves the relative path correctly.
        let good_result = ManifestValidator::validate(&manifest_path, &project)
            .expect("manifest parses")
            .missing_task_files;
        assert!(good_result.is_empty());
    }
}

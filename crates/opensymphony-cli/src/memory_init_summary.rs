use std::path::Path;

use crate::opensymphony_memory::{MemoryInitApplyReport, MemoryInitFileChange};

pub(crate) fn record_memory_init_changes(
    report: &MemoryInitApplyReport,
    target_repo: &Path,
    created: &mut Vec<String>,
    updated: &mut Vec<String>,
    unchanged: &mut Vec<String>,
) {
    record_memory_init_change(
        relative_path_for_summary(target_repo, &report.config_path),
        report.config,
        created,
        updated,
        unchanged,
    );
    record_memory_init_change(
        relative_path_for_summary(target_repo, &report.gitignore_path),
        report.gitignore,
        created,
        updated,
        unchanged,
    );
}

pub(crate) fn memory_init_change_lists(
    report: &MemoryInitApplyReport,
    target_repo: &Path,
) -> (Vec<String>, Vec<String>, Vec<String>) {
    let mut created = Vec::new();
    let mut updated = Vec::new();
    let mut unchanged = Vec::new();
    record_memory_init_changes(
        report,
        target_repo,
        &mut created,
        &mut updated,
        &mut unchanged,
    );
    (created, updated, unchanged)
}

fn record_memory_init_change(
    path: String,
    change: MemoryInitFileChange,
    created: &mut Vec<String>,
    updated: &mut Vec<String>,
    unchanged: &mut Vec<String>,
) {
    match change {
        MemoryInitFileChange::Created => created.push(path),
        MemoryInitFileChange::Updated => updated.push(path),
        MemoryInitFileChange::Unchanged => unchanged.push(path),
    }
}

fn relative_path_for_summary(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .into_owned()
}

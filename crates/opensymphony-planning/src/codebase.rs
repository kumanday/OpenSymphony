use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Result of scanning a repository for structural analysis.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodebaseAnalysis {
    pub root_path: String,
    pub languages: Vec<LanguageSignature>,
    pub packages: Vec<PackageInfo>,
    pub build_systems: Vec<String>,
    pub ownership_files: Vec<OwnershipSignal>,
    pub integration_points: Vec<IntegrationPoint>,
    pub conventions: Vec<Convention>,
    pub risks: Vec<AnalysisRisk>,
    pub total_files: usize,
    pub total_rust_files: usize,
    pub total_typescript_files: usize,
}

/// Detected language with file count and representative paths.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LanguageSignature {
    pub language: String,
    pub file_count: usize,
    pub sample_paths: Vec<String>,
}

/// A package/crate/module within the repository.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageInfo {
    pub name: String,
    pub relative_path: String,
    pub kind: PackageKind,
    pub dependencies: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PackageKind {
    Library,
    Binary,
    TestUtilities,
    Frontend,
}

/// Signal that indicates ownership or boundary information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OwnershipSignal {
    pub file_path: String,
    pub signal_type: OwnershipSignalType,
    pub content_hint: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OwnershipSignalType {
    CargoWorkspace,
    Readme,
    License,
    Gitignore,
    Codeowners,
    PackageJson,
}

/// A detected integration point between packages or with external systems.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntegrationPoint {
    pub source_package: String,
    pub target_package: Option<String>,
    pub integration_type: IntegrationType,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntegrationType {
    CrossCrateDependency,
    ApiClient,
    DatabaseAccess,
    ExternalService,
    SharedSchema,
}

/// A detected coding or structural convention.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Convention {
    pub area: String,
    pub description: String,
    pub evidence_path: String,
}

/// A risk or concern identified during analysis.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalysisRisk {
    pub category: RiskCategory,
    pub severity: RiskSeverity,
    pub description: String,
    pub affected_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskCategory {
    Complexity,
    Security,
    Coupling,
    Testing,
    Performance,
    Maintenance,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskSeverity {
    Low,
    Medium,
    High,
}

/// Scans a repository path and produces a structured codebase analysis.
pub struct CodebaseAnalyzer {
    root: PathBuf,
}

impl CodebaseAnalyzer {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn analyze(&self) -> Result<CodebaseAnalysis, CodebaseAnalysisError> {
        if !self.root.is_dir() {
            return Err(CodebaseAnalysisError::NotADirectory(
                self.root.display().to_string(),
            ));
        }

        let walker = RepoWalker::new(&self.root);
        let file_inventory = walker.walk()?;

        let languages = detect_languages(&file_inventory);
        let packages = detect_packages(&self.root, &file_inventory)?;
        let build_systems = detect_build_systems(&self.root);
        let ownership_files = detect_ownership_signals(&self.root, &file_inventory);
        let integration_points = detect_integration_points(&self.root, &packages, &file_inventory);
        let conventions = detect_conventions(&self.root, &file_inventory);
        let risks = assess_risks(&self.root, &packages, &integration_points);

        let total_rust = file_inventory
            .keys()
            .filter(|p| p.extension().map(|e| e == "rs").unwrap_or(false))
            .count();
        let total_ts = file_inventory
            .keys()
            .filter(|p| {
                p.extension()
                    .map(|e| e == "ts" || e == "tsx")
                    .unwrap_or(false)
            })
            .count();

        Ok(CodebaseAnalysis {
            root_path: self.root.display().to_string(),
            languages,
            packages,
            build_systems,
            ownership_files,
            integration_points,
            conventions,
            risks,
            total_files: file_inventory.len(),
            total_rust_files: total_rust,
            total_typescript_files: total_ts,
        })
    }
}

/// Walks a directory tree and returns relative paths for each file.
struct RepoWalker {
    root: PathBuf,
    exclude_dirs: HashSet<String>,
}

impl RepoWalker {
    fn new(root: &Path) -> Self {
        let mut exclude_dirs = HashSet::new();
        for dir in [
            "node_modules",
            ".git",
            "target",
            ".venv",
            "__pycache__",
            "dist",
            "build",
        ] {
            exclude_dirs.insert(dir.to_string());
        }
        Self {
            root: root.to_path_buf(),
            exclude_dirs,
        }
    }

    fn walk(&self) -> Result<BTreeMap<PathBuf, usize>, CodebaseAnalysisError> {
        let mut inventory = BTreeMap::new();
        self.walk_dir(&self.root, &mut inventory)?;
        Ok(inventory)
    }

    fn walk_dir(
        &self,
        dir: &Path,
        inventory: &mut BTreeMap<PathBuf, usize>,
    ) -> Result<(), CodebaseAnalysisError> {
        let entries = fs::read_dir(dir).map_err(|e| CodebaseAnalysisError::Io {
            path: dir.display().to_string(),
            source: e,
        })?;

        for entry in entries {
            let entry = entry.map_err(|e| CodebaseAnalysisError::Io {
                path: dir.display().to_string(),
                source: e,
            })?;
            let path = entry.path();

            // Use entry.file_type() to avoid following symlinks, preventing infinite loops.
            if entry
                .file_type()
                .ok()
                .map(|ft| ft.is_dir() && !ft.is_symlink())
                .unwrap_or(false)
            {
                if path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .map(|c| self.exclude_dirs.contains(c))
                    .unwrap_or(false)
                {
                    continue;
                }
                if path
                    .file_name()
                    .map(|n| n.to_string_lossy().starts_with('.'))
                    .unwrap_or(false)
                    && path.file_name().map(|n| n != ".github").unwrap_or(true)
                {
                    continue;
                }
                self.walk_dir(&path, inventory)?;
            } else {
                let relative = path.strip_prefix(&self.root).unwrap_or(&path).to_path_buf();
                let size = entry.metadata().ok().map(|m| m.len() as usize).unwrap_or(0);
                inventory.insert(relative, size);
            }
        }
        Ok(())
    }
}

/// Derive a package name from a file path by looking for the crate name under
/// `crates/<name>/` or falling back to the best-matching package by path prefix.
fn derive_package_name(path: &Path, packages: &[PackageInfo]) -> String {
    let path_str = path.display().to_string();
    path.components()
        .skip_while(|c| c.as_os_str() != "crates")
        .nth(1)
        .map(|c| c.as_os_str().to_string_lossy().to_string())
        .or_else(|| {
            packages
                .iter()
                .filter(|p| path_str.starts_with(&p.relative_path))
                .max_by_key(|p| p.relative_path.len())
                .map(|p| p.name.clone())
        })
        .unwrap_or_else(|| path.display().to_string())
}

fn detect_languages(inventory: &BTreeMap<PathBuf, usize>) -> Vec<LanguageSignature> {
    let mut lang_map: BTreeMap<String, (usize, Vec<String>)> = BTreeMap::new();

    for path in inventory.keys() {
        let ext = path.extension().and_then(|e| e.to_str());
        let language = match ext {
            Some("rs") => "rust",
            Some("ts") | Some("tsx") => "typescript",
            Some("js") | Some("jsx") => "javascript",
            Some("toml") => "toml",
            Some("json") => "json",
            Some("yaml") | Some("yml") => "yaml",
            Some("md") => "markdown",
            Some("graphql") => "graphql",
            Some("sh") => "shell",
            Some("py") => "python",
            _ => continue,
        };

        let entry = lang_map
            .entry(language.to_string())
            .or_insert_with(|| (0, Vec::new()));
        entry.0 += 1;
        if entry.1.len() < 3 {
            entry.1.push(path.display().to_string());
        }
    }

    lang_map
        .into_iter()
        .map(|(lang, (count, samples))| LanguageSignature {
            language: lang,
            file_count: count,
            sample_paths: samples,
        })
        .collect()
}

fn detect_packages(
    root: &Path,
    _inventory: &BTreeMap<PathBuf, usize>,
) -> Result<Vec<PackageInfo>, CodebaseAnalysisError> {
    let mut packages = Vec::new();

    // Detect Rust crates
    let crates_dir = root.join("crates");
    if crates_dir.is_dir() {
        for entry in fs::read_dir(&crates_dir).map_err(|e| CodebaseAnalysisError::Io {
            path: crates_dir.display().to_string(),
            source: e,
        })? {
            let entry = entry.map_err(|e| CodebaseAnalysisError::Io {
                path: crates_dir.display().to_string(),
                source: e,
            })?;
            if !entry.path().is_dir() {
                continue;
            }
            let cargo_toml = entry.path().join("Cargo.toml");
            if !cargo_toml.exists() {
                continue;
            }

            let name = entry.file_name().to_string_lossy().to_string();
            // Parse Cargo.toml once and reuse for both dependency extraction and binary detection
            let parsed_toml = match parse_cargo_toml(&cargo_toml) {
                Ok(table) => Some(table),
                Err(e) => {
                    eprintln!("Warning: failed to parse {}: {}", cargo_toml.display(), e);
                    None
                }
            };
            let deps = parsed_toml
                .as_ref()
                .map(extract_deps_from_table)
                .unwrap_or_default();
            let kind = if parsed_toml.as_ref().is_some_and(has_binary_in_table)
                || entry.path().join("src").join("main.rs").exists()
                || entry.path().join("src").join("bin").is_dir()
            {
                PackageKind::Binary
            } else if name.contains("test") || name.contains("testkit") {
                PackageKind::TestUtilities
            } else {
                PackageKind::Library
            };

            packages.push(PackageInfo {
                name,
                relative_path: format!("crates/{}", entry.file_name().to_string_lossy()),
                kind,
                dependencies: deps,
            });
        }
    }

    // Detect TypeScript packages
    for pkg_dir in ["packages", "apps"] {
        let dir = root.join(pkg_dir);
        if !dir.is_dir() {
            continue;
        }
        for entry in fs::read_dir(&dir).map_err(|e| CodebaseAnalysisError::Io {
            path: dir.display().to_string(),
            source: e,
        })? {
            let entry = entry.map_err(|e| CodebaseAnalysisError::Io {
                path: dir.display().to_string(),
                source: e,
            })?;
            if !entry.path().is_dir() {
                continue;
            }
            let pkg_json = entry.path().join("package.json");
            if !pkg_json.exists() {
                continue;
            }

            let name = entry.file_name().to_string_lossy().to_string();
            let deps = match extract_npm_deps(&pkg_json) {
                Ok(deps) => deps,
                Err(e) => {
                    eprintln!("Warning: failed to parse {}: {}", pkg_json.display(), e);
                    Vec::new()
                }
            };
            packages.push(PackageInfo {
                name,
                relative_path: format!("{}/{}", pkg_dir, entry.file_name().to_string_lossy()),
                kind: if pkg_dir == "apps" {
                    PackageKind::Binary
                } else {
                    PackageKind::Frontend
                },
                dependencies: deps,
            });
        }
    }

    Ok(packages)
}

/// Parse a Cargo.toml file and return the parsed TOML table.
fn parse_cargo_toml(path: &Path) -> Result<toml::Table, CodebaseAnalysisError> {
    let content = fs::read_to_string(path).map_err(|e| CodebaseAnalysisError::Io {
        path: path.display().to_string(),
        source: e,
    })?;

    content.parse().map_err(|e| CodebaseAnalysisError::Toml {
        path: path.display().to_string(),
        source: e,
    })
}

/// Extract dependencies from a parsed TOML table.
fn extract_deps_from_table(parsed: &toml::Table) -> Vec<String> {
    let mut deps = Vec::new();

    // Extract from [dependencies]
    if let Some(deps_table) = parsed.get("dependencies").and_then(|v| v.as_table()) {
        for key in deps_table.keys() {
            deps.push(key.clone());
        }
    }

    // Extract from [dev-dependencies]
    if let Some(dev_deps) = parsed.get("dev-dependencies").and_then(|v| v.as_table()) {
        for key in dev_deps.keys() {
            if !deps.contains(key) {
                deps.push(key.clone());
            }
        }
    }

    // Extract from [build-dependencies]
    if let Some(build_deps) = parsed.get("build-dependencies").and_then(|v| v.as_table()) {
        for key in build_deps.keys() {
            if !deps.contains(key) {
                deps.push(key.clone());
            }
        }
    }

    deps.sort();
    deps
}

/// Check if a parsed TOML table indicates a binary crate via [[bin]] section.
fn has_binary_in_table(parsed: &toml::Table) -> bool {
    // Check for [[bin]] section (array of tables)
    match parsed.get("bin") {
        Some(toml::Value::Array(arr)) => !arr.is_empty(),
        _ => false,
    }
}

/// Extract dependencies from a package.json file using basic JSON parsing.
/// Returns a sorted deduplicated list of dependency names from all dependency categories.
fn extract_npm_deps(path: &Path) -> Result<Vec<String>, CodebaseAnalysisError> {
    let content = fs::read_to_string(path).map_err(|e| CodebaseAnalysisError::Io {
        path: path.display().to_string(),
        source: e,
    })?;

    #[derive(serde::Deserialize)]
    struct NpmPackage {
        #[serde(default)]
        dependencies: BTreeMap<String, serde_json::Value>,
        #[serde(default, rename = "devDependencies")]
        dev_dependencies: BTreeMap<String, serde_json::Value>,
        #[serde(default, rename = "peerDependencies")]
        peer_dependencies: BTreeMap<String, serde_json::Value>,
        #[serde(default, rename = "optionalDependencies")]
        optional_dependencies: BTreeMap<String, serde_json::Value>,
    }

    let pkg: NpmPackage =
        serde_json::from_str(&content).map_err(|e| CodebaseAnalysisError::Json {
            path: path.display().to_string(),
            source: e,
        })?;

    let mut deps: BTreeMap<String, ()> = BTreeMap::new();
    for (name, _) in pkg.dependencies {
        deps.insert(name, ());
    }
    for (name, _) in pkg.dev_dependencies {
        deps.entry(name).or_insert(());
    }
    for (name, _) in pkg.peer_dependencies {
        deps.entry(name).or_insert(());
    }
    for (name, _) in pkg.optional_dependencies {
        deps.entry(name).or_insert(());
    }

    Ok(deps.into_keys().collect())
}

fn detect_build_systems(root: &Path) -> Vec<String> {
    let mut systems = Vec::new();

    if root.join("Cargo.toml").exists() {
        systems.push("cargo".to_string());
    }
    if root.join("package.json").exists() {
        systems.push("npm".to_string());
    }
    if root.join("Makefile").exists() {
        systems.push("make".to_string());
    }
    if root.join("justfile").exists() {
        systems.push("just".to_string());
    }

    systems
}

fn detect_ownership_signals(
    root: &Path,
    inventory: &BTreeMap<PathBuf, usize>,
) -> Vec<OwnershipSignal> {
    let mut signals = Vec::new();

    let signal_files = [
        (
            "Cargo.toml",
            OwnershipSignalType::CargoWorkspace,
            "Rust workspace definition",
        ),
        (
            "README.md",
            OwnershipSignalType::Readme,
            "Project documentation",
        ),
        ("LICENSE", OwnershipSignalType::License, "License file"),
        (
            ".gitignore",
            OwnershipSignalType::Gitignore,
            "Git ignore rules",
        ),
        (
            ".github/CODEOWNERS",
            OwnershipSignalType::Codeowners,
            "Code ownership mapping",
        ),
        (
            "package.json",
            OwnershipSignalType::PackageJson,
            "Node.js package definition",
        ),
    ];

    for (file, signal_type, hint) in &signal_files {
        let path = PathBuf::from(file);
        if inventory.contains_key(&path) || root.join(file).exists() {
            signals.push(OwnershipSignal {
                file_path: file.to_string(),
                signal_type: signal_type.clone(),
                content_hint: hint.to_string(),
            });
        }
    }

    signals
}

fn detect_integration_points(
    _root: &Path,
    packages: &[PackageInfo],
    inventory: &BTreeMap<PathBuf, usize>,
) -> Vec<IntegrationPoint> {
    let mut points = Vec::new();

    // Cross-crate dependencies from Cargo.toml
    for pkg in packages {
        for dep in &pkg.dependencies {
            if packages.iter().any(|p| p.name == *dep) {
                points.push(IntegrationPoint {
                    source_package: pkg.name.clone(),
                    target_package: Some(format!("crates/{dep}")),
                    integration_type: IntegrationType::CrossCrateDependency,
                    detail: format!("Cargo dependency: {dep}"),
                });
            }
        }
    }

    // Detect API/client patterns
    for path in inventory.keys() {
        let path_str = path.display().to_string();
        if (path_str.contains("client") || path_str.contains("transport"))
            && path.extension().map(|e| e == "rs").unwrap_or(false)
            && path.parent().is_some()
        {
            let pkg_name = derive_package_name(path, packages);
            points.push(IntegrationPoint {
                source_package: pkg_name,
                target_package: None,
                integration_type: IntegrationType::ApiClient,
                detail: format!("Client/transport in: {}", path.display()),
            });
        }
        // Detect database access
        if (path_str.contains("duckdb")
            || path_str.contains("database")
            || path_str.contains("db_"))
            && path.extension().map(|e| e == "rs").unwrap_or(false)
        {
            let db_pkg_name = derive_package_name(path, packages);
            points.push(IntegrationPoint {
                source_package: db_pkg_name,
                target_package: None,
                integration_type: IntegrationType::DatabaseAccess,
                detail: format!("Database access: {}", path.display()),
            });
        }
    }

    points
}

fn detect_conventions(root: &Path, inventory: &BTreeMap<PathBuf, usize>) -> Vec<Convention> {
    let mut conventions = Vec::new();

    // Rust workspace convention
    if root.join("Cargo.toml").exists() {
        conventions.push(Convention {
            area: "build".to_string(),
            description:
                "Rust workspace with shared dependency versions via [workspace.dependencies]"
                    .to_string(),
            evidence_path: "Cargo.toml".to_string(),
        });
    }

    // Linting convention
    if root.join("clippy.toml").exists() {
        conventions.push(Convention {
            area: "linting".to_string(),
            description: "Custom Clippy lint configuration".to_string(),
            evidence_path: "clippy.toml".to_string(),
        });
    }

    // Formatting convention
    if root.join("rustfmt.toml").exists() {
        conventions.push(Convention {
            area: "formatting".to_string(),
            description: "Custom rustfmt configuration".to_string(),
            evidence_path: "rustfmt.toml".to_string(),
        });
    }

    // Test organization - collect all unique crate test directories
    let mut seen_test_crates: HashSet<String> = HashSet::new();
    for path in inventory.keys() {
        let is_test_file = path
            .parent()
            .and_then(|p| p.file_name())
            .is_some_and(|n| n == "tests");
        if path.starts_with("crates/") && path.components().count() >= 3 && is_test_file {
            let crate_name = path
                .components()
                .nth(1)
                .map(|c| c.as_os_str().to_string_lossy().to_string())
                .unwrap_or_default();
            if seen_test_crates.insert(crate_name.clone()) {
                conventions.push(Convention {
                    area: "testing".to_string(),
                    description: format!("Integration tests in crates/{crate_name}/tests/"),
                    evidence_path: path.display().to_string(),
                });
            }
        }
    }

    // TypeScript project structure
    if root.join("tsconfig.json").exists() {
        conventions.push(Convention {
            area: "typescript".to_string(),
            description: "TypeScript with project references".to_string(),
            evidence_path: "tsconfig.json".to_string(),
        });
    }

    conventions
}

fn assess_risks(
    root: &Path,
    packages: &[PackageInfo],
    integration_points: &[IntegrationPoint],
) -> Vec<AnalysisRisk> {
    let mut risks = Vec::new();

    // Check for high coupling
    let mut coupling_counts: BTreeMap<String, usize> = BTreeMap::new();
    for ip in integration_points {
        if let Some(ref target) = ip.target_package {
            let count = coupling_counts.entry(target.clone()).or_insert(0);
            *count += 1;
        }
    }
    for (pkg, count) in &coupling_counts {
        if *count >= 5 {
            risks.push(AnalysisRisk {
                category: RiskCategory::Coupling,
                severity: RiskSeverity::High,
                description: format!("High coupling: {pkg} is depended on by {count} packages"),
                affected_path: pkg.clone(),
            });
        }
    }

    // Check for missing test utilities
    let has_testkit = packages
        .iter()
        .any(|p| p.name.contains("test") || p.name.contains("testkit"));
    if !has_testkit && packages.len() > 3 {
        risks.push(AnalysisRisk {
            category: RiskCategory::Testing,
            severity: RiskSeverity::Medium,
            description: "No dedicated test utility crate found; shared test infrastructure may be duplicated".to_string(),
            affected_path: "crates/".to_string(),
        });
    }

    // Mixed language/build system risk
    // Detect both sub-package splits (crates/ + packages/) and root-level mixed builds
    let has_rust_packages = packages
        .iter()
        .any(|p| p.relative_path.starts_with("crates/"));
    let has_ts_packages = packages
        .iter()
        .any(|p| p.relative_path.starts_with("packages/") || p.relative_path.starts_with("apps/"));
    let has_root_cargo = root.join("Cargo.toml").exists();
    let has_root_npm = root.join("package.json").exists();
    let mixed_sub_packages = has_rust_packages && has_ts_packages;
    let mixed_root = has_root_cargo && has_root_npm;
    if mixed_sub_packages || mixed_root {
        risks.push(AnalysisRisk {
            category: RiskCategory::Maintenance,
            severity: RiskSeverity::Medium,
            description:
                "Mixed Rust/TypeScript monorepo requires coordinated build and CI strategies"
                    .to_string(),
            affected_path: "root".to_string(),
        });
    }

    risks
}

/// Error type for codebase analysis operations.
#[derive(Debug, thiserror::Error)]
pub enum CodebaseAnalysisError {
    #[error("not a directory: {0}")]
    NotADirectory(String),
    #[error("IO error reading {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("TOML parse error in {path}: {source}")]
    Toml {
        path: String,
        #[source]
        source: toml::de::Error,
    },
    #[error("JSON parse error in {path}: {source}")]
    Json {
        path: String,
        #[source]
        source: serde_json::Error,
    },
}

// Intentionally left as a standalone result type

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn create_test_repo(tmp_dir: &TempDir) -> PathBuf {
        let root = tmp_dir.path().to_path_buf();

        // Create Cargo.toml
        fs::write(
            root.join("Cargo.toml"),
            r#"[workspace]
members = ["."]
resolver = "2"

[workspace.dependencies]
serde = { version = "1", features = ["derive"] }
tokio = { version = "1", features = ["full"] }

[dependencies]
serde = { workspace = true }
tokio = { workspace = true }
"#,
        )
        .unwrap();

        // Create crates
        for crate_name in [
            "opensymphony-core",
            "opensymphony-linear",
            "opensymphony-testkit",
        ] {
            let crate_dir = root.join("crates").join(crate_name);
            fs::create_dir_all(crate_dir.join("src")).unwrap();

            fs::write(
                crate_dir.join("Cargo.toml"),
                format!(
                    r#"[package]
name = "{crate_name}"
version = "0.1.0"
edition = "2024"

[dependencies]
serde = {{ workspace = true }}"#
                ),
            )
            .unwrap();

            fs::write(crate_dir.join("src").join("lib.rs"), "// stub\n").unwrap();
        }

        // Create src/main.rs (binary crate)
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src").join("main.rs"), "fn main() {}\n").unwrap();

        // Create package.json
        fs::write(
            root.join("package.json"),
            r#"{ "name": "opensymphony", "version": "1.0.0" }"#,
        )
        .unwrap();

        // Create README.md
        fs::write(root.join("README.md"), "# OpenSymphony\n").unwrap();

        // Create .gitignore
        fs::write(root.join(".gitignore"), "/target\n/node_modules\n").unwrap();

        // Create rust-toolchain.toml
        fs::write(
            root.join("rust-toolchain.toml"),
            "[toolchain]\nchannel = \"1.93\"\n",
        )
        .unwrap();

        // Create clippy.toml
        fs::write(root.join("clippy.toml"), "").unwrap();

        // Create rustfmt.toml
        fs::write(root.join("rustfmt.toml"), "max_width = 100\n").unwrap();

        // Create a client module to test integration detection
        let client_dir = root.join("crates").join("opensymphony-linear").join("src");
        fs::write(client_dir.join("client.rs"), "// HTTP client stub\n").unwrap();

        root
    }

    #[test]
    fn analyze_detects_rust_packages_and_languages() {
        let tmp = TempDir::new().expect("temp dir");
        let root = create_test_repo(&tmp);

        let analyzer = CodebaseAnalyzer::new(&root);
        let analysis = analyzer.analyze().expect("analysis should succeed");

        assert!(analysis.total_rust_files > 0, "should detect Rust files");
        assert!(analysis.total_files > 0, "should count files in repository");

        // Verify Rust language detected
        assert!(
            analysis.languages.iter().any(|l| l.language == "rust"),
            "should detect rust language"
        );

        // Verify packages detected
        assert!(
            analysis
                .packages
                .iter()
                .any(|p| p.name == "opensymphony-core"),
            "should detect opensymphony-core crate"
        );
        assert!(
            analysis
                .packages
                .iter()
                .any(|p| p.name == "opensymphony-linear"),
            "should detect opensymphony-linear crate"
        );
    }

    #[test]
    fn analyze_detects_build_systems() {
        let tmp = TempDir::new().expect("temp dir");
        let root = create_test_repo(&tmp);

        let analyzer = CodebaseAnalyzer::new(&root);
        let analysis = analyzer.analyze().expect("analysis should succeed");

        assert!(
            analysis.build_systems.contains(&"cargo".to_string()),
            "should detect cargo build system"
        );
        assert!(
            analysis.build_systems.contains(&"npm".to_string()),
            "should detect npm build system"
        );
    }

    #[test]
    fn analyze_detects_ownership_signals() {
        let tmp = TempDir::new().expect("temp dir");
        let root = create_test_repo(&tmp);

        let analyzer = CodebaseAnalyzer::new(&root);
        let analysis = analyzer.analyze().expect("analysis should succeed");

        assert!(
            analysis
                .ownership_files
                .iter()
                .any(|s| s.file_path == "Cargo.toml"),
            "should detect Cargo.toml"
        );
        assert!(
            analysis
                .ownership_files
                .iter()
                .any(|s| s.file_path == "README.md"),
            "should detect README.md"
        );
        assert!(
            analysis
                .ownership_files
                .iter()
                .any(|s| s.file_path == ".gitignore"),
            "should detect .gitignore"
        );
    }

    #[test]
    fn analyze_detects_conventions() {
        let tmp = TempDir::new().expect("temp dir");
        let root = create_test_repo(&tmp);

        let analyzer = CodebaseAnalyzer::new(&root);
        let analysis = analyzer.analyze().expect("analysis should succeed");

        assert!(
            analysis.conventions.iter().any(|c| c.area == "build"),
            "should detect build convention"
        );
        assert!(
            analysis.conventions.iter().any(|c| c.area == "linting"),
            "should detect linting convention"
        );
        assert!(
            analysis.conventions.iter().any(|c| c.area == "formatting"),
            "should detect formatting convention"
        );
    }

    #[test]
    fn analyze_detects_integration_points() {
        let tmp = TempDir::new().expect("temp dir");
        let root = create_test_repo(&tmp);

        let analyzer = CodebaseAnalyzer::new(&root);
        let analysis = analyzer.analyze().expect("analysis should succeed");

        // Should detect at least the client integration point
        assert!(
            !analysis.integration_points.is_empty(),
            "should detect integration points"
        );
    }

    #[test]
    fn analyze_detects_mixed_language_risk() {
        let tmp = TempDir::new().expect("temp dir");
        let root = create_test_repo(&tmp);

        // Add a TypeScript package
        let pkg_dir = root.join("packages").join("ui-core");
        fs::create_dir_all(&pkg_dir).unwrap();
        fs::write(pkg_dir.join("package.json"), r#"{ "name": "ui-core" }"#).unwrap();

        let analyzer = CodebaseAnalyzer::new(&root);
        let analysis = analyzer.analyze().expect("analysis should succeed");

        assert!(
            analysis
                .risks
                .iter()
                .any(|r| r.category == RiskCategory::Maintenance),
            "should detect mixed language maintenance risk"
        );
    }

    #[test]
    fn analyze_serializes_to_json() {
        let tmp = TempDir::new().expect("temp dir");
        let root = create_test_repo(&tmp);

        let analyzer = CodebaseAnalyzer::new(&root);
        let analysis = analyzer.analyze().expect("analysis should succeed");

        let json = serde_json::to_string(&analysis).expect("should serialize");
        assert!(json.contains("opensymphony-core"));
        assert!(json.contains("cargo"));

        let deserialized: CodebaseAnalysis =
            serde_json::from_str(&json).expect("should deserialize");
        assert_eq!(deserialized.root_path, analysis.root_path);
        assert_eq!(deserialized.total_files, analysis.total_files);
    }

    #[test]
    fn analyze_returns_error_for_nonexistent_directory() {
        let analyzer = CodebaseAnalyzer::new("/nonexistent/path/that/does/not/exist");
        let result = analyzer.analyze();
        assert!(result.is_err());
        match result.unwrap_err() {
            CodebaseAnalysisError::NotADirectory(p) => {
                assert!(p.contains("nonexistent"));
            }
            other => panic!("expected NotADirectory, got {other:?}"),
        }
    }
}

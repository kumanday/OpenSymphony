use minijinja::{Environment, UndefinedBehavior};
use opensymphony_domain::{AttemptContext, Issue};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(deny_unknown_fields)]
pub struct TrackerConfig {
    pub kind: Option<String>,
    pub project_slug: String,
    pub active_states: Vec<String>,
    pub terminal_states: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PollingConfig {
    #[serde(default = "default_poll_interval_ms")]
    pub interval_ms: u64,
}

impl Default for PollingConfig {
    fn default() -> Self {
        Self {
            interval_ms: default_poll_interval_ms(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceConfig {
    pub root: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct HooksConfig {
    pub after_create: Option<String>,
    pub before_run: Option<String>,
    pub after_run: Option<String>,
    pub before_remove: Option<String>,
    #[serde(default = "default_hook_timeout_ms")]
    pub timeout_ms: u64,
}

impl Default for HooksConfig {
    fn default() -> Self {
        Self {
            after_create: None,
            before_run: None,
            after_run: None,
            before_remove: None,
            timeout_ms: default_hook_timeout_ms(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AgentConfig {
    #[serde(default = "default_max_concurrent_agents")]
    pub max_concurrent_agents: usize,
    #[serde(default = "default_max_turns")]
    pub max_turns: u32,
    #[serde(default = "default_max_retry_backoff_ms")]
    pub max_retry_backoff_ms: i64,
    #[serde(default = "default_stall_timeout_ms")]
    pub stall_timeout_ms: i64,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            max_concurrent_agents: default_max_concurrent_agents(),
            max_turns: default_max_turns(),
            max_retry_backoff_ms: default_max_retry_backoff_ms(),
            stall_timeout_ms: default_stall_timeout_ms(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(deny_unknown_fields)]
pub struct OpenHandsConfig {
    #[serde(default)]
    pub conversation: serde_json::Value,
    #[serde(default)]
    pub mcp: serde_json::Value,
    #[serde(default)]
    pub websocket: serde_json::Value,
    #[serde(default)]
    pub local_server: serde_json::Value,
    #[serde(default)]
    pub transport: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(deny_unknown_fields)]
pub struct WorkflowFrontMatter {
    pub tracker: TrackerConfig,
    #[serde(default)]
    pub polling: PollingConfig,
    #[serde(default)]
    pub workspace: WorkspaceConfig,
    #[serde(default)]
    pub hooks: HooksConfig,
    #[serde(default)]
    pub agent: AgentConfig,
    #[serde(default)]
    pub openhands: OpenHandsConfig,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowDocument {
    pub front_matter: WorkflowFrontMatter,
    pub body: String,
}

#[derive(Debug, Error)]
pub enum WorkflowError {
    #[error("workflow front matter is required")]
    MissingFrontMatter,
    #[error("workflow file is required: {0}")]
    MissingWorkflow(String),
    #[error("workflow front matter is invalid: {0}")]
    InvalidFrontMatter(String),
    #[error("workflow template failed to render: {0}")]
    Render(String),
    #[error("workflow file IO failed: {0}")]
    Io(String),
    #[error("missing required environment variable `{0}`")]
    MissingEnvVar(String),
    #[error("resolved path is invalid: {0}")]
    InvalidPath(String),
}

impl WorkflowDocument {
    pub fn load_from_path(path: &Path) -> Result<Self, WorkflowError> {
        let contents =
            fs::read_to_string(path).map_err(|error| WorkflowError::Io(error.to_string()))?;
        Self::load_from_str(&contents)
    }

    pub fn load_from_str(contents: &str) -> Result<Self, WorkflowError> {
        let Some(rest) = contents.strip_prefix("---\n") else {
            return Err(WorkflowError::MissingFrontMatter);
        };

        let Some((front_matter, body)) = rest.split_once("\n---\n") else {
            return Err(WorkflowError::MissingFrontMatter);
        };

        let front_matter = serde_yaml::from_str::<WorkflowFrontMatter>(front_matter)
            .map_err(|error| WorkflowError::InvalidFrontMatter(error.to_string()))?;

        Ok(Self {
            front_matter,
            body: body.trim_start().to_string(),
        })
    }

    pub fn render_fresh_prompt(&self, issue: &Issue) -> Result<String, WorkflowError> {
        self.render_template(issue, None)
    }

    pub fn render_continuation_prompt(
        &self,
        issue: &Issue,
        attempt: &AttemptContext,
    ) -> Result<String, WorkflowError> {
        self.render_template(issue, Some(attempt))
    }

    fn render_template(
        &self,
        issue: &Issue,
        attempt: Option<&AttemptContext>,
    ) -> Result<String, WorkflowError> {
        let mut environment = Environment::new();
        environment.set_undefined_behavior(UndefinedBehavior::Strict);
        environment
            .add_template("workflow", &self.body)
            .map_err(|error| WorkflowError::Render(error.to_string()))?;

        let template = environment
            .get_template("workflow")
            .map_err(|error| WorkflowError::Render(error.to_string()))?;

        let rendered = match attempt {
            Some(attempt) => {
                template.render(minijinja::context! { issue => issue, attempt => attempt })
            }
            None => template.render(minijinja::context! { issue => issue }),
        };

        rendered.map_err(|error| WorkflowError::Render(error.to_string()))
    }

    pub fn resolve_workspace_root(
        &self,
        base_dir: &Path,
    ) -> Result<Option<PathBuf>, WorkflowError> {
        self.front_matter
            .workspace
            .root
            .as_deref()
            .map(|value| resolve_path(base_dir, value))
            .transpose()
    }
}

pub fn resolve_env_value(value: &str) -> Result<String, WorkflowError> {
    let pattern = Regex::new(r"\$\{([A-Z0-9_]+)\}").expect("regex is valid");
    let mut resolved = String::with_capacity(value.len());
    let mut cursor = 0;

    for capture in pattern.captures_iter(value) {
        let full_match = capture.get(0).expect("full match exists");
        let variable = capture.get(1).expect("capture exists").as_str();
        resolved.push_str(&value[cursor..full_match.start()]);
        let value =
            env::var(variable).map_err(|_| WorkflowError::MissingEnvVar(variable.to_string()))?;
        resolved.push_str(&value);
        cursor = full_match.end();
    }

    resolved.push_str(&value[cursor..]);
    Ok(resolved)
}

pub fn resolve_path(base_dir: &Path, value: &str) -> Result<PathBuf, WorkflowError> {
    let resolved = resolve_env_value(value)?;
    let path = PathBuf::from(&resolved);
    if path.is_absolute() {
        return Ok(path);
    }

    let base = if base_dir.exists() {
        base_dir
            .canonicalize()
            .map_err(|error| WorkflowError::InvalidPath(error.to_string()))?
    } else {
        base_dir.to_path_buf()
    };

    Ok(base.join(path))
}

const fn default_poll_interval_ms() -> u64 {
    5_000
}

const fn default_hook_timeout_ms() -> u64 {
    60_000
}

const fn default_max_concurrent_agents() -> usize {
    4
}

const fn default_max_turns() -> u32 {
    20
}

const fn default_max_retry_backoff_ms() -> i64 {
    300_000
}

const fn default_stall_timeout_ms() -> i64 {
    300_000
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use opensymphony_domain::Issue;
    use tempfile::tempdir;

    fn issue() -> Issue {
        Issue {
            id: "1".to_string(),
            identifier: "ABC-123".to_string(),
            title: "Fix tests".to_string(),
            description: Some("Make the test suite green.".to_string()),
            priority: Some(1),
            state: "Todo".to_string(),
            labels: vec!["bug".to_string()],
            blocked_by: vec![],
            created_at: Utc.with_ymd_and_hms(2026, 3, 21, 20, 0, 0).unwrap(),
            updated_at: Utc.with_ymd_and_hms(2026, 3, 21, 20, 0, 0).unwrap(),
        }
    }

    #[test]
    fn parses_front_matter_and_body() {
        let workflow = WorkflowDocument::load_from_str(
            "---\ntracker:\n  project_slug: demo\n  active_states: [Todo]\n  terminal_states: [Done]\n---\n# Assignment",
        )
        .expect("workflow should parse");

        assert_eq!(workflow.front_matter.tracker.project_slug, "demo");
        assert_eq!(workflow.body, "# Assignment");
    }

    #[test]
    fn fails_without_front_matter() {
        let error =
            WorkflowDocument::load_from_str("# nope").expect_err("front matter should be required");
        assert!(matches!(error, WorkflowError::MissingFrontMatter));
    }

    #[test]
    fn fails_on_unknown_front_matter_keys() {
        let error = WorkflowDocument::load_from_str(
            "---\ntracker:\n  project_slug: demo\n  active_states: [Todo]\n  terminal_states: [Done]\nagent:\n  max_turn: 3\n---\n# Assignment",
        )
        .expect_err("unknown front-matter keys should fail");
        assert!(matches!(error, WorkflowError::InvalidFrontMatter(_)));
    }

    #[test]
    fn fails_on_unknown_openhands_front_matter_keys() {
        let error = WorkflowDocument::load_from_str(
            "---\ntracker:\n  project_slug: demo\n  active_states: [Todo]\n  terminal_states: [Done]\nopenhands:\n  websockett: {}\n---\n# Assignment",
        )
        .expect_err("unknown openhands keys should fail");
        assert!(matches!(error, WorkflowError::InvalidFrontMatter(_)));
    }

    #[test]
    fn renders_distinct_fresh_and_continuation_prompts() {
        let workflow = WorkflowDocument::load_from_str(
            "---\ntracker:\n  project_slug: demo\n  active_states: [Todo]\n  terminal_states: [Done]\n---\n{% if attempt is defined and attempt %}Continue {{ issue.identifier }} attempt {{ attempt.number }}{% else %}Start {{ issue.identifier }}{% endif %}",
        )
        .expect("workflow should parse");

        let fresh = workflow
            .render_fresh_prompt(&issue())
            .expect("fresh template should render");
        let continuation = workflow
            .render_continuation_prompt(
                &issue(),
                &AttemptContext {
                    number: 2,
                    continuation: true,
                },
            )
            .expect("continuation template should render");

        assert_eq!(fresh, "Start ABC-123");
        assert_eq!(continuation, "Continue ABC-123 attempt 2");
    }

    #[test]
    fn fresh_prompts_omit_attempt_from_template_context() {
        let workflow = WorkflowDocument::load_from_str(
            "---\ntracker:\n  project_slug: demo\n  active_states: [Todo]\n  terminal_states: [Done]\n---\n{% if attempt is defined %}defined{% else %}missing{% endif %}",
        )
        .expect("workflow should parse");

        let fresh = workflow
            .render_fresh_prompt(&issue())
            .expect("fresh template should render");
        let continuation = workflow
            .render_continuation_prompt(
                &issue(),
                &AttemptContext {
                    number: 2,
                    continuation: true,
                },
            )
            .expect("continuation template should render");

        assert_eq!(fresh, "missing");
        assert_eq!(continuation, "defined");
    }

    #[test]
    fn fails_on_unknown_template_variables() {
        let workflow = WorkflowDocument::load_from_str(
            "---\ntracker:\n  project_slug: demo\n  active_states: [Todo]\n  terminal_states: [Done]\n---\n{{ issue.unknown }}",
        )
        .expect("workflow should parse");

        let error = workflow
            .render_fresh_prompt(&issue())
            .expect_err("unknown vars should fail");
        assert!(matches!(error, WorkflowError::Render(_)));
    }

    #[test]
    fn resolves_relative_paths_against_base_dir() {
        let tempdir = tempdir().expect("tempdir should exist");
        let resolved = resolve_path(tempdir.path(), "./workspaces").expect("path should resolve");
        assert!(resolved.ends_with("workspaces"));
    }

    #[test]
    fn resolves_environment_variables() {
        env::set_var("WORKSPACE_ROOT", "/tmp/opensymphony");
        let resolved = resolve_env_value("${WORKSPACE_ROOT}/issues").expect("env should resolve");
        assert_eq!(resolved, "/tmp/opensymphony/issues");
    }
}

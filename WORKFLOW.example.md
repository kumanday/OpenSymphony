---
tracker:
  kind: linear
  project_slug: "example-project-slug"
  # tracker.api_key is optional here; the loader falls back to LINEAR_API_KEY.
  active_states:
    - Todo
    - In Progress
    - Human Review
    - Rework
  terminal_states:
    - Done
    - Closed
    - Cancelled
    - Canceled
    - Duplicate

polling:
  interval_ms: 5000

workspace:
  # `~` and exact $VAR/${VAR} tokens are expanded during config resolution.
  # Any non-absolute path here is resolved relative to the repository's WORKFLOW.md.
  root: ~/.opensymphony/workspaces

hooks:
  after_create: |
    git clone --depth 1 git@github.com:example-org/example-repo.git .
  before_run: |
    git status --short
  after_run: |
    git status --short
  before_remove: |
    git status --short
  timeout_ms: 60000

agent:
  max_concurrent_agents: 4
  max_turns: 20
  max_retry_backoff_ms: 300000
  stall_timeout_ms: 300000

openhands:
  transport:
    base_url: "http://127.0.0.1:8000"
    session_api_key_env: null

  local_server:
    enabled: true
    command:
      - python
      - -m
      - openhands.agent_server
      - --host
      - 127.0.0.1
      - --port
      - "8000"
    startup_timeout_ms: 30000
    readiness_probe_path: "/openapi.json"
    env:
      LOG_JSON: "true"
      RUNTIME: process

  conversation:
    reuse_policy: per_issue
    # This path stays relative to the per-issue workspace; parent traversal is rejected.
    persistence_dir_relative: ".opensymphony/openhands"
    max_iterations: 500
    stuck_detection: true
    confirmation_policy:
      kind: NeverConfirm
    agent:
      kind: Agent
      llm:
        # Exact $VAR/${VAR} tokens are resolved before runtime launch.
        model: ${OPENHANDS_MODEL}
        api_key_env: OPENHANDS_LLM_API_KEY
        base_url_env: OPENHANDS_LLM_BASE_URL
      log_completions: false

  websocket:
    enabled: true
    ready_timeout_ms: 30000
    reconnect_initial_ms: 1000
    reconnect_max_ms: 30000
    auth_mode: auto
    query_param_name: session_api_key

  mcp:
    stdio_servers:
      - name: linear
        command:
          - opensymphony
          - linear-mcp
          - --stdio
---

# Assignment

You are working on Linear issue `{{ issue.identifier }}`.

## Issue
- Title: {{ issue.title }}
- State: {{ issue.state }}
- Priority: {{ issue.priority }}
- Labels:
{% for label in issue.labels %}
  - {{ label }}
{% endfor %}

## Description
{% if issue.description %}
{{ issue.description }}
{% else %}
No additional description was provided.
{% endif %}

## Constraints
- Work only inside the current repository workspace.
- Preserve existing coding standards and test conventions.
- Prefer small, reviewable changes.
- Leave the repository in a runnable and testable state when possible.
- Use the Linear MCP tools for ticket comments or state transitions if they are configured.
- Do not wait for interactive approvals because this workflow is intended for unattended execution.

## Expected behavior
- Investigate the issue from the repository state, not only from the ticket text.
- Update code, tests, and documentation as needed.
- Run focused validation and summarize the outcome in the final agent response.
- If the work is blocked, explain the blocker precisely and leave evidence in the repository or ticket comment.

{% if attempt %}
## Continuation metadata
This run is a continuation or retry. Resume from current workspace state and prior conversation context.
Attempt: {{ attempt }}
{% endif %}

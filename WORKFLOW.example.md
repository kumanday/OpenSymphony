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
    # The current readiness probe path only supports bare `http://host:port`
    # origins. `https://`, path-prefixed, query-bearing, and fragment-bearing
    # origins are rejected for now.
    base_url: "http://127.0.0.1:8000"

  local_server:
    # Defaults to `true` when omitted. Explicit `false` is rejected until the
    # runtime can honor workflow-owned local-server disablement instead of still
    # deciding launch behavior from the localhost base URL plus pinned tooling.
    enabled: true
    # Omit `command` to use the pinned launcher chosen by the runtime-owned tooling layer.
    # Explicit launcher overrides are rejected until the runtime can honor workflow-owned commands.
    # Explicit startup-timeout overrides are rejected until the runtime
    # supervisor creation path consumes workflow-owned timeout settings.
    # Explicit readiness-probe-path overrides are rejected until the runtime
    # supervisor launch path consumes workflow-owned probe settings end-to-end.
    # Explicit launcher env overrides are rejected until the runtime
    # supervisor creation path forwards workflow-owned environment variables.

  conversation:
    # Supported values:
    # - per_issue: reuse one conversation across worker lifetimes for the issue.
    # - fresh_each_run: create a new conversation and resend the full workflow prompt each run.
    reuse_policy: per_issue
    # This path stays relative to the per-issue workspace; parent traversal is rejected.
    persistence_dir_relative: ".opensymphony/openhands"
    max_iterations: 500
    stuck_detection: true
    # Defaults to `NeverConfirm` when omitted.
    confirmation_policy:
      kind: NeverConfirm
    agent:
      # Defaults to `Agent` when omitted.
      kind: Agent
      llm:
        # Exact $VAR/${VAR} tokens are resolved before runtime launch.
        # Provider-specific auth/base-url overrides and extra LLM option keys are
        # rejected until the current conversation-create adapter can forward them.
        model: ${LLM_MODEL}
      condenser:
        # Disabled by default when omitted. When enabled, OpenSymphony forwards
        # a fixed `LLMSummarizingCondenser` request that reuses the agent LLM
        # configuration and defaults to `max_size: 240` plus `keep_first: 2`
        # if those thresholds are not set explicitly.
        enabled: true
        max_size: 240
        keep_first: 2
      # Workflow-owned agent extras other than `condenser` such as
      # `log_completions` are rejected until the current conversation-create
      # payload can actually forward them.

  # Workflow-owned websocket enablement and timeout/reconnect knobs are
  # currently rejected until the runtime readiness/reconnect path consumes them.

  # Workflow-owned MCP stdio server declarations are rejected until the current
  # conversation-create adapter can forward `mcp_config` to OpenHands.
  # Provision `opensymphony linear-mcp` through the host tool environment until
  # that adapter wiring lands.
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

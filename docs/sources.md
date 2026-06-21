# Sources and Trust Notes

This file lists the primary references that define the intended behavior for OpenSymphony.

## Trust order

1. OpenAI Symphony `SPEC.md`
2. OpenHands SDK agent-server documentation
3. OpenHands SDK source for `RemoteConversation` when the docs do not state wire-level details clearly
4. FrankenTUI repository documentation
5. User-provided findings file for prior research and framing

## Primary references

### Symphony

- Repository: `https://github.com/openai/symphony`
- Spec: `https://github.com/openai/symphony/blob/main/SPEC.md`

Use these for:

- system goals and non-goals
- orchestration state machine
- workspace invariants
- `WORKFLOW.md` contract
- retry and reconciliation behavior
- optional status-surface boundary

### OpenHands SDK agent-server

Pinned for the current local-MVP implementation branch:

- SDK release: `v1.24.0`
- Release date: `2026-05-27`
- Release notes:
  `https://github.com/OpenHands/software-agent-sdk/releases/tag/v1.24.0`
- Local tooling pin location:
  `tools/openhands-server/`
  Published `opensymphony` embeds this bundle for
  `opensymphony install openhands`.

- Local server guide:
  `https://docs.openhands.dev/sdk/guides/agent-server/local-server`
- Agent-server architecture:
  `https://docs.openhands.dev/sdk/arch/agent-server`
- Agent-server overview:
  `https://docs.openhands.dev/sdk/guides/agent-server/overview`
- Workspace architecture:
  `https://docs.openhands.dev/sdk/arch/workspace`
- Start conversation:
  `https://docs.openhands.dev/sdk/guides/agent-server/api-reference/conversations/start-conversation`
- Get conversation:
  `https://docs.openhands.dev/sdk/guides/agent-server/api-reference/conversations/get-conversation`
- Run conversation:
  `https://docs.openhands.dev/sdk/guides/agent-server/api-reference/conversations/run-conversation`
- Search conversation events:
  `https://docs.openhands.dev/sdk/guides/agent-server/api-reference/events/search-conversation-events`
- Event API reference:
  `https://docs.openhands.dev/sdk/api-reference/openhands.sdk.event`

Use these for:

- local no-Docker development pattern
- per-conversation `workspace.working_dir`
- REST endpoints and payload shapes
- event model
- `ConversationStateUpdateEvent`
- `LLMCompletionLogEvent`
- local vs remote workspace tradeoffs

### OpenHands source used for wire-level clarifications

- `remote_conversation.py`:
  `https://github.com/OpenHands/software-agent-sdk/blob/main/openhands-sdk/openhands/sdk/conversation/impl/remote_conversation.py`

Use this source only for details that are underspecified or absent in the docs, such as:

- current WebSocket URL shape
- readiness barrier behavior
- reconciliation timing
- dedupe and ordering strategy
- reconnect backoff pattern
- current query-param auth fallback
- current `tool_module_qualnames` and `agent_definitions` forwarding in the start-conversation payload

Pinned implementation source:

- release: `v1.24.0`
- server entrypoint: `openhands-agent-server/openhands/agent_server/__main__.py`
- API router: `openhands-agent-server/openhands/agent_server/api.py`
- WebSocket router: `openhands-agent-server/openhands/agent_server/sockets.py`
- server readiness endpoints: `openhands-agent-server/openhands/agent_server/server_details_router.py`

Re-validate all wire-level assumptions against that pinned version before changing the adapter contract.

### OpenHands release notes

- SDK releases:
  `https://github.com/OpenHands/software-agent-sdk/releases`

Use release notes to track:

- API changes
- WebSocket auth changes
- event-model additions
- compatibility risks across versions

### Pinned OpenHands version notes

As of 2026-06-02, this repository pins:

- `openhands-agent-server==1.24.0`
- `openhands-sdk==1.24.0`
- `openhands-tools==1.24.0`
- `openhands-workspace==1.24.0`
- release tag `v1.24.0`
- Python `3.13.12` for the repo-local server environment

Validation sources for this pin:

- PyPI project:
  `https://pypi.org/project/openhands-agent-server/1.24.0/`
- GitHub release:
  `https://github.com/OpenHands/software-agent-sdk/releases/tag/v1.24.0`

The current local supervisor assumptions validated against this pin are:

- the server still starts with `python -m openhands.agent_server`
- the CLI still accepts `--host` and `--port`
- the default bind host remains broader than loopback, so OpenSymphony keeps the
  loopback-only wrapper
- REST auth uses the `X-Session-API-Key` header when session API keys are configured
- the SDK remote client still defaults WebSocket auth to the `session_api_key`
  query parameter when an API key is present
- the server also accepts WebSocket header auth, with query-param auth taking
  precedence when both are present
- `openhands-sdk==1.24.0` exposes
  `LLM.subscription_login(vendor, model, force_login, open_browser, auth_method,
  **llm_kwargs)` for OpenAI ChatGPT/Codex subscription login. Its OpenAI path
  constructs an `LLM` with `model="openai/<codex-model>"`,
  `base_url="https://chatgpt.com/backend-api/codex"`, the OAuth access token as
  the LLM API key, Codex headers, `litellm_extra_body.store=false`, and
  streaming enabled.
- OpenSymphony accepts any non-empty bare model name or `openai/...` model name
  in subscription mode so newer pinned SDK releases can add Codex-capable models
  without a Rust whitelist change. Non-OpenAI provider prefixes remain rejected.

When bumping this version, re-validate the launch surface, readiness probe, HTTP
contract assumptions, and WebSocket notes before changing the repo pin.

### OpenHands skills and context loading

- Agent skills guide:
  `https://docs.openhands.dev/sdk/guides/skill`
- General skills overview:
  `https://docs.openhands.dev/overview/skills`
- Repository agent guidance:
  `https://docs.openhands.dev/overview/skills/repo`

Use these for:

- repo-root `AGENTS.md`
- project skill loading
- `.agents/skills/` conventions

### OpenHands PR review workflow

- Workflow:
  `.github/workflows/ai-pr-review.yml`
- Extensions repository:
  `https://github.com/OpenHands/extensions`
- PR review action pin:
  `75e39288f6c2366b75cf290441da06e187395f63`

Use this pin for the automated advisory AI PR review workflow. The review
action installs the current `openhands-sdk` package at runtime, so the action
pin must remain compatible with the SDK's current project-skill loading API.
Re-validate the action import path and repository skill loading behavior before
changing the pin.

### OpenHands sandboxing

- Process sandbox:
  `https://docs.openhands.dev/openhands/usage/sandboxes/process`
- Sandbox overview:
  `https://docs.openhands.dev/openhands/usage/sandboxes/overview`

Use these for:

- local trusted-mode safety posture
- later hardening discussions
- documentation of host-access risk

### FrankenTUI

- Repository:
  `https://github.com/Dicklesworthstone/frankentui`

Use this for:

- inline mode assumptions
- diff-based rendering model
- pane workspace capabilities
- current dependency strategy

## Sources that are intentionally out of scope for the MVP runtime contract

These documents are useful for understanding the broader OpenHands product, but they are not the protocol contract for OpenSymphony's local MVP runtime adapter:

- OpenHands web-app Socket.IO WebSocket docs
- `openhands serve` GUI server docs
- ACP client protocols
- web-app REST docs that are not part of the SDK agent-server surface

## User-provided findings incorporated here

User file:

- `/mnt/data/symphony-design-opencode-analysis.md`

Key takeaways carried forward:

- Symphony should be treated as a harness-agnostic orchestration design.
- Workspace, retry, reconciliation, and tracker semantics stay Symphony-owned.
- Session-oriented harness integration is the right abstraction.
- Real-time runtime streaming is important enough to build in early rather than retrofit later.

<!-- BEGIN OPENSYMPHONY MANAGED MEMORY SYNC -->

## Current model

- COE-253 contributed: PR #19: COE-253: OpenHands Runtime Adapter (merge `911b0b4`)
- COE-255 contributed: PR #4: COE-255: add control plane and FrankenTUI slice
- COE-256 contributed: PR #1: COE-257: tighten hosted deployment guidance
- COE-261 contributed: PR #83: Add memory init and mapped docs sync
- COE-262 contributed: PR #34: COE-262: Harden OpenHands REST client contract (merge `0e2be26`)
- COE-265 contributed: PR #36: COE-265: WebSocket event stream, reconciliation, and recovery (merge `d78a8ce`)

## Important invariants

- Preserve the behavior described in the recent captured changes unless current code and tests show it has changed.
- Use capsule source refs to inspect the original PR or Linear issue when context is ambiguous.

## Operational flow

- No generated diagram requested for this sync.

## Known gotchas

- No area-specific gotchas were inferred from the selected memory.

## Recent changes

- COE-253: OpenHands Runtime Adapter
- COE-255: Observability and FrankenTUI
- COE-256: Validation and Local Operations
- COE-261: Local agent-server supervisor
- COE-262: REST client and conversation contract
- COE-265: WebSocket event stream, reconciliation, and recovery
- COE-266: Issue session runner
- COE-269: Control-plane API and snapshot store
- COE-271: FrankenTUI operator client
- COE-272: Fake OpenHands server and protocol contract suite
- COE-273: Live local end-to-end suite
- COE-274: CLI packaging, doctor, and local operations docs
- COE-275: Remote agent-server mode and auth hardening
- COE-280: Support workflow-owned OpenHands auth, provider, and launcher overrides at runtime
- COE-281: Support path-bearing OpenHands base URLs and MCP config at runtime
- COE-282: Support workflow-owned OpenHands conversation reuse policy at runtime
- COE-287: Add opensymphony debug command for conversational session debugging
- COE-294: Detect LLM config changes and rehydrate conversations with updated env vars
- COE-382: Add supply-chain and security audits to CI
- COE-383: Decompose oversized session and TUI modules into focused submodules
- COE-384: Expand error-path tests for Linear client and workspace hooks
- COE-385: Resolve runtime tracking TODO in OpenHands session runner
- COE-386: Wire cargo-llvm-cov coverage reporting and regression floor into CI
- COE-387: Audit tracing spans and diagnostics for secret leakage
- COE-389: Current Gateway Inventory And Vocabulary
- COE-390: Gateway Schemas And Stream Feasibility
- COE-391: Gateway Module, Capabilities, And Dashboard Snapshot
- COE-392: Task Graph, Run Detail, File, And Diff Read APIs
- COE-393: Event Journal And Stream Broker
- COE-395: Planning Artifact Schema And Session Service
- COE-396: Action Receipts And Initial Run Actions
- COE-401: Web App Entry And Deployment Modes
- COE-406: Repository, Linear, And Research Analysis
- COE-407: Browser Transport And Remote Stream Protocols
- COE-413: Implementation Plan Generator Stage
- COE-415: Milestone, Issue, And Sub-Issue Compiler
- COE-416: Dependency Graph And Plan Checks
- COE-417: Planning Workspace UI
- COE-419: Hosted Auth Placeholders And Web Parity
- COE-429: Codex Approvals And Harness/Model Selection
- COE-473: Desktop task graph dependency and run detail parity
- COE-479: Codex Debug Session Resume
- COE-480: Run Detail Metrics And Density

## Source refs

- COE-253
- COE-255
- COE-256
- COE-261
- COE-262
- COE-265
- COE-266
- COE-269
- COE-271
- COE-272
- COE-273
- COE-274
- COE-275
- COE-280
- COE-281
- COE-282
- COE-287
- COE-294
- COE-382
- COE-383
- COE-384
- COE-385
- COE-386
- COE-387
- COE-389
- COE-390
- COE-391
- COE-392
- COE-393
- COE-395
- COE-396
- COE-401
- COE-406
- COE-407
- COE-413
- COE-415
- COE-416
- COE-417
- COE-419
- COE-429
- COE-473
- COE-479
- COE-480

<!-- END OPENSYMPHONY MANAGED MEMORY SYNC -->

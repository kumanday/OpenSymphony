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

- release: `v1.14.0`
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

As of 2026-03-22, this repository pins:

- `openhands-agent-server==1.14.0`
- `openhands-sdk==1.14.0`
- `openhands-tools==1.14.0`
- `openhands-workspace==1.14.0`
- release tag `v1.14.0`
- Python `3.12.x` for the repo-local server environment

Validation sources for this pin:

- PyPI project:
  `https://pypi.org/project/openhands-agent-server/1.14.0/`
- GitHub release:
  `https://github.com/OpenHands/software-agent-sdk/releases/tag/v1.14.0`

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

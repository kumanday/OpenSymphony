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

- SDK release: `v1.14.0`
- Release date: `2026-03-13`
- Release notes:
  `https://github.com/OpenHands/software-agent-sdk/releases/tag/v1.14.0`
- Local tooling pin location:
  `tools/openhands-server/`

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

Pin the OpenHands version before implementation and re-validate all wire-level assumptions against that pinned version.

### OpenHands release notes

- SDK releases:
  `https://github.com/OpenHands/software-agent-sdk/releases`

Use release notes to track:

- API changes
- WebSocket auth changes
- event-model additions
- compatibility risks across versions

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

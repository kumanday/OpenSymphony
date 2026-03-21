---
id: OSYM-202
title: Build REST client and conversation contract
type: feature
area: agent-runtime
priority: P0
estimate: 4d
milestone: M2 OpenHands runtime adapter
parent: OSYM-200
depends_on:
  - OSYM-101
  - OSYM-103
blocks:
  - OSYM-203
  - OSYM-204
  - OSYM-501
  - OSYM-601
project_context:
  - AGENTS.md
  - README.md
  - docs/openhands-agent-server.md
  - docs/workspace-and-lifecycle.md
repo_paths:
  - crates/opensymphony-openhands/
definition_of_ready:
  - OSYM-101 and OSYM-103 are merged
  - Pinned API fields are documented
---

# OSYM-202: Build REST client and conversation contract

## Summary
Implement the typed REST client for the minimal agent-server contract OpenSymphony needs, including conversation create, state read, event post, run trigger, and event search.

## Scope
- Define typed request and response models for the minimal API subset
- Serialize `workspace.working_dir`, `conversation_id`, and `persistence_dir` correctly
- Support auth configuration hooks needed for local and future remote modes
- Map HTTP and serialization failures into stable runtime error categories

## Out of scope
- Full coverage of every agent-server endpoint
- Leaking raw reqwest or HTTP types into orchestrator code

## Deliverables
- REST client facade
- Minimal typed wire models
- Error mapping layer

## Acceptance criteria
- Conversation creation works against the pinned server
- The client can fetch conversation state, post user events, trigger runs, and search events
- The public API exposed to the orchestrator is transport-agnostic

## Test plan
- Serialization tests for create and run payloads
- Fake-server request/response tests
- Negative tests for auth and malformed payload handling

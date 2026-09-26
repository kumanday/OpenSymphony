---
id: OSYM-840
title: Multi-Harness Debug Attachment Core
milestone: 'M13: ACP Debugging And IDE Attach'
priority: 2
estimate: 8
blockedBy:
- OSYM-902
blocks:
- OSYM-907
- OSYM-843
areas:
- debugging
- acp
- cli
parent: null
---

## Summary

Refactor debug attachment around the shared runtime owner so every supported ACP profile is a first-class IDE target and existing native debug paths retain their behavior.

## Scope

### In scope

- Resolve the exact issue workspace from issue reference or ACP cwd and verify its repository/run/generation envelope and persisted harness/profile/session binding.
- Use opaque native session IDs and adapter capabilities in the shared attachment core. ACP attaches through the runtime owner control/event seam without launching a second agent.
- Distinguish live attach, capability-gated restoration, transcript inspection and fresh-context reset. Report unsupported restoration clearly; a debug attach never silently starts a replacement conversation.
- Reuse active/archived/legacy OpenHands store resolution and native attach/reconcile. Preserve Codex thread unarchive, codex resume and --app deep-link compatibility.
- Expose attachment status, event subscription and commands for terminal and IDE consumers; keep secrets and native protocol types inside their adapters.

### Out of scope

- IDE ACP wire handling is OSYM-841; exclusive writer transfer is OSYM-907; a new native Codex-to-ACP bridge is not required to preserve native Codex debug.

## Deliverables

- Harness-neutral attachment resolution and host-owned session access.
- Regression coverage for native debug paths and multiple ACP profiles.

## Acceptance Criteria

- [ ] Two different ACP profiles attach through the same core with no vendor-specific debug branch and no new harness process for an existing live owner.
- [ ] Exact workspace identity and runtime envelope validation reject parent/nested/repository/store paths and stale or mismatched metadata.
- [ ] Nonpersistent ACP sessions remain attachable while the host retains them; after loss, the UI distinguishes unavailable context from transcript inspection.
- [ ] OpenHands active/archived/legacy stores and Codex unarchive/resume/--app paths preserve their existing identities and workspace binding.
- [ ] No parallel debug manifest or session database is introduced.

## Test Plan

- Run workspace binding and debug-session regression tests for ACP, OpenHands and Codex, including profile changes and missing owners.
- Test separate-process attachment uses the owner seam and cannot create a competing runtime.

## Context

- docs/specs/opensymphony-acp-debugging-spec.md; docs/specs/acp-harness-adapter.md.
- crates/opensymphony-cli/src/debug_session.rs; crates/opensymphony-openhands/src/conversation_store.rs; runtime envelope and control-plane APIs.

## Definition of Ready

- [ ] Linked specifications and repository contracts have been read.
- [ ] Required dependencies are merged and their evidence is available.
- [ ] The implementation can begin using this task and its referenced sources.

## Notes

M12.99 supplies executable ACP sessions. Native debug compatibility remains capability-explicit; multi-profile ACP support is required.

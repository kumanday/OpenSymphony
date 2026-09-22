---
id: OSYM-844
title: Default Multi-Harness IDE Debug UX And CLI Compatibility
milestone: 'M13: ACP Debugging And IDE Attach'
priority: 3
estimate: 5
blockedBy:
- OSYM-845
blocks: []
areas:
- debugging
- cli
- desktop
parent: null
---

## Summary

Make the qualified IDE flow the default debug experience while preserving explicit terminal and native harness debugging options.

## Scope

### In scope

- Route opensymphony debug <issue-key> to the capability-appropriate IDE flow after OSYM-845 qualification.
- Preserve --cli terminal behavior, native Codex resume/unarchive and --app deep links, and explicit noninteractive --acp-stdio semantics.
- Provide usable fallback guidance for missing Zed, unavailable owner, lost nonpersistent context or unsupported native IDE capability.
- Update help, operations and debugging docs with command precedence and profile/capability behavior.

### Out of scope

- Removing terminal debug or native harness alternatives; hosted browser IDE integration.

## Deliverables

- Qualified default command routing, compatibility flags and documented fallback behavior.

## Acceptance Criteria

- [ ] The default debug flow works for both qualified ACP profiles and uses accurate capabilities for native routes.
- [ ] --cli, --app and --acp-stdio retain their documented semantics; incompatible flag combinations fail clearly.
- [ ] No fallback silently resets agent context, creates a second live runtime, or bypasses writer ownership.
- [ ] OSYM-845 evidence is complete before the default changes; CLI help and operations examples match implemented behavior.

## Test Plan

- Run command dispatch/help and native debug regression tests.
- Repeat the default entrypoint smoke path using the already qualified profiles; verify missing-editor fallback.

## Context

- docs/specs/opensymphony-acp-debugging-spec.md: commands and rollout.
- OSYM-845 qualification gate; crates/opensymphony-cli/src/debug_session.rs and CLI argument declarations.

## Definition of Ready

- [ ] Linked specifications and repository contracts have been read.
- [ ] Required dependencies are merged and their evidence is available.
- [ ] The implementation can begin using this task and its referenced sources.

## Notes

This is the last M13 task: validation precedes the default UX switch.

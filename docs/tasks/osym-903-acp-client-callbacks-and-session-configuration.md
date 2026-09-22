---
id: OSYM-903
title: ACP Client Callbacks And Session Configuration
milestone: 'M12.99: ACP Harness Runtime Adapter'
priority: 2
estimate: 8
blockedBy:
- OSYM-900
blocks:
- OSYM-904
- OSYM-906
- OSYM-841
areas:
- acp
- runtime
- workspace
parent: null
---

## Summary

Implement the ACP client facilities that allow different harnesses to execute within the bound issue workspace and use scoped tools consistently.

## Scope

### In scope

- Implement fs/read_text_file and fs/write_text_file with specified text/line semantics, containment, symlink checks and limits.
- Implement terminal create/output/wait_for_exit/kill/release with session-bound IDs, cwd/env policy, bounded output, cancellation and teardown.
- Attach existing scoped MCP memory/tool servers using negotiated supported transports; apply advertised model/config options and legacy modes before prompting and consume updates.
- Gate advertised filesystem/terminal/auth/elicitation capabilities on implemented handlers and policy. Preserve callback ordering while long waits run asynchronously.
- Keep execution facilities owned by the host when an IDE later attaches; IDE-provided environment, filesystems, terminals or MCP entries cannot silently replace the running session contract.

### Out of scope

- Operator approval/input responses are OSYM-904; remote transports and a new sandbox.

## Deliverables

- Production filesystem/terminal callbacks and configuration/MCP setup.
- Callback capability matrix and resource lifecycle tests.

## Acceptance Criteria

- [ ] A harness completes a workspace read/write and terminal command with correct output, exit state and release handling; another session cannot reuse its handles.
- [ ] Path traversal, symlink escape, unauthorized cwd and oversized requests are rejected, including nonexistent write targets.
- [ ] Unsupported explicit model/config choices or required MCP transport fail clearly rather than silently selecting defaults.
- [ ] Cancellation and host shutdown resolve or expire outstanding callbacks and reap owned children without blocking RPC dispatch.
- [ ] Late attachment leaves the established execution environment and scoped memory grants intact.

## Test Plan

- Run focused ACP callback tests for line ranges, output truncation, terminal exit races, release-before-exit, cancellation and cross-session access.
- Exercise negotiated capability absence, model/config changes and scoped MCP attachment through the executable fake peer.

## Context

- docs/specs/acp-harness-adapter.md: client callbacks and effective capabilities.
- Existing workspace containment, scoped memory/environment and subprocess utilities; official ACP v1 filesystem, terminals and session-config-options contracts.

## Definition of Ready

- [ ] Linked specifications and repository contracts have been read.
- [ ] Required dependencies are merged and their evidence is available.
- [ ] The implementation can begin using this task and its referenced sources.

## Notes

The local agent remains a trusted host process. Callback containment must not be described as a sandbox.

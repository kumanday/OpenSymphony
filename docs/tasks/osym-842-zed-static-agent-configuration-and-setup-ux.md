---
id: OSYM-842
title: Multi-Harness IDE Setup And Capability Guidance
milestone: 'M13: ACP Debugging And IDE Attach'
priority: 3
estimate: 3
blockedBy:
- OSYM-841
blocks:
- OSYM-843
areas:
- debugging
- docs
- acp
parent: null
---

## Summary

Provide one static Zed integration and harness-aware setup guidance that works across supported ACP profiles without per-vendor editor configuration.

## Scope

### In scope

- Document the static opensymphony debug --acp-stdio command and exact issue-workspace selection; verify current Zed agent_servers syntax.
- Explain the attached harness/profile/session, negotiated capabilities, observe/control state, approval destination and host-bound execution facilities.
- Provide recovery guidance for missing editor/owner/manifests, invalid cwd/binding, unsupported extensions/restoration, nonpersistent lost sessions and OpenHands store mismatch.
- Document the ACP client contract for other IDEs while qualifying actual editor support only with evidence.

### Out of scope

- Per-issue agent_servers entries and undocumented editor auto-start APIs.

## Deliverables

- Copyable static Zed setup and multi-harness capability/troubleshooting guide.

## Acceptance Criteria

- [ ] One configuration attaches to issues from two ACP profiles with no provider-specific setup changes.
- [ ] Guidance accurately distinguishes live attach, restored context and transcript inspection, including required host availability.
- [ ] Users can identify who owns control and where to answer a pending request; unsupported IDE extensions have a concrete fallback/error.
- [ ] Examples use the exact issue workspace and expose no secret environment values.

## Test Plan

- Validate the JSON settings example and perform a real Zed setup smoke check.
- Run relevant CLI guidance tests and docs link checks.

## Context

- docs/specs/opensymphony-acp-debugging-spec.md: IDE setup; docs/operations.md.
- OSYM-841 capability/errors contract; current official Zed external-agent documentation.

## Definition of Ready

- [ ] Linked specifications and repository contracts have been read.
- [ ] Required dependencies are merged and their evidence is available.
- [ ] The implementation can begin using this task and its referenced sources.

## Notes

Zed is the initial qualified editor. The server protocol remains usable by other compliant clients with supported capabilities.

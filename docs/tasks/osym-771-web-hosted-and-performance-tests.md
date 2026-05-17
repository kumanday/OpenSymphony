---
id: OSYM-771
title: Web, Hosted, And Performance Tests
milestone: "M13: Hardening And Release Quality"
priority: 1
estimate: 8
blockedBy: ["OSYM-717", "OSYM-741", "OSYM-753", "OSYM-765", "OSYM-770"]
blocks: ["OSYM-772"]
parent: null
---

## Summary

Add web E2E, hosted E2E, and performance gates for snapshots, streams, terminal/log throughput, task graph loading, and planning artifact rendering.

## Scope

### In scope

- Add web E2E tests for web client load, gateway connection, reconnect, task graph, run views, and planning draft.
- Add hosted E2E tests for login, project access, Linear sync, disconnect persistence, second-user visibility, unauthorized-user blocking, and workspace cleanup.
- Add performance gates for dashboard snapshot latency, event stream latency, terminal/log throughput, UI frame responsiveness, large task graph loads, and planning artifact renders.
- Add CI reports for performance metrics.

### Out of scope

- Full security review.
- Accessibility remediation.
- Public release notes.

## Deliverables

- Web E2E suite.
- Hosted E2E suite.
- Performance benchmark suite.
- CI performance report.

## Acceptance Criteria

- [ ] Browser reconnect recovers committed event history.
- [ ] Hosted E2E proves runs continue after clients disconnect.
- [ ] Performance gates fail clearly when latency, throughput, or responsiveness targets regress.

## Test Plan

- Run web E2E tests against a local/external gateway.
- Run hosted E2E tests against hosted alpha fixtures or environment.
- Run benchmark suite and inspect generated CI reports.

## Context

- Source sections: `docs/host-client-implementation_plan.md` P10.3 through P10.5.
- Performance gates should use representative active-run, terminal/log, large-project, and planning-session fixtures.

## Definition of Ready

- [ ] Hidden assumptions from prior discussion are written down.
- [ ] Required files, docs, and dependencies are explicitly referenced.
- [ ] A coding agent could begin execution without additional planning context.

## Notes

Keep benchmark thresholds documented and adjustable as implementation data improves.

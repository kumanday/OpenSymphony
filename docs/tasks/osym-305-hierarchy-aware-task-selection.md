---
id: OSYM-305
title: Implement hierarchy-aware task selection
type: feature
area: orchestration
priority: P1
estimate: 3d
milestone: M3 Symphony orchestration core
parent: OSYM-304
depends_on:
  - OSYM-302
  - OSYM-304
project_context:
  - docs/architecture.md
  - docs/symphony-spec-alignment.md
  - docs/tasks/osym-103-domain-model-and-orchestrator-state-machine.md
  - docs/tasks/osym-304-orchestrator-scheduler-retries-and-reconciliation.md
repo_paths:
  - crates/opensymphony-domain/
  - crates/opensymphony-linear/
  - crates/opensymphony-orchestrator/
definition_of_ready:
  - OSYM-302 and OSYM-304 are merged
  - Linear GraphQL `children` schema for issue hierarchy is documented
  - Decision made on parent vs sub-issue dispatch ordering
---

# OSYM-305: Implement hierarchy-aware task selection

## Summary

Extend the orchestrator's candidate selection logic to respect Linear's parent/sub-issue hierarchy. Ensure parent issues are not dispatched until their sub-issues are complete, and encode this dependency in the task selection pipeline.

## Background

The current orchestrator (OSYM-304) treats all issues as independent tasks. However, Linear supports parent issues with sub-issues that represent hierarchical decomposition of work. The orchestrator should:

1. Not dispatch a parent issue until all its sub-issues reach terminal states
2. Prefer leaf issues (sub-issues without children) over parent issues when both are ready
3. Handle the case where sub-issues may be added dynamically after parent creation

## Scope

### In Scope

- Add `sub_issues` field to the normalized issue model
- Extend Linear GraphQL queries to fetch the `children` relationship
- Add hierarchy check to candidate filtering logic
- Update `should_dispatch_issue?` to skip parents with incomplete sub-issues
- Add topological sorting: prefer leaves over parents when both are ready
- Handle dynamic sub-issue creation (parent may gain new sub-issues after being created)

### Out of Scope

- Automatic sub-issue creation (that's a tracker write operation, not orchestration)
- Parent issue auto-completion when sub-issues complete (Linear handles this)
- Complex dependency graphs beyond parent/child (use blockers for that)

## Deliverables

1. **Domain model updates** (`opensymphony-domain`):
   - Add `sub_issues: [IssueRef]` field to normalized issue
   - Add `parent_id: Option<String>` field to normalized issue
   - Define `IssueRef` type with `id`, `identifier`, `state`

2. **Linear adapter updates** (`opensymphony-linear`):
   - Extend GraphQL query to fetch `children` with their states
   - Extend `normalize_issue` to extract sub-issue references
   - Add test fixtures for hierarchical issue structures

3. **Orchestrator updates** (`opensymphony-orchestrator`):
   - Add `parent_issue_blocked_by_incomplete_children?` check
   - Update `sort_issues_for_dispatch` to prefer leaves
   - Add hierarchy-aware candidate filtering

4. **Documentation**:
   - Update architecture.md with hierarchy handling
   - Add test cases for parent/sub-issue scenarios

## Acceptance Criteria

- [ ] Parent issue with open sub-issues is not dispatched
- [ ] Parent issue with all sub-issues in terminal states is eligible for dispatch
- [ ] Leaf issues (no sub-issues) are preferred over parent issues when both are ready
- [ ] Dynamic sub-issue addition is handled (parent becomes blocked when new sub-issue added)
- [ ] Hierarchy check works alongside existing blocker check
- [ ] Performance: hierarchy check doesn't add significant latency to poll cycle

## Test Plan

### Unit Tests

- `normalize_issue` extracts sub-issues correctly from GraphQL response
- `parent_issue_blocked_by_incomplete_children?` returns true when sub-issues are active
- `parent_issue_blocked_by_incomplete_children?` returns false when all sub-issues are terminal
- `sort_issues_for_dispatch` orders leaf issues before parent issues

### Integration Tests

- Fake Linear returns hierarchical issues; orchestrator filters parents correctly
- Parent issue becomes dispatchable after its sub-issues complete
- New sub-issue added to parent causes parent to be skipped in next poll

### Scenario Tests

1. **Simple hierarchy**: Parent P1 with sub-issues S1, S2. S1 and S2 must complete before P1 is eligible.
2. **Nested hierarchy**: Parent P1 with sub-issue S1, which has sub-issue SS1. SS1 must complete before S1, which must complete before P1.
3. **Dynamic addition**: Parent P1 is eligible (no sub-issues). Sub-issue S1 is added. P1 becomes ineligible until S1 completes.

## Implementation Notes

### GraphQL Query Extension

Add to the existing issue query:

```graphql
children(first: 50) {
  nodes {
    id
    identifier
    state {
      name
    }
  }
}
```

### Hierarchy Check Logic

```rust
fn parent_issue_blocked_by_incomplete_children?(
    issue: &Issue,
    terminal_states: &HashSet<String>
) -> bool {
    // If no sub-issues, not blocked
    if issue.sub_issues.is_empty() {
        return false;
    }
    
    // Check if any sub-issue is in a non-terminal state
    issue.sub_issues.iter().any(|sub| {
        !terminal_states.contains(&sub.state)
    })
}
```

### Sorting Logic

Update `sort_issues_for_dispatch` to include hierarchy depth:

```rust
// Prefer: higher priority, earlier created, more leaf-like (fewer sub-issues)
(priority_rank, created_at, sub_issue_count, identifier)
```

## Related Work

- **OSYM-302**: Linear read adapter - provides the GraphQL client
- **OSYM-304**: Orchestrator scheduler - provides the candidate selection pipeline
- **Upstream Elixir**: The reference implementation handles `blocked_by` but not parent/child hierarchy. This task extends beyond the reference.

## References

- Linear GraphQL API: `children` field on Issue type
- Symphony spec: "The orchestrator is the source of truth for scheduling state"
- AGENTS.md: "Parent issues and sub-issues represent hierarchical decomposition"

---
name: convert-tasks-to-linear
description: |
  Use this skill when a repository contains a markdown implementation plan that
  needs to become real Linear issues with correct parent/sub-issue relationships,
  blockers, and Definition of Ready context.
---

# Convert Markdown Task Plans to Linear Parent Issues and Sub-Issues

## Purpose

Use this skill when a repository contains a markdown implementation plan that needs to become real Linear issues with correct parent/sub-issue relationships, blockers, and Definition of Ready context.

The goal is not merely to copy text into Linear. The goal is to create a clean, navigable issue tree that preserves dependencies, removes document-only IDs, and leaves each Linear issue ready for an implementation subagent with independent context.

## Inputs required

Collect these inputs before creating issues:

- path to the implementation-plan markdown file
- workspace or team slug in Linear
- project name in Linear, if applicable
- repository root path
- supporting docs referenced by the plan
- labels, priority mapping, and naming conventions required by the team

## Linear Project overview content

Linear Projects support an **overview** field that serves as a guide and live dashboard for the project. This overview should contain high-level project information, especially regarding its implementation plan, and can include references to key issues, which will display with live status icons.

### Purpose of the project overview

- Provide a central landing page for the project with summary information
- Explain the implementation plan
- Act as a live dashboard where referenced issues show current status
- Document project goals, scope, and key deliverables
- Link to important resources (docs, repos, external references)

### Content to include in the overview

When creating or updating a Linear Project as part of task conversion:

- **Project summary**: Brief description of what the project accomplishes
- **Key milestones**: List of project milestones with brief descriptions
- **Parent issue references**: Links to major parent issues (these render with live status)
- **Definition of done**: Criteria for project completion
- **Resources**: Links to relevant documentation, repositories, and external tools
- **Timeline**: High-level schedule or sequence of phases

### Live dashboard behavior

Issue references in the overview field automatically display their current status. This means:
Include references to the most important parent issues and milestones in the overview to maximize dashboard utility.

## Outputs required

A successful conversion produces:

- parent issues created in Linear
- sub-issues created under the correct parent issues
- blocker relationships set through Linear metadata
- issue bodies rewritten so they reference actual Linear issue IDs or canonical Linear URLs
- a local mapping table from document IDs to created Linear issue IDs and URLs
- no stale document-only IDs left in created issue bodies unless explicitly preserved in a migration note

## Preferred OpenSymphony Linear assets

In an OpenSymphony-managed repository, use the checked-in Linear helper and
query files instead of ad hoc inline GraphQL for the core conversion steps:

- create parent issues and sub-issues with `.agents/skills/linear/queries/issue_create.graphql`
- rewrite created issue bodies and metadata with `.agents/skills/linear/queries/issue_update.graphql`
- attach blocker or related links with `.agents/skills/linear/queries/issue_relation_create.graphql`

This keeps task conversion on the same supported GraphQL surface that
`WORKFLOW.md` and the repo-local `linear` skill expect.

## Core rules

1. Document-only IDs such as `TASK-001`, `BENCH-014`, or similar are not issue IDs.
2. Never leave document-only IDs in created Linear descriptions, blocker fields, or related-issue text.
3. Create parent issues first.
4. Create sub-issues second.
5. Apply blockers using Linear blocker metadata, not plain text.
6. After creation, update every issue body so internal references point to the real created Linear issues.
7. Use `parent issue` and `sub-issue` terminology. Do not use `epic` unless the source system explicitly requires it.
8. Use canonical Linear issue URLs only: `https://linear.app/<workspace>/issue/<ISSUE-ID>/<slug>`.
9. Never construct broken URLs that splice parent and sub-issue identifiers together.
10. Validate the finished issue graph before declaring success.
11. Never create an issue with a blocker reference to another issue that has not already been created and confirmed in Linear.
12. Never batch together dependent issue creations in the same concurrent block.
13. Create dependency-free issues concurrently only when none of them reference each other by blocker, parent/sub-issue relationship, or inline related-issue text that must be resolved at creation time.
14. Treat issue creation and blocker assignment as separate phases unless all referenced issue IDs are already known and verified.

## Parent/Sub-Issue Work Boundaries

When creating parent issues and sub-issues, establish clear work boundaries to prevent duplicate work and ambiguity:

### Parent issue scope

Parent issues should contain:
- **High-level acceptance criteria** that define when the parent is complete
- **References to all sub-issues** that must be completed
- **Integration requirements** that can only be verified after sub-issues merge
- **NO implementation details** that belong in sub-issues

Parent issues should NOT contain:
- Detailed implementation steps (belongs in sub-issues)
- Code changes (work happens in sub-issue branches)
- Unit tests for specific components (belongs in sub-issues)

### Sub-issue scope

Sub-issues should contain:
- **Specific, implementable scope** with clear deliverables
- **Acceptance criteria** that can be verified independently
- **Test plan** for the specific component
- **Definition of Done** that includes PR merge to main

Sub-issues should NOT contain:
- Dependencies on other sub-issues being merged first (use blockers)
- Integration testing that requires parent completion
- Vague references to "parent issue work" - be specific

### Preventing duplicate work

To avoid the anti-pattern where work is done in the parent issue while child issues remain open:

1. **Parent issue acceptance criteria must reference sub-issue completion**
   - Example: "All sub-issues (COE-263, COE-264, COE-267, COE-268, COE-270) are in Done state"
   - Example: "Integration tests pass after all sub-issue PRs are merged"

2. **Sub-issues must have independent Definition of Ready**
   - Each sub-issue should be implementable without reading the parent issue body
   - Cross-reference related sub-issues but don't duplicate their content

3. **Parent issue state reflects sub-issue aggregate state**
   - Parent stays in Todo until first sub-issue moves to In Progress
   - Parent moves to In Progress when integration work begins
   - Parent cannot move to Done until all sub-issues are Done

4. **Orchestrator behavior with hierarchy**
   - Parent issues are blocked from dispatch while any sub-issue is in non-terminal state
   - Sub-issues are preferred over parent issues in dispatch ordering
   - This ensures children complete before parent integration work begins

### Avoiding circular dependencies

**Critical**: Never create a blocker relationship between a parent and its own child. This creates a deadlock:

```
COE-268 (Parent)          COE-277 (Child)
     │                          │
     │◄──── blocked by ──────────┘
     │                          │
     └──── sub-issue ───────────►│
```

**The deadlock**:
1. Parent can't dispatch because child is incomplete (hierarchy check)
2. Child can't dispatch because it's blocked by parent (blocker check)
3. Result: Neither can ever be dispatched

**Correct patterns**:

| Relationship | Meaning | Blocker Needed? |
|--------------|---------|-----------------|
| Parent → Child | Parent contains/aggregates child work | NO - hierarchy handles this |
| Child A → Child B | Sibling dependency (A must complete before B) | YES - use blockers |
| Issue → External | Depends on external issue | YES - use blockers |

**Rule**: The parent-child relationship already implies "parent depends on child completion". Adding a blocker from child to parent creates a circular dependency.

### Example: Correct parent/sub-issue split

**Parent Issue (COE-254): Linear GraphQL issue and project operations**

```markdown
## Summary
Implement the checked-in Linear GraphQL workflow for issue and project operations.

## Acceptance Criteria
- [ ] All sub-issues are completed and merged:
  - [COE-263](...) - Create issue tool
  - [COE-264](...) - Update issue tool
  - [COE-267](...) - Get project tool
  - [COE-268](...) - Save project tool
  - [COE-270](...) - List issues tool
- [ ] Integration tests verify all tools work together
- [ ] Documentation is updated with tool reference

## Scope
### In scope
- Coordination of sub-issue implementation
- Integration testing after sub-issues merge
- Documentation updates

### Out of scope
- Individual tool implementation (in sub-issues)
- Unit tests for specific tools (in sub-issues)
```

**Sub-Issue (COE-268): Implement project overview update workflow**

```markdown
## Summary
Implement the `project_update_content.graphql` workflow for updating Linear project descriptions.

## Acceptance Criteria
- [ ] Tool accepts project ID and new description
- [ ] Tool validates description length limits
- [ ] Tool returns success/failure with error details
- [ ] Unit tests cover validation logic
- [ ] PR is merged to main

## Scope
### In scope
- Query assets and helper usage in `.agents/skills/linear/`
- Input validation
- Error handling
- Unit tests

### Out of scope
- Other Linear tools (in sibling sub-issues)
- Integration testing (in parent issue)
- Documentation (in parent issue)

## Context
- Parent issue: [COE-254](...)
- Related sub-issues: COE-267 (get project), COE-263 (create issue)
```

## Recommended issue body structure

Use a predictable structure in each Linear issue body:

```markdown
## Summary

## Scope
### In scope
### Out of scope

## Deliverables

## Acceptance criteria

## Test plan
### Unit
### Integration
### Manual

## Context
- repo areas
- docs to read
- linked parent issue if relevant
- blocker issues if relevant

## Definition of Ready
```

Keep the body concise but complete. Preserve enough implementation context that a subagent can work without hidden chat history.

## Parent issue creation procedure

### Step 1: parse the plan

Read the markdown plan and extract:

- parent issue titles
- sub-issues
- local planning IDs
- blockers
- labels
- repo areas
- docs to read
- acceptance criteria
- test plan

### Step 2: create parent issues first

For each parent issue:

- create one Linear issue at the correct project/team
- use the parent issue title exactly unless the repo owner requested a naming tweak
- add the summary, goal, labels, docs, and repo areas
- set priority to medium unless explicitly specified otherwise (see "Phasing vs prioritizing")
- capture the created Linear issue ID and URL in a mapping table

Suggested mapping table fields:

- `local_parent_title`
- `linear_issue_id`
- `linear_url`
- `priority`
- `labels`

### Step 3: confirm parent issue uniqueness

Before creating sub-issues, verify:

- no duplicate parent issue titles exist in the target project where ambiguity would matter
- every planned parent issue has a created Linear issue ID
- no parent issue points to itself in links or descriptions

## Sub-issue creation procedure

### Step 4: create sub-issues under the correct parent issue

For each planned sub-issue:

- create the issue under the mapped parent issue
- use the sub-issue title from the plan
- copy the structured sections from the markdown task
- include Definition of Ready context directly in the issue body
- include repo file paths and docs paths needed for execution
- set priority to medium unless explicitly specified otherwise (see "Phasing vs prioritizing")
- capture the created Linear issue ID and URL in the mapping table

Extend the mapping table with:

- `local_task_id`
- `title`
- `parent_issue_title`
- `linear_issue_id`
- `linear_url`
- `priority`
- `labels`

## Dependency procedure

### Step 5: build a dependency graph before issue creation

Before creating blocker relationships, construct a dependency graph from the plan.

For each planned sub-issue, record:

- local planning ID
- title
- parent issue
- list of blocker local IDs
- whether the issue can be created without referencing any not-yet-created issue

Then do the following:

- compute a topological ordering or dependency levels from the blocker graph
- identify all dependency-free issues with in-degree zero
- identify each subsequent wave of issues whose blockers are entirely in earlier waves
- detect cycles before creating anything

If there is a cycle, stop and resolve the plan inconsistency instead of creating partial data.

### Step 6: use wave-based creation, not naive batching

Creation must happen in waves.

Safe pattern:

1. create all parent issues
2. verify the parent issue mapping table is complete
3. create all dependency-free sub-issues that do not require unresolved issue references
4. verify those issues exist and store their real Linear IDs
5. create the next wave whose blockers now exist
6. repeat until all issues are created
7. apply blocker metadata only after all referenced issue IDs for that wave are confirmed

Unsafe pattern:

- creating issue B in the same concurrent block as issue A when B is blocked by A
- assuming that the first call in a batch will complete before later calls
- using predicted future issue IDs in blocker fields
- creating issues with blocker metadata that references issues not yet visible in Linear

### Step 7: distinguish between creation concurrency and dependency sequencing

Concurrency is allowed only within a wave of mutually independent issues.

Allowed:

- creating several parent issues concurrently
- creating several sub-issues concurrently when none of them block each other and all referenced parent issue IDs already exist
- applying blockers concurrently only when every referenced blocker issue already exists and has been verified

Not allowed:

- creating a chain like A -> B -> C in one parallel block
- creating blocker metadata for B before A has been created and confirmed
- mixing issue creation and blocker assignment in one speculative batch when real issue IDs are still propagating

### Step 8: use read-after-write verification between waves

After each creation wave:

- verify each created issue exists in Linear
- record the actual issue ID and canonical URL returned by Linear
- confirm the issue is queryable before using it as a blocker in later waves
- only then proceed to the next wave

If the workspace or API appears eventually consistent, insert an explicit verification step rather than assuming immediate availability.

### Step 9: resolve blockers using the mapping table

For each sub-issue blocker list:

- look up the blocker's created Linear issue ID from the mapping table
- add the blocker relationship using Linear's blocker metadata fields
- do not rely on body text alone

This means the issue graph in Linear must be queryable through Linear's dependency model, not only readable to humans.

## Linear GraphQL API for Blocker Relationships

Linear's REST API does not expose blocker relationships directly. Use the GraphQL API with `issueRelationCreate` mutation.

In OpenSymphony-managed repos, prefer the repo-local helper with
`.agents/skills/linear/queries/issue_relation_create.graphql`; the raw
GraphQL and `curl` examples below are the fallback shape when you need to
inspect or debug the underlying request directly.

### Authentication

Read the Linear API key from:
- Environment variable: `LINEAR_API_KEY`
- Or file: `~/.config/opensymphony/secrets/linear-api-key.txt`

```bash
LINEAR_API_KEY=$(cat ~/.config/opensymphony/secrets/linear-api-key.txt 2>/dev/null || echo "$LINEAR_API_KEY")
```

### Create blocker relationship

The `issueRelationCreate` mutation creates a "blocks" relationship:

```graphql
mutation CreateRelation($input: IssueRelationCreateInput!) {
  issueRelationCreate(input: $input) {
    success
    issueRelation {
      id
      type
      issue { identifier }
      relatedIssue { identifier }
    }
  }
}
```

Variables:
```json
{
  "input": {
    "issueId": "<blocker-issue-uuid>",
    "type": "blocks",
    "relatedIssueId": "<blocked-issue-uuid>"
  }
}
```

### Important: Direction matters

The `blocks` type means: `issueId` **blocks** `relatedIssueId`.

- **Correct**: If BENCH-002 is blocked by BENCH-001, then:
  - `issueId` = BENCH-001's UUID (the blocker)
  - `relatedIssueId` = BENCH-002's UUID (the blocked issue)
  
- **Wrong**: Setting `issueId` = BENCH-002 would mean BENCH-002 blocks BENCH-001

### curl example

```bash
curl -s https://api.linear.app/graphql \
  -H "Authorization: $LINEAR_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"query": "mutation CreateRelation($input: IssueRelationCreateInput!) { issueRelationCreate(input: $input) { success } }", "variables": {"input": {"issueId": "9dd8e4d2-6c14-4438-9bdb-8f090293d948", "type": "blocks", "relatedIssueId": "af6927c8-3874-4f74-9fe5-55375fd019a3"}}}'
```

### Query existing blockers

To verify blocker relationships after creation:

```graphql
query {
  team(id: "<team-uuid>") {
    issues(first: 100) {
      nodes {
        identifier
        title
        relations {
          nodes {
            type
            relatedIssue { identifier }
          }
        }
      }
    }
  }
}
```

Filter for `type: "blocks"` to see blocker relationships.

### Batch applying blockers

For efficiency, apply multiple blockers in sequence:

```bash
# Apply blockers for an issue with multiple dependencies
for blocker in "uuid-1" "uuid-2" "uuid-3"; do
  curl -s https://api.linear.app/graphql \
    -H "Authorization: $LINEAR_API_KEY" \
    -H "Content-Type: application/json" \
    -d "{\"query\": \"mutation CreateRelation(\$input: IssueRelationCreateInput!) { issueRelationCreate(input: \$input) { success } }\", \"variables\": {\"input\": {\"issueId\": \"$blocker\", \"type\": \"blocks\", \"relatedIssueId\": \"<blocked-issue-uuid>\"}}}"
done
```

### Step 10: rewrite blocker text in issue bodies

After blocker metadata is set, inspect each issue body and replace any document-only blocker references.

Bad:

- `Depends on BENCH-014`
- `Related Issues: TASK-3, TASK-4`

Good:

- `Blocked by COE-172`
- `Related issues: COE-172, COE-173`
- direct canonical Linear links where the team prefers URLs

If the body includes a list of related issues, update the text after creation so it matches the real created issue IDs.

## Reference-rewrite procedure

### Step 11: remove stale document-only IDs from created issue bodies

Search every created issue body for local planning IDs such as:

- `BENCH-`
- `TASK-`
- `EPIC-`
- any other plan-local identifier pattern

Replace them with one of:

- actual Linear issue IDs such as `COE-172`
- canonical Linear URLs
- plain issue titles when hyperlinks are not necessary

The only acceptable case for leaving a document-only ID in a body is when the issue explicitly contains a migration note or a preserved source-plan appendix. That should be rare and deliberate.

### Step 12: rewrite taxonomy language

If the source plan says `Epic`, rewrite it to `parent issue`.
If the source plan says `child task`, rewrite it to `sub-issue` if that matches team practice.

Do not leave mixed taxonomy in the created Linear issues.

## Definition of Ready rules

Each created Linear issue should be independently actionable.

Include or link:

- docs paths to read first
- repo paths likely to be changed
- external references only if actually needed
- blockers via metadata
- acceptance criteria
- test plan
- operational assumptions that are essential for implementation

Do not assume the implementing agent has the original markdown plan open.

## Recommended execution strategy

### Phase A: parse and normalize

- parse the markdown plan into structured parent issues and sub-issues
- normalize taxonomy to parent issue and sub-issue
- extract local planning IDs and blocker references
- build the dependency graph
- detect cycles or missing blocker targets

### Phase B: create parent issues

- create parent issues first
- this can be concurrent if they are independent
- store returned Linear IDs and canonical URLs immediately
- verify each parent issue exists before proceeding

### Phase C: create sub-issues by wave

For each dependency wave:

- create only the sub-issues in that wave
- store returned Linear IDs and canonical URLs immediately
- verify all created issues in the wave exist and are queryable
- only after verification, proceed to the next wave

### Phase D: apply blocker metadata

After all issues referenced by a given blocker set exist:

- apply blocker relationships using Linear metadata
- verify blocker metadata was attached to the correct real issue IDs
- never apply blocker metadata against guessed or document-only IDs

### Phase E: rewrite and validate

- rewrite issue bodies to replace document-only IDs with real Linear IDs or canonical URLs
- validate terminology, links, blocker metadata, and issue hierarchy
- run a final search for stale local IDs

## Validation checklist

Run this checklist before finishing:

### Structural validation

- every parent issue exists in Linear
- every sub-issue exists under the correct parent issue
- every planned blocker is represented in Linear blocker metadata
- no issue is blocked by itself
- no duplicate sub-issues were created

### Content validation

- no stale local planning IDs remain in bodies unless explicitly intentional
- no `Epic` wording remains when the target workflow uses parent issues and sub-issues
- docs links and repo paths are readable and correct
- acceptance criteria and test plans are present

### URL validation

- every Linear link uses the canonical issue URL format
- no malformed URLs include both parent and sub-issue IDs in one path
- no self-links point to the wrong issue

### Dependency validation

- blockers point to the actual created Linear issue IDs
- dependency text and dependency metadata agree
- related-issue text does not mention document-only IDs
- no blocker was attached before the blocker issue existed and was verified
- no dependent issues were created in the same unresolved batch as their blockers

## Common failure modes and how to prevent them

### Failure: blocker text still references document IDs

Prevention:

- always do the post-creation rewrite pass
- search all created bodies for the local ID pattern before finishing

### Failure: dependencies only exist as prose

Prevention:

- use Linear blocker metadata fields after both issues exist
- verify blockers in the issue graph, not just in the description

### Failure: malformed Linear links

Prevention:

- build links from the created issue's canonical URL
- never handcraft URLs by combining multiple issue identifiers

### Failure: parent issue terminology leaks into the wrong taxonomy

Prevention:

- normalize issue language during the rewrite pass
- use a consistent title and body format across all created issues

### Failure: created issues are not Definition of Ready

Prevention:

- copy scope, deliverables, acceptance criteria, test plan, repo areas, and docs into the issue body
- add the minimal missing context during conversion instead of leaving the issue underspecified

### Failure: issue creation fails with `Entity not found: Issue` during batched creation

Cause:

- a dependent issue was created in the same concurrent block as its blocker
- the blocker ID was referenced before the blocker issue had fully propagated or been verified

Prevention:

- never create dependent issues in the same batch as their blockers
- create issues by dependency wave
- verify each earlier wave before creating later waves
- treat blocker assignment as a separate phase unless all referenced issue IDs already exist and are confirmed

### Failure: partial project creation leaves the converter unsure what exists

Prevention:

- after any failed creation wave, query the project or recent issues to reconcile actual created state
- rebuild the mapping table from real Linear data before retrying
- resume from the first incomplete wave rather than restarting blindly

### Failure: issue bodies contain guessed future issue IDs

Prevention:

- never write placeholder Linear IDs into issue bodies
- write neutral text first if needed
- do a rewrite pass only after the real IDs and URLs are known

## Recovery procedure after partial failure

If a creation wave fails partway through:

1. stop issuing more create calls
2. query Linear to determine which issues were actually created
3. update the mapping table using only confirmed real issues
4. identify the first incomplete dependency wave
5. resume creation from that point
6. only after all issues exist, apply blockers and rewrite bodies

Do not assume that failed calls mean nothing was created. Reconcile against real Linear state first.

## Linear Milestones

Linear uses **milestones** to represent different stages in a project's lifecycle. Unlike parent/sub-issues which capture hierarchy and encapsulation, milestones capture sequential phases of development.

### Data model distinction

- **Parent issues and sub-issues** represent hierarchical decomposition (what contains what)
- **Milestones** represent sequential project phases (what happens when)
- An issue can belong to both a parent issue hierarchy AND a milestone simultaneously

### Handling milestone metadata in task conversion

When processing a task plan that includes milestone-related metadata:

1. **Preserve milestone assignments**: If the source plan assigns tasks to specific phases or milestones, maintain that association when creating issues in Linear
2. **Create milestones as needed**: If the target Linear project does not yet have milestones corresponding to the phases in the plan, create them before or alongside issue creation
3. **Assign issues to milestones**: When creating parent issues and sub-issues, associate them with the appropriate milestone based on their phase in the plan
4. **Track milestone coverage**: Ensure all issues from a given phase are assigned to the corresponding milestone

### Milestone vs parent issue semantics

| Concept | Linear construct | Purpose |
|---------|----------------|---------|
| Hierarchical containment | Parent issue / sub-issue | Decomposition of work |
| Sequential phase | Milestone | Time-based grouping |
| Cross-cutting concern | Label | Categorization across hierarchy |

A parent issue can have sub-issues that belong to different milestones. This is valid when a deliverable spans multiple project phases.

## Phasing vs prioritizing

Milestones and priority are independent axes in Linear. Use them for their distinct purposes:

- **Milestones encode the temporal aspect**: Which phase of the project an issue belongs to
- **Priority encodes urgency/importance**: How critical an issue is within its phase, or critical importance to the overall project.

### Default priority policy

- **Default all issues to medium priority** unless explicitly specified otherwise
- Do not assign highest priority to initial tasks simply because they come first
- Priority should reflect actual *importance*, not sequence

### When to adjust priority

Adjust priority up or down from the default (medium) only when:

- **Pre-specified**: The source plan explicitly marks certain tasks as high/urgent or low priority
- **Dynamically determined**: During conversion, identify issues that are genuinely critical path or have external deadlines, or that are optional.

### Phasing through milestones

Encode sequential dependencies through milestones rather than priority:

- Phase 1 (Foundation): All issues in the first milestone
- Phase 2 (Core): All issues in the second milestone
- Phase 3 (Integration): All issues in the third milestone

Within each phase/milestone, issues have independent priority levels. A Phase 2 issue can be high priority while a Phase 1 issue is medium or low priority, even though Phase 1 must complete first.

### Priority and milestone interaction

| Scenario | Milestone | Priority |
|----------|-----------|----------|
| Foundation work, normal urgency | Phase 1: Foundation | Medium (default) |
| Foundation work, critical path | Phase 1: Foundation | High |
| Polish work, can defer | Phase 3: Polish | Low |
| Integration blocker | Phase 2: Core | Urgent |

This separation allows the project to show:
- **What comes first**: Milestone sequence
- **What matters most**: Priority within each phase

## Final deliverable after conversion

At the end of the conversion, produce a compact report containing:

- created parent issues with IDs and URLs
- created sub-issues with IDs, parent issues, and URLs
- created milestones with IDs (if applicable)
- milestone assignments for each issue (if applicable)
- blocker mappings that were applied
- any issues that required manual judgment during conversion
- confirmation that the final validation checklist passed

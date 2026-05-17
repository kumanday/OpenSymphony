# OpenSymphony Memory Capture and Documentation Sync

## Reader and outcome

This document is for a fresh implementer who needs to add OpenSymphony's first
memory capture and documentation sync workflow to the current orchestrator and
CLI, without relying on hidden planning context.

After reading it, the implementer should be able to build the initial local
workflow that:

1. Captures completed Linear issue and GitHub PR history into an issue-centered
   project memory.
2. Indexes that memory for humans and agents.
3. Lets users manually archive Linear issues only after memory capture.
4. Syncs selected private or public memory into component-oriented repository
   documentation.
5. Installs a repo-local agent skill that teaches OpenSymphony agents and other
   harness agents how to consult the memory before doing work.

The scope here is the current local orchestrator and CLI. Desktop UI, hosted
mode, and embedding-based retrieval are future layers that should build on the
same concepts.

## Problem

OpenSymphony currently uses Linear as the near-term planning and execution
surface. That works well while work is active, but it creates a retention
problem:

- Free Linear workspaces have issue limits.
- Archiving completed issues can remove or bury valuable implementation context.
- GitHub PRs preserve code review history, but they are not a convenient
  project memory.
- Agent workpads and comments contain useful audit details, but they are scoped
  to the issue and are hard to query after the issue leaves the active queue.

The product value at risk is not "all logs forever." The value is the condensed
record of what mattered:

- what the task was trying to accomplish
- what actually changed
- what decisions were made
- which review feedback mattered
- what validation evidence was produced
- which follow-ups or risks remain
- which existing docs should now say something different

Linear should remain the short-term planning and coordination system.
OpenSymphony memory should become the longer-lived knowledge layer produced from
completed work.

## Product principle

OpenSymphony memory is the post-merge distillation of what mattered.

The system should not require every issue agent to edit long-lived memory during
implementation. Implementation agents should focus on code, validation, PRs, and
workpad accuracy. After a PR is merged, a separate capture workflow should run
against main and reconcile Linear, GitHub, task markdown, and OpenSymphony
context into durable memory.

The memory system should support two different outputs:

- Issue-centric memory captures what happened for one completed issue.
- Component-oriented docs describe what is now true about the codebase.

Those are different artifacts. The first is issue-level source refs and
distillation. The second is documentation.

## Goals

- Preserve important completed-issue knowledge before Linear archival.
- Avoid making feature branches responsible for memory merges.
- Support run-loop auto-capture for completed work, plus manual capture and
  archive commands for backfill and recovery.
- Keep private memory private by default.
- Allow users who build in public to publish memory if they choose.
- Support public repository documentation generated or updated from private or
  public memory.
- Provide agent discoverability through a real `.agents/skills` skill, not just
  ad hoc CLI commands.
- Use a queryable embedded index for structured search and source-ref queries.
- Keep Obsidian and similar markdown tools optional.
- Avoid storing full agent transcripts or duplicated run logs.

## Non-goals

- No automatic Linear archival by default. Auto-archive is an explicit
  configuration opt-in after successful capture.
- No requirement to commit private memory into the source repository.
- No full run transcript storage.
- No new top-level `decisions` or `components` memory hierarchy as the canonical
  source of truth.
- No desktop app implementation in the first cut.
- No hosted memory backend in the first cut.
- No embedding or late-interaction retrieval in the first cut, though the design
  should leave room for it.

## Core concepts

### Issue capsule

An issue capsule is the primary memory artifact.

It is a markdown page centered on one completed Linear issue. It contains the
distilled record of intent, outcome, decisions, actions, validation, review,
follow-ups, and source refs.

The issue capsule is not a play-by-play transcript. It is a compact closeout
document with enough links and source references to reconstruct the full record
when needed.

### Source evidence

Source evidence is the material used to generate or refresh an issue capsule.
The first implementation should use:

- Linear issue title, description, state, labels, milestone, relations, links,
  and comments.
- The persistent Linear workpad comment when present.
- GitHub PR title, description, branch, commits, merge SHA, review comments,
  check summaries, and changed file list.
- Existing task markdown or planning artifacts when the issue was generated from
  repository task files.
- Minimal OpenSymphony issue context only when needed for source discovery.

OpenSymphony runtime conversations should not be copied into memory. The
existing `opensymphony debug ISSUE-ID` command is the drill-down path when a
human or agent needs the original agent conversation.

### Memory index

The memory index is an embedded local query layer over issue capsules and source
metadata.

DuckDB is a good first embedded database because it supports local analytical
queries without a server, can ingest structured data easily, and can power both
CLI and future UI views.

The DuckDB file should be treated as a local index and query store. It may store
full source snapshots in private-memory mode, but public-memory mode should be
able to use a sanitized or rebuildable index.

### Documentation sync

Documentation sync transforms issue-centric memory into component-oriented
repository documentation.

Humans and agents looking for "authentication," "workspace lifecycle," or
"OpenHands runtime recovery" should not need to know which Linear issues
introduced the relevant knowledge. They should be able to scan ordinary project
docs and find the current model, invariants, gotchas, and recent changes.

Documentation sync should therefore update topic docs, not just export issue
capsules.

### Agent memory skill

The agent UX is a repo-local skill installed under `.agents/skills`, for example
`opensymphony-memory`.

A skill is not a CLI command. It is persistent agent guidance. The skill should
tell agents when and how to consult project memory, which CLI commands to run,
how to interpret the returned context, and which memory mutations are forbidden
during ordinary implementation work.

The CLI provides the retrieval and capture tools. The skill provides agent
behavior.

## Storage modes and visibility

OpenSymphony should support different user privacy postures. The default should
be private memory and optionally public docs.

### Mode A: private memory, public docs

This is likely the safest default for open-source or build-in-public users.

Memory lives outside public source control, for example in local OpenSymphony
state or a private memory repository. It may include Linear comments, review
history, internal notes, and source snapshots.

Repository docs are public and versioned with the codebase when the user chooses
to commit them.

Implications:

- Private issue capsules may link to public docs.
- Public docs should not contain direct links to private issue capsule paths.
- Public docs may include public source refs such as issue identifiers, PR
  numbers, commit SHAs, and public URLs.
- A private DuckDB index can preserve the full mapping from public docs back to
  private issue capsules.
- A docs sync lint should block accidental private path links in public docs.
- Obsidian can be pointed at the private memory vault; public docs can be
  mirrored or linked into that vault for private graph navigation.

The link direction should be mostly one-way:

```text
private issue capsule -> public topic doc
public topic doc -> public PR/commit/issue IDs, not private capsule paths
```

OpenSymphony can still resolve the private relationship in its own CLI or future
UI because the DuckDB index knows the mapping.

### Mode B: public memory, public docs

This mode is for users who intentionally want the audit memory itself to be part
of the public project.

Memory and docs may both live in the repository or in a public companion
repository.

Implications:

- Issue capsules and topic docs can use reciprocal markdown links.
- Obsidian can treat the repository or documentation subtree as one vault.
- Public docs can include direct links to issue capsules.
- The DuckDB index should still be ignored or rebuilt unless the user explicitly
  wants to publish a generated query artifact.
- Redaction and review still matter because public memory may include comments
  or implementation details that were not originally intended as docs.

The link direction can be reciprocal:

```text
public issue capsule <-> public topic doc
```

### Optional modes

The system should not hard-code only the two modes above. Some teams may want:

- private memory and private docs
- public memory and private internal docs
- hosted memory with selective public docs export

The first implementation should model visibility explicitly so these modes can
be added without changing the capture format.

## Suggested local layout

The exact paths should be configurable, but the first implementation needs a
concrete convention.

Private default:

```text
.opensymphony/
  memory/
    issues/
      COE-123.md
    indexes/
      log.md
      index.md
    memory.duckdb
    source-cache/
      private snapshots when enabled
```

Public memory option:

```text
docs/
  memory/
    issues/
      COE-123.md
    index.md
    log.md
```

Public docs sync target:

```text
docs/
  authentication.md
  openhands-runtime.md
  workspace-lifecycle.md
  linear-integration.md
```

These names are examples. The implementation should let users configure the
memory root and documentation targets.

## Issue capsule schema

Each issue capsule should use frontmatter for indexing and markdown for human
reading.

Example:

```markdown
---
type: issue-capsule
visibility: private
issue: COE-123
title: WebSocket reconnect recovery
state: Done
milestone: M3
linear_url: https://linear.app/example/issue/COE-123
prs:
  - number: 456
    url: https://github.com/example/repo/pull/456
    merge_sha: abc123
areas:
  - openhands-runtime
  - reconnect-recovery
source_refs:
  linear_issue: linear:COE-123
  github_prs:
    - github:pr:456
captured_at: 2026-05-13T00:00:00Z
docs_sync:
  status: pending
---

# COE-123: WebSocket reconnect recovery

## Original intent

What the issue asked for and why it mattered.

## Outcome

What shipped after review and merge.

## Decisions and actions

- Decision or action with enough context to be useful later.
- Another decision or action.

## Validation evidence

- Commands, checks, or manual verification that mattered.

## Review and rework

- Important review feedback and how it was resolved.

## Follow-ups and risks

- Follow-up work, deferred risks, or intentionally out-of-scope findings.

## Documentation impact

- Topic docs that should be created or updated.

## Source refs

- Linear: link
- PR: link
- Merge SHA: immutable audit pointer to merged code state
- Debug: `opensymphony debug COE-123`
```

The debug command is sufficient for drilling into the agent conversation. The
capsule should not duplicate the run transcript.

## DuckDB index

DuckDB should support structured queries and future UI views. It should not be
the only readable copy of the memory.

Suggested logical tables:

- `issues`: issue key, title, state, milestone, labels, completion time, archive
  status, capsule path, visibility.
- `issue_sources`: source references, source type, URL, captured timestamp,
  source hash, optional raw snapshot location.
- `pull_requests`: PR number, title, URL, branch, merge SHA, merged timestamp,
  associated issue key.
- `commits`: commit SHA, PR number, author, timestamp, summary.
- `changed_files`: issue key, PR number, file path, change kind when available.
- `checks`: PR number, check name, conclusion, completed timestamp.
- `reviews`: PR number, reviewer, state, timestamp, summarized disposition.
- `areas`: area slug, display name, optional docs target.
- `issue_areas`: issue key to area slug mapping.
- `capsules`: issue key, capsule path, generated timestamp, source hashes,
  capture confidence, warnings.
- `doc_targets`: topic doc identity, path, visibility, managed sections.
- `doc_sync_runs`: sync run id, selected issues, target docs, generated
  timestamp, status.
- `doc_memory_links`: topic doc to issue capsule relationships, with visibility
  handling.

The implementation can start smaller, but it should keep the distinction between
source facts, issue capsules, areas, and docs sync results.

## Manual capture workflow

Capture is manual and writes by default unless `--dry-run` is supplied.

Example command shape:

```bash
opensymphony memory capture COE-123
opensymphony memory capture --issues COE-123,COE-124
opensymphony memory capture --issues-file completed-issues.csv
opensymphony memory capture --issue-range COE-100..COE-199
opensymphony memory capture --before-issue COE-300
opensymphony memory capture --milestone "M9: Collaborative Planning Alpha"
opensymphony memory capture --milestones-file milestones.csv
opensymphony memory capture --state Done --before-date 2026-05-01
```

Dry run should show:

- selected issues
- Linear sources found
- GitHub PRs found
- task files or planning artifacts found
- proposed capsule paths
- proposed areas
- proposed docs impact
- warnings and missing sources
- whether each issue already has a capsule
- whether the capsule appears stale relative to source evidence

Write mode should:

- generate or update issue capsules
- update the DuckDB index
- update private `index.md` and `log.md` if markdown indexes are enabled
- report unresolved warnings

Capture should not archive Linear issues.

## Manual archive workflow

Archival is a separate explicit action.

Example command shape:

```bash
opensymphony linear archive --issues COE-123,COE-124
opensymphony linear archive --issues-file completed-issues.csv
opensymphony linear archive --milestone "M9: Collaborative Planning Alpha"
opensymphony linear archive --captured-before 2026-05-01
opensymphony linear archive --from-memory --state captured
```

Archive should default to requiring a fresh issue capsule before it archives an
issue. A `--force` escape hatch may exist, but it should be noisy.

The archive dry run should show:

- which issues are eligible
- which issues are blocked because memory is missing or stale
- which issues have unresolved capture warnings
- which Linear operation would be performed

No auto-archive should happen as part of PR merge, docs sync, or capture unless
the user explicitly requests it.

## Documentation sync workflow

Documentation sync transforms captured issue memory into topic docs.

Example command shape:

```bash
opensymphony memory init
opensymphony memory sync-docs --since-last-sync
opensymphony memory sync-docs --issues COE-123,COE-124
opensymphony memory sync-docs --milestone "M9: Collaborative Planning Alpha"
opensymphony memory sync-docs --area authentication
opensymphony memory sync-docs --issues-file completed-issues.csv
```

Dry run should show:

- selected issue capsules
- proposed target docs
- docs that would be created
- docs that would be updated
- diagrams that would be added or refreshed
- privacy warnings
- a summary of the proposed documentation diff

Write mode should produce a reviewable working tree diff or patch. It should not
silently publish or commit docs.

Topic docs should answer "what is now true?" rather than "what happened in issue
X?"

Suggested topic doc structure:

```markdown
---
type: topic-doc
area: authentication
visibility: public
last_memory_sync: 2026-05-13T00:00:00Z
---

# Authentication

## Current model

The current behavior and architecture.

## Important invariants

Things future agents must preserve.

## Operational flow

Mermaid diagrams or concise flow descriptions when useful.

## Known gotchas

Failure modes, integration quirks, and constraints.

## Recent changes

Short bullets derived from issue capsules.

## Source refs

Public PRs, commits, and issue identifiers. Private capsule links only when the
docs visibility allows them.
```

Docs sync should use topic docs as the component-centric view. It should not
make users read issue capsules to understand a subsystem.

## Mermaid diagrams

Mermaid diagrams are most useful in documentation sync, not in every issue
capsule.

Good diagram targets:

- sequence diagrams for authentication, runtime startup, reconnect, and publish
  flows
- state diagrams for Linear issue lifecycle and run lifecycle
- flowcharts for scheduler, reconciliation, capture, and docs sync behavior
- dependency graphs for milestone summaries

Issue capsules may include diagrams only when a particular issue introduced a
new flow or changed a state machine. Topic docs and milestone summaries are the
better default place for diagrams because they accumulate and simplify
cross-issue knowledge.

Diagram generation should be optional and reviewable:

```bash
opensymphony memory sync-docs --area openhands-runtime --with-diagrams
```

Docs sync should avoid diagrams that expose private issue-capsule paths in
public docs.

## Obsidian and graph navigation

Obsidian is optional. OpenSymphony should not require users to install it.

However, the markdown memory and docs structure should work well as an Obsidian
vault for users who want graph navigation.

The graph is useful only if generated markdown contains links. For private
memory, issue capsules can link to:

- related issue capsules
- milestone index pages
- public topic docs
- public PRs and commits

For public memory and public docs, topic docs can link back to issue capsules.

For private memory and public docs, public docs should avoid direct links to
private pages. The private vault can still show the complete graph because
private issue capsules can link outward to public docs, and OpenSymphony can
maintain a private link index in DuckDB.

Possible vault modes:

- Memory-only vault: point Obsidian at the private memory root.
- Public repo vault: point Obsidian at the repository or public docs tree when
  memory is public.
- Composite private vault: OpenSymphony mirrors or links public docs into the
  private memory root for local graph navigation.

Symlinks should be optional because they can behave differently across
platforms. A generated mirror of selected public docs is safer for the first
implementation.

## Agent UX: repo-local memory skill

The memory feature should add a template-managed agent skill that can be
installed into target repositories by `opensymphony init` or a dedicated setup
command when the template repo provides it. The OpenSymphony binary should not
carry or inject its own copy of the skill.

The skill should be named clearly, for example `opensymphony-memory`.

The skill should instruct agents to consult memory at specific moments:

1. At kickoff for any nontrivial issue.
2. After reading the issue body and before making a plan.
3. After discovering the likely files or areas to touch.
4. During rework when review feedback resembles prior issues.
5. Before updating docs.

The skill should also tell agents what not to do:

- Do not create or update issue capsules during ordinary implementation unless
  the user explicitly asks.
- Do not archive Linear issues.
- Do not rewrite public docs from private memory without running the approved
  docs sync workflow.
- Do not treat retrieved memory as authoritative over current code.

The skill can direct agents to run commands such as:

```bash
opensymphony memory context --issue COE-456
opensymphony memory related --issue COE-456
opensymphony memory related --paths crates/opensymphony-openhands
opensymphony memory search "subscription credential refresh"
opensymphony memory docs --area authentication
opensymphony memory brief COE-123
```

The command names can change during implementation, but the agent-facing
capabilities should exist:

- `context`: return a compact implementation brief for a new issue.
- `related`: find prior issue capsules and docs related to an issue, path, area,
  label, or query.
- `search`: search issue capsules, topic docs, and indexed source summaries.
- `docs`: retrieve current component-oriented docs for an area.
- `brief`: show one issue capsule in a compact form.

The output should be optimized for agent use:

- concise markdown by default
- optional JSON for tools
- explicit source refs
- warnings when memory is stale
- separation between "facts from source evidence" and "LLM-generated synthesis"
- suggested docs or tests to inspect

The skill should be generic enough for other harnesses. It should be plain
markdown instructions plus shell commands, not OpenHands-specific behavior.

## Human UX

The first user experience can be CLI-first.

Important CLI affordances:

- explicit `--dry-run` previews before writes
- clear source discovery output
- confidence and warning summaries
- stat-style write output for docs sync
- separate capture and archive actions
- commands that open or print generated capsules
- status commands that show what has been captured, stale, synced, or archived

Useful commands:

```bash
opensymphony memory status
opensymphony memory status --milestone "M9: Collaborative Planning Alpha"
opensymphony memory show COE-123
opensymphony memory open COE-123
opensymphony memory lint
opensymphony memory lint --public-docs
```

Future desktop UI can wrap the same operations:

- capture queue
- issue selection preview
- source evidence preview
- capsule review
- docs diff review
- archive eligibility
- graph/search UI

The CLI should be useful without waiting for that UI.

## Capture source reconciliation

The capture workflow needs deterministic source matching.

Recommended source priority:

1. Explicit issue IDs supplied by the user.
2. Linear issue links to GitHub PR attachments.
3. PR title/body references to issue IDs.
4. Branch names that include issue IDs.
5. Task manifest or publish receipt mappings when available.
6. User-provided overrides.

The dry run should show how each PR was matched to each issue.

If matching is ambiguous, the workflow should ask for explicit input through a
flag or mapping file rather than silently choosing.

Example override file shape:

```yaml
issues:
  COE-123:
    prs: [456]
    areas:
      - openhands-runtime
      - reconnect-recovery
  COE-124:
    prs: [457, 458]
```

## Area and docs mapping

Areas are the bridge between issue memory and topic docs.

Area inference can use:

- Linear title, description, labels, milestone, parent, children, and active
  Workpad comment
- PR title, description, checks, and review discussions
- existing learned areas and aliases in the memory config

Area inference should not inspect code or diffs. GitHub changed files are useful
structured metadata for later path-to-issue lookup, but they should not create
areas or appear in capsule/doc prose. Merge SHA is source-ref metadata, not
inference input.

Example learned area configuration:

```yaml
areas:
  openhands-runtime:
    title: OpenHands Runtime
    docs_target: docs/openhands-runtime.md
    visibility: public
    status: stable
    confidence: 85
    aliases:
      - OpenHands Runtime
      - runtime
    source_refs:
      docs:
        - docs/openhands-runtime.md
      linear_labels:
        - runtime
      linear_issues:
        - COE-123
```

`opensymphony memory init` seeds stable areas from existing top-level
`docs/*.md` files only. Capture then evolves this file from Linear and GitHub
narrative evidence.

## Linting and health checks

Memory lint should check:

- issue capsules with missing source refs
- stale capsules where source evidence changed
- public docs that link to private memory paths
- docs sync runs that failed or were never reviewed
- issues marked archive-eligible without fresh capture
- issue capsules with no learned area
- stable areas with no docs target
- unresolved follow-ups that were never converted into issues or explicitly
  dismissed

The lint output should be actionable. It should tell the user which command to
run next.

## Privacy and redaction

Memory capture will often see sensitive material. The default posture should be
private.

Required behaviors:

- The default memory root is not committed to the source repository.
- Public memory mode requires explicit configuration.
- Public docs sync must detect and warn about private links.
- Public docs sync should prefer public source refs: PR URLs, commit SHAs, and
  issue identifiers.
- Redaction hooks should exist before writing public memory or public docs.
- Source snapshots should be optional and private by default.
- The DuckDB index should not be committed unless explicitly configured.

The system should distinguish:

- private source evidence
- private issue capsules
- public issue capsules
- public topic docs
- generated local indexes

Visibility should be explicit in frontmatter and index tables.

## Source control posture

Memory does not need to be versioned in the code repository.

Supported postures:

- Private unversioned memory in local OpenSymphony state.
- Private versioned memory in a separate private repository.
- Public versioned memory in the source repository or a public companion
  repository.
- Public versioned docs in the source repository, generated from private or
  public memory.

The docs sync workflow is the clean boundary for source control. It produces
ordinary documentation changes that the user may review, commit, and publish.

Issue memory capture should not force a code repository commit.

## Future embedding layer

Embedding-based retrieval can be added later without changing the memory model.

The future retrieval layer should index:

- issue capsules
- topic docs
- source summaries
- changed file paths
- code chunks
- area tags

Late-interaction or multi-vector retrieval may be especially useful for
codebase-aware questions such as:

- "What prior work touched this subsystem?"
- "Which issues changed authentication semantics?"
- "What should an agent know before editing this file?"
- "Which validation patterns have caught bugs here before?"

The embedding layer should augment DuckDB and markdown. It should not become the
only memory substrate.

## Implementation work to be done

This is not a task manifest. It is the implementation shape a future Codex
conversation can follow.

### 1. Configuration

Add memory configuration for:

- enabled/disabled memory
- memory root
- visibility mode
- DuckDB index path
- source snapshot policy
- docs sync targets
- learned areas
- public/private link policy
- redaction hooks or deny patterns

### 2. Source adapters

Implement source adapters for:

- Linear issue details, comments, links, relations, milestones, and labels
- GitHub PR details, review comments, checks, commits, and changed files
- task markdown and publish receipt mappings when present
- minimal OpenSymphony issue lookup only for source discovery and debug command
  generation

Use existing Linear GraphQL assets and existing local conventions where possible.
GitHub integration can start with `gh` where available, with a cleaner adapter
boundary for future hosted mode.

### 3. Source matching

Build deterministic issue-to-PR matching with dry-run reporting and explicit
override support.

### 4. Capsule generation

Generate or refresh issue capsules from source evidence.

The generator should preserve human edits where possible. If preservation is too
large for the first cut, the tool should clearly mark generated sections and
warn before overwriting edited capsules.

### 5. DuckDB indexing

Create and migrate the DuckDB schema. Index issue capsules, source metadata,
PRs, changed files, checks, areas, and docs sync state.

### 6. Markdown indexes

Generate optional `index.md` and `log.md` files for humans and simple agent
navigation.

### 7. Manual archive command

Implement Linear archive commands with selection filters, dry-run output, and
fresh-capture requirements.

### 8. Documentation sync

Implement docs sync from issue capsules to topic docs.

The first version should:

- select capsules by issue, milestone, area, date, or since-last-sync
- propose target docs
- update current model, invariants, gotchas, recent changes, and source refs
- optionally generate Mermaid diagrams
- produce stat-style write output
- enforce public/private link policy

### 9. Agent skill installation

Add a template-managed `.agents/skills/opensymphony-memory` skill and install
or update it through repository initialization when the target template provides
the file. Do not embed the skill in the OpenSymphony binary.

The skill should guide agents to consult memory at kickoff and after discovering
likely areas, while forbidding ordinary implementation agents from mutating
memory or archiving issues.

### 10. Search and context commands

Implement agent- and human-facing retrieval:

- brief by issue
- search by text
- related by issue
- related by path
- related by area
- context bundle for a new issue
- docs lookup by area

### 11. Lint and status

Implement memory status and lint commands for stale capsules, missing sources,
privacy leaks, docs sync drift, and archive eligibility.

### 12. Tests

Add tests with fake Linear and GitHub source data for:

- issue selection
- issue-to-PR matching
- capsule generation
- DuckDB indexing
- public/private link policy
- docs sync patches
- archive eligibility
- agent skill template rendering

### 13. Documentation

Document the user workflow:

- configure memory
- capture completed issues
- review capsules
- sync docs
- archive Linear issues
- consult memory from agents
- use Obsidian optionally

## Acceptance criteria for the first cut

- A user can run a dry-run capture for one issue and see Linear and GitHub
  sources that would be used.
- A user can write an issue capsule for one completed issue.
- A user can index that capsule in DuckDB.
- A user can query a compact brief for that issue.
- A user can retrieve related memory by issue, path, area, or text query.
- A user can run docs sync in dry-run mode and see a proposed documentation
  diff.
- Public docs sync does not emit private memory links.
- A user can manually archive only captured issues unless they force override.
- A repo-local agent skill can be installed and tells agents how to consult
  memory.
- No full OpenSymphony run transcript is copied into memory.
- `opensymphony debug ISSUE-ID` remains the drill-down path for original agent
  conversations.

## Open questions

- Should private source snapshots store full comment bodies by default, or only
  hashes plus generated capsules?
- Should capsule generation use fully managed sections, human-editable sections,
  or a mixed model?
- Should docs sync write directly into topic docs or generate patch files that a
  human applies?
- Which GitHub integration should be the default for users without `gh`
  installed?
- How much redaction should happen before the LLM sees source evidence versus
  only before writing public artifacts?
- Should the first Obsidian support create a mirrored composite vault, or simply
  document how to open the private memory root?

These questions do not block the first implementation, but the first
implementation should keep the boundaries explicit enough to change the answers
later.

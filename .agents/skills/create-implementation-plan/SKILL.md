---
name: create-implementation-plan
description: |
  Generate a structured implementation plan from project requirements with
  decomposed tasks, milestones, dependencies, and acceptance criteria.
  Use when starting a new project or planning a new development wave that
  should produce docs/tasks/task-package.yaml and issue-ready task files.
---

# Create Implementation Plan Skill

## Purpose

Generate a structured planning package from project requirements. The package
must include shared project context plus a deterministic task manifest that can
be reviewed by humans, converted to Linear, and executed by implementation
agents without hidden chat history.

## When To Use

Use this skill when a team is ready to turn a product idea, PRD, design note,
research brief, or follow-on development request into implementation tasks.

For iterative projects, create a new planning wave instead of rewriting the
identity of an already-published wave. A planning wave is a named round of
planning and decomposition such as `bootstrap-mvp`, `hosted-alpha`, or
`rich-client-hosted-mode`.

## Required Inputs

Before generating files, gather or infer:

- Project or planning-wave name.
- Project description and success criteria.
- Key requirements and features.
- Technical constraints and preferences.
- Existing PRDs, architecture notes, source files, external links, and research.
- Existing task and Linear context when this is a follow-on planning wave.

## Process

### Step 1: Gather Context

Collect relevant source material and synthesize it before creating tasks:

- Existing design documents and PRDs.
- Stakeholder requirements and success criteria.
- Technical research findings.
- Reference implementations or public API documentation.
- Existing repo conventions, architecture, and task history.

Add targeted supplemental research only where it improves the task plan.

### Step 2: Generate Shared Context And Architecture Documentation

Create or update these files when the project needs them:

**AGENTS.md** - Persistent implementation context for coding agents:

- Project mission and scope.
- Non-negotiable constraints and architectural invariants.
- Cross-cutting definitions and repository conventions.
- Commands, environment expectations, and references to deeper docs.

If `AGENTS.md` already exists, refine it while preserving useful project-specific guidance.

**README.md** - Human-facing project overview:

- Problem statement and goals.
- Setup and primary workflows.
- High-level architecture summary.
- Links to detailed docs and task plans.

**docs/architecture.md** - System architecture:

- Component breakdown.
- Data flow.
- Integration points.
- Technology choices and rationale.

**docs/decisions/** - Architecture Decision Records when decisions need a durable record.

### Step 3: Create The Task Package Manifest

Create `docs/tasks/task-package.yaml`. This file is the canonical
machine-readable input for `convert-tasks-to-linear`.

Use this shape:

```yaml
planningWave: rich-client-hosted-mode
tasksDir: docs/tasks
milestones:
  - "M1: Gateway And Stream Contract"
  - "M2: Shared Client And Desktop Alpha"
tasks:
  - id: TASK-001
    file: docs/tasks/001-current-gateway-inventory.md
  - id: TASK-002
    file: docs/tasks/002-gateway-schemas.md
```

Rules:

- `planningWave` is a stable string identifier for this planning round.
- `tasksDir` is the directory containing task files.
- `milestones` contains exact Linear milestone names.
- `tasks` is the complete list of task files for this wave.
- Task discovery reads the manifest task list.

### Step 4: Generate Implementation Tasks

Create one Markdown file per task. File names may follow a readable convention
such as `001-brief-description.md`, but the manifest is the source of truth.

Each task file must include this frontmatter:

```markdown
---
id: TASK-001
title: Human-readable task title
milestone: "M1: Gateway And Stream Contract"
priority: 3
estimate: 3
blockedBy: []
blocks: []
parent: null
---
```

Field rules:

- `id` must be unique within `task-package.yaml`.
- `milestone` must exactly match one entry in `task-package.yaml`.
- `priority` uses Linear-compatible numeric priority: `1=Urgent`, `2=High`, `3=Normal`, `4=Low`.
- `estimate` is a numeric story-point estimate.
- `blockedBy` and `blocks` contain task IDs from the same manifest.
- `parent` is `null` for top-level issues or a task ID for a Linear sub-issue.

Use this body structure:

```markdown
## Summary

One or two sentences describing what this task accomplishes.

## Scope

### In scope

- Specific item 1
- Specific item 2

### Out of scope

- Explicitly excluded item 1

## Deliverables

- File or artifact 1
- File or artifact 2

## Acceptance Criteria

- [ ] Criterion 1: measurable outcome
- [ ] Criterion 2: measurable outcome

## Test Plan

- Test command or verification step 1
- Test command or verification step 2

## Context

- Relevant repo paths to inspect or modify.
- Docs, specs, or external sources to read first.
- Parent task, blockers, or sibling work that matter.

## Definition of Ready

- [ ] Hidden assumptions from prior discussion are written down.
- [ ] Required files, docs, and dependencies are explicitly referenced.
- [ ] A coding agent could begin execution without additional planning context.

## Notes

Any additional context, references, or gotchas.
```

### Step 5: Generate The Human Milestone Index

Create `docs/tasks/milestones.md` as the human-readable overview. It should use
the same milestone names as `task-package.yaml`.

```markdown
# Project Milestones

## M1: Gateway And Stream Contract

Goal: Establish the versioned gateway and stream contract.

Tasks:

- TASK-001 Current Gateway Inventory
- TASK-002 Gateway Schemas
```

`milestones.md` can include goals and explanatory prose. Conversion relies on
`task-package.yaml`.

### Step 6: Validate Completeness

Before finishing, check that:

- Every manifest task file exists.
- Every task has required frontmatter and body sections.
- Every task ID is unique.
- Every dependency and parent reference points to a manifest task.
- `blockedBy`, `blocks`, and `parent` references point to manifest tasks.
- The dependency graph has no cycles.
- Each task is independently implementable.
- Acceptance criteria and test plans are measurable.

## Expected Output

After completing this skill, the repository should contain:

```text
AGENTS.md
README.md
docs/
├── architecture.md
├── decisions/
└── tasks/
    ├── task-package.yaml
    ├── milestones.md
    ├── 001-bootstrap.md
    ├── 002-setup-testing.md
    └── ...
```

## Next Steps

After generating the package:

1. Ask the user to review `docs/tasks/task-package.yaml`, `docs/tasks/milestones.md`, and the task files.
2. Use `convert-tasks-to-linear` to validate, dry-run, and publish the planning wave.
3. Verify hierarchy, blockers, and project placement in Linear.
4. Begin execution with `opensymphony run`.

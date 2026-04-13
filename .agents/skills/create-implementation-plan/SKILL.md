---
name: create-implementation-plan
description: |
  Generate a structured implementation plan from project requirements with
  decomposed tasks, milestones, dependencies, and acceptance criteria.
  Use when starting a new project to create docs/tasks/*.md files suitable
  for conversion to Linear issues.
---

# Create Implementation Plan Skill

## Purpose

Generate a structured implementation package from project requirements, including persistent project context and implementation tasks that are suitable for conversion to Linear issues and autonomous execution.

## When to Use

Use this skill when starting a new project after:
1. Brainstorming requirements and design with LLM
2. Having a rough PRD or product vision
3. Ready to decompose work into actionable tasks

## Required Inputs

Before generating files, gather or infer:
- Project name and description
- Key requirements and features
- Technical constraints and preferences
- Any existing design documents or PRDs
- User-provided links, attached findings, or earlier planning notes

## Process

### Step 1: Gather Context

Collect all relevant context:
- Existing design documents
- Requirements from stakeholders
- Technical research findings
- Reference implementations or specs
- User-provided links, attachments, and findings

Synthesize the provided source material first, then do targeted supplemental research only where it materially improves the implementation plan.

Capture the cross-cutting context that later coding agents will need without relying on hidden chat history.

### Step 2: Generate Shared Context and Architecture Documentation

Create the following files:

**AGENTS.md** - Persistent implementation context for coding agents:
- Project mission and scope
- Non-negotiable constraints and architectural invariants
- Cross-cutting concerns and definitions
- Key commands, environment expectations, and repo conventions
- References to deeper docs when needed

If `AGENTS.md` already exists, refine it instead of replacing useful project-specific guidance.

**README.md** - Human-facing project overview:
- Problem statement and goals
- Setup and primary workflows
- High-level architecture summary
- Pointers to detailed docs and task plans

If `README.md` already exists, update it so it remains consistent with the generated plan.

**docs/architecture.md** - System architecture:
- High-level architecture diagram (text-based)
- Component breakdown
- Data flow
- Integration points
- Technology choices and rationale

**docs/decisions/** - Architecture Decision Records (ADRs):
- One file per major decision
- Context, Decision, Consequences format

### Step 3: Generate Implementation Tasks

Create tasks in `docs/tasks/` directory with **one file per task**:

**File naming**: `NNN-brief-description.md` (e.g., `001-bootstrap-project.md`)

**Required schema**:

```markdown
---
title: Human-readable task title
milestone: M1  # M1, M2, M3, etc.
priority: 1    # 1=Urgent, 2=High, 3=Normal, 4=Low
estimate: 3    # Story points (optional)
blockedBy: []  # List of task IDs this depends on
blocks: []     # List of task IDs this blocks
parent: null   # Parent task ID for sub-issues
---

## Summary

One or two sentence description of what this task accomplishes.

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
- [ ] Criterion 3: measurable outcome

## Test Plan

- Test command or verification step 1
- Test command or verification step 2

## Context

- Relevant repo paths to inspect or modify
- Docs or specs to read before implementation
- External links, findings, or decisions that constrain the work
- Parent task, blockers, or sibling work that matter

## Definition of Ready

- [ ] Hidden assumptions from prior discussion are written down
- [ ] Required files, docs, and dependencies are explicitly referenced
- [ ] A coding agent could begin execution without additional planning context

## Notes

Any additional context, references, or gotchas.
```

### Step 4: Organize Milestones

Group tasks into logical milestones:

```markdown
## docs/tasks/milestones.md

# Project Milestones

## M1: Foundation

Goal: Establish project skeleton, tooling, and core infrastructure.

Tasks:
- 001-bootstrap-project
- 002-setup-testing
- 003-configure-ci

## M2: Core Features

Goal: Implement primary functionality.

Tasks:
- 010-implement-auth
- 011-implement-data-layer
- 012-implement-api

## M3: Integration

Goal: Connect components and add polish.

Tasks:
- 020-integrate-components
- 021-add-documentation
- 022-performance-optimization
```

### Step 5: Define Dependencies

Ensure the dependency graph is valid:
- No circular dependencies
- Foundation tasks have no dependencies
- Each milestone builds on previous
- Critical path is clear

### Step 6: Validate Completeness

Check that:
- Each task is independently implementable
- Acceptance criteria are testable
- Dependencies are correctly specified
- No duplicate tasks
- Coverage of all requirements
- Shared context files cover the cross-cutting guidance needed across tasks
- Each task includes enough repo and documentation context to meet Definition of Ready for an independent coding agent

## Expected Output

After completing this skill, the repository should contain:

```
AGENTS.md                    # Persistent agent context
README.md                    # Human-facing project overview
docs/
├── architecture.md          # System architecture
├── decisions/               # ADRs
│   ├── 001-technology-choice.md
│   └── 002-data-model.md
└── tasks/
    ├── milestones.md        # Milestone overview
    ├── 001-bootstrap.md
    ├── 002-setup-testing.md
    └── ...                  # One file per task
```

## Next Steps

After generating the implementation package, suggest these next steps to the user:

1. Review the generated tasks for accuracy
2. Use the repo's `convert-tasks-to-linear` skill to create the Linear issues, or create them manually
3. Verify hierarchy, blockers, and project placement in Linear
4. Begin execution with `opensymphony run`

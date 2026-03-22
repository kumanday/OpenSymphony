---
name: inspect-opensymphony-artifacts
description: Inspect local OpenSymphony workspace artifacts without changing repo-owned policy files.
---

# Inspect OpenSymphony Artifacts

Use this skill when you need to confirm what OpenSymphony generated for the current issue workspace.

- Treat `WORKFLOW.md`, `AGENTS.md`, and other checked-in repo files as repo-owned policy.
- Treat `.opensymphony/` as additive local state and audit output.
- Prefer reading:
  - `.opensymphony/issue.json`
  - `.opensymphony/run.json`
  - `.opensymphony/conversation.json`
  - `.opensymphony/generated/issue-context.md`
  - `.opensymphony/generated/session-context.json`
  - `.opensymphony/prompts/`
  - `.opensymphony/runs/`
- Do not edit or commit `.opensymphony/` unless the task explicitly asks for fixture changes.

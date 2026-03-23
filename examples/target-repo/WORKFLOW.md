---
tracker:
  kind: linear
  project_slug: sample-project
  active_states:
    - Todo
    - In Progress
  terminal_states:
    - Done
workspace:
  root: ./var/workspaces

openhands:
  transport:
    base_url: http://127.0.0.1:8000
---

# Example Workflow

Work the assigned issue inside this repository and keep changes small and reviewable.

Repository context precedence for this example target repo:

1. Follow this `WORKFLOW.md`.
2. Respect repo-owned `AGENTS.md` and any repo-owned `.agents/skills/`.
3. Treat `.opensymphony/generated/issue-context.md` and other `.opensymphony/` files as additive context and audit artifacts.
4. Do not rewrite repo-owned policy files to make the run succeed.

---
name: opensymphony-memory
description: Consult OpenSymphony project memory before planning nontrivial work, during review rework, and before documentation updates.
---

# OpenSymphony Memory

Use this skill when working in a repository that has OpenSymphony memory enabled.
Memory is the post-merge record of completed issue work. It is useful context,
but current code, tests, and explicit user instructions remain authoritative.

## When to Consult Memory

- At kickoff for any nontrivial Linear issue, after reading the issue body and before writing a plan.
- After discovering the likely files, directories, labels, or subsystem areas you may touch.
- During review rework when feedback resembles a prior issue or failure mode.
- Before updating docs or changing subsystem behavior that may already have topic documentation.

## Commands

Run the smallest command that answers the question:

```bash
opensymphony memory context --issue COE-456
opensymphony memory related --issue COE-456
opensymphony memory related --paths crates/opensymphony-openhands
opensymphony memory related --area openhands-runtime
opensymphony memory search "subscription credential refresh"
opensymphony memory docs --area openhands-runtime
opensymphony memory brief COE-123
```

Use `opensymphony memory status` when you need to know whether memory exists,
is pending docs sync, or has capture warnings. Use `opensymphony memory lint
--public-docs` before relying on generated public docs in a public repository.

## Rules

- Do not create or update issue capsules during ordinary implementation unless the user explicitly asks; normal capture is run-loop infrastructure when `memory.auto_capture` is enabled.
- Do not archive Linear issues unless the user explicitly asks. Run-loop auto-archive is disabled by default and still requires successful capture.
- Do not rewrite public docs from private memory by hand; use `opensymphony memory sync-docs` so the managed sections and indexes stay consistent.
- Do not copy full agent transcripts into memory or docs.
- Do not treat retrieved memory as authoritative over current source files, tests, or upstream specs.

## Interpretation

Memory output separates source-derived facts from generated synthesis when the
capture has enough evidence. Prefer source refs such as Linear issue IDs, PR
URLs, and merge SHAs when making audit claims. Merge SHA is an immutable pointer
to the merged code state, not an area inference signal. If memory is stale,
missing, or warning-heavy, inspect the linked PRs, current docs, and
`opensymphony debug ISSUE-ID` before depending on it.

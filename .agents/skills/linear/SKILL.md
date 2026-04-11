---
name: linear
description: |
  Interact with Linear through the best available transport for the current
  session: OpenSymphony Linear MCP tools first, optional injected
  `linear_graphql` when available, and raw GraphQL via `LINEAR_API_KEY` for
  GraphQL-only gaps.
---

# Linear

Use this skill whenever the agent needs to read or write Linear state from an
OpenSymphony-managed repository.

## Goal

Choose the narrowest transport that can complete the task while keeping the
workflow reproducible across OpenSymphony sessions, Codex desktop sessions, and
future injected-tool runtimes.

## Transport order

OpenSymphony currently supports three Linear access paths:

1. Linear MCP tools
   - Primary OpenSymphony path for routine issue operations.
   - The current MCP surface is intentionally narrow: issue fetch, comment
     create, state transition, URL/PR link attachment, and workflow-state
     lookup.
   - Prefer this when the operation is covered by the MCP surface.
2. Optional `linear_graphql` dynamic tool
   - Present in Symphony/Codex app-server sessions and any future runtime that
     injects the same tool.
   - Prefer this over raw shell GraphQL when you need schema-level GraphQL
     access and the tool is available in-session.
3. Raw GraphQL via `LINEAR_API_KEY`
   - Fallback when the required operation is not covered by MCP or no injected
     Linear tool is available.
   - Use the reference files below instead of improvising large GraphQL
     documents from scratch.

If none of the three paths is available, report a real Linear blocker.

## How to choose quickly

- For routine issue fetch, comment creation, transitions, PR links, and state
  lookup, start with [references/mcp-capabilities.md](references/mcp-capabilities.md).
- For raw GraphQL transport and auth fallback, open
  [references/raw-graphql.md](references/raw-graphql.md).
- For issue, comment, attachment, and dependency mutations, open
  [references/issue-and-comment-operations.md](references/issue-and-comment-operations.md).
- For project overview/content updates, uploads, and schema discovery, open
  [references/project-and-advanced-operations.md](references/project-and-advanced-operations.md).

## Rules

- Prefer MCP for routine issue operations that it already abstracts cleanly.
- Prefer `linear_graphql` over raw shell GraphQL when the session already
  exposes it and you need a GraphQL-only operation.
- Use raw GraphQL via `LINEAR_API_KEY` for gaps such as project overview
  updates, comment edits, uploads, schema introspection, or issue-relation
  mutations when those are not available through MCP.
- Keep GraphQL operations narrow: one operation per request, minimal fields, and
  variables instead of string interpolation.
- Reuse the repository's existing project slug semantics:
  `tracker.project_slug` stores Linear `Project.slugId`.
- Do not invent new ad hoc GraphQL shapes if a reference file already covers the
  operation.
- If the needed mutation shape is unfamiliar, use the introspection patterns in
  the advanced reference file before guessing.

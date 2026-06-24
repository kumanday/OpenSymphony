# OKF Memory System Specification

Status: draft

Source basis: [Open Knowledge Format v0.1 draft](https://github.com/GoogleCloudPlatform/knowledge-catalog/blob/main/okf/SPEC.md) and the current OpenSymphony project memory model.

Reader: an OpenSymphony engineer evolving the memory subsystem.

Post-read action: implement OKF-compliant memory capture, validation, indexing, and export without weakening OpenSymphony's private memory, source citation, or scheduler boundaries.

## 1. Summary

OpenSymphony memory should evolve from "Markdown files that happen to work in Obsidian" into a first-class Open Knowledge Format bundle producer and consumer.

OKF becomes the portable document contract:

- A knowledge bundle is a directory tree of Markdown files.
- Every non-reserved Markdown file is a concept document with YAML frontmatter.
- Every concept has a non-empty `type`.
- `index.md` and `log.md` are reserved filenames with defined meanings.
- Standard Markdown links express graph relationships.
- Consumers tolerate unknown types, unknown fields, missing optional fields, broken links, and missing indexes.

OpenSymphony keeps its existing operational architecture:

- The memory catalog, DuckDB index, MCP tools, and gateway APIs remain derived service layers.
- Markdown concepts remain the durable document store.
- Private memory remains private by default.
- Scheduler correctness must not depend on memory capture, docs sync, or graph rendering.

## 2. Goals

1. Produce OKF v0.1-conformant bundles from captured project memory.
2. Preserve existing issue capsule value: intent, outcome, decisions, validation, review context, risks, documentation impact, and source refs.
3. Make bundle concepts portable across tools, including agent readers, static sites, Obsidian-like viewers, and hosted OpenSymphony.
4. Keep the catalog/index/query layer fast while making it rebuildable from OKF documents.
5. Support private, public, and exported bundle variants without leaking private source material.
6. Give the graph viewer a stable, standards-shaped corpus to visualize.

## 3. Non-Goals

- Do not replace the memory MCP server, DuckDB catalog, or retrieval APIs with raw filesystem traversal.
- Do not create a central taxonomy or registry of all possible concept types.
- Do not make repository the root memory taxonomy. Repository remains a facet under the OpenSymphony work graph.
- Do not require every OKF consumer to understand OpenSymphony extension fields.
- Do not require hosted mode, vector search, or code graph providers for the first OKF compliance slice.

## 4. OKF Interpretation For OpenSymphony

### 4.1 Bundle

An OpenSymphony OKF bundle represents one coherent memory scope. The default local scope is the current project set or local instance, not a repository checkout.

The bundle may include:

- projects
- milestones
- issues and sub-issues
- areas
- repositories
- code context artifacts
- run summaries
- citations and mirrored references
- generated topic docs

Repository-specific code intelligence is attached through scope refs and source refs. It is not the top-level bundle root.

### 4.2 Concept

Every durable memory document that participates in retrieval or graph navigation is an OKF concept.

Initial concept types:

| Type | Purpose |
| --- | --- |
| `issue-capsule` | Completed Linear issue or sub-issue memory. |
| `milestone-memory-node` | Milestone-level aggregation and navigation. |
| `project-memory-node` | Project-level aggregation and navigation. |
| `area-memory-node` | Stable subsystem or topic area. |
| `topic-doc` | Generated public or private topic documentation. |
| `run-summary` | Summarized execution attempt or retry history. |
| `code-context` | Generated codebase, symbol, dependency, or path-level context. |
| `repository-memory-node` | Repository facet and source metadata. |
| `reference` | Mirrored source material used by citations. |

These are producer-chosen values. Consumers must treat unknown values as generic concepts.

### 4.3 Concept ID

The concept ID is the bundle-relative Markdown path without the `.md` suffix.

Implementation rules:

- IDs must be stable across reindexing.
- IDs must be sanitized and remain inside the bundle root.
- Existing issue identifiers should remain visible in concept IDs.
- Moving a concept requires link rewriting or redirect metadata.
- A concept may carry legacy aliases for old capsule paths.

### 4.4 Frontmatter

All concept documents must start with YAML frontmatter.

Required OKF field:

```yaml
type: issue-capsule
```

Recommended OKF fields:

```yaml
title: COE-123: WebSocket reconnect recovery
description: Captures the delivered reconnect behavior, validation, and follow-ups.
resource: https://linear.app/example/issue/COE-123
tags: [openhands-runtime, websocket, recovery]
timestamp: 2026-06-13T17:00:00Z
```

OpenSymphony-specific metadata should be namespaced under `opensymphony` where possible:

```yaml
opensymphony:
  visibility: private
  kind: issue_capsule
  schema_version: 1
  scope_refs:
    - kind: project
      id: project-id
      label: OpenSymphony
    - kind: work_item
      id: COE-123
      label: WebSocket reconnect recovery
  source_refs:
    - kind: linear_issue
      id: COE-123
      url: https://linear.app/example/issue/COE-123
    - kind: github_pr
      id: "456"
      url: https://github.com/example/repo/pull/456
  docs_sync:
    status: pending
```

Backward compatibility:

- Existing top-level fields such as `issue`, `milestone`, `linear_url`, `areas`, and `source_refs` may remain during migration.
- New writers should also emit the namespaced form.
- Round-tripping must preserve unknown keys.

### 4.5 Body

The body remains standard Markdown. Capture should keep structural sections because both OKF and OpenSymphony retrieval benefit from headings, lists, tables, and fenced code blocks.

Issue capsules should continue to include:

- Original intent
- Relationships
- Outcome
- Decisions and actions
- Validation evidence
- Review and rework
- Follow-ups and risks
- Documentation impact
- Citations

Source-backed claims should appear in a `Citations` section. Existing `Source refs` output can be kept during migration, but the OKF export should provide a `Citations` section with numbered Markdown links to Linear, GitHub PRs, merge commits, source snapshots, or mirrored `reference` concepts.

### 4.6 Links

Standard Markdown links are the canonical relationship format.

Rules:

- Prefer bundle-relative absolute links such as `/issues/COE-123.md`.
- Relative Markdown links are allowed.
- Wiki links may be emitted only as an optional compatibility aid and must not be the only representation of an edge.
- Broken links are warnings, not conformance failures.
- Link context can be indexed for graph display, but the link itself remains an untyped relationship unless producer metadata adds a typed edge.

### 4.7 Reserved Files

`index.md` and `log.md` have OKF-defined meanings at every directory level.

`index.md`:

- Lists directory contents for progressive disclosure.
- Uses Markdown headings and bullet links.
- Should include the target concept description where available.
- Contains no frontmatter except the bundle-root `index.md`, which may declare `okf_version: "0.1"`.

`log.md`:

- Records update history for that scope.
- Uses ISO `YYYY-MM-DD` date headings.
- Lists newest entries first.

The current generated flat log should be migrated to date-grouped entries before the bundle is advertised as OKF-conformant.

## 5. Target Bundle Layout

The exact local storage path is an implementation detail. The logical bundle should look like this:

```text
bundle-root/
  index.md
  log.md
  projects/
  milestones/
  issues/
  areas/
  repositories/
  code/
  runs/
  references/
```

Recommended grouping:

- `projects/` contains project-level navigation concepts.
- `milestones/` contains milestone nodes.
- `issues/` contains issue and sub-issue capsules.
- `areas/` contains subsystem topics.
- `repositories/` contains repository facets.
- `code/` contains code-intelligence artifacts.
- `runs/` contains execution summaries when retained.
- `references/` contains mirrored citation material or source snapshots safe for the bundle's visibility.

Private and public bundles should be generated separately. Public bundles must exclude private capsules, private comments, private snapshots, and private local paths.

## 6. Producer Responsibilities

Memory capture must:

1. Select the target bundle scope.
2. Render each selected memory record as one OKF concept.
3. Emit required and recommended frontmatter.
4. Emit OpenSymphony extension metadata.
5. Use standard Markdown links for concept relationships.
6. Update the relevant `index.md` files.
7. Append date-grouped entries to the relevant `log.md` files.
8. Reindex the catalog from the written concepts.
9. Record capture warnings without making warnings fatal unless safety requires it.

Docs sync must:

1. Read from OKF concepts or the catalog derived from them.
2. Preserve the public/private boundary.
3. Avoid linking public docs directly to private bundle paths.
4. Include stable source identifiers such as issue keys, PR URLs, and merge SHAs when appropriate.

Import and backfill must:

1. Accept legacy capsule documents.
2. Normalize missing recommended OKF fields when derivable.
3. Preserve unknown frontmatter.
4. Report non-conforming documents with actionable diagnostics.

## 7. Consumer Responsibilities

OpenSymphony consumers include retrieval, docs sync, graph view, export, and hosted memory APIs.

Consumers must:

- Parse YAML frontmatter and Markdown body without requiring OpenSymphony-specific fields.
- Treat missing optional fields as normal.
- Treat unknown concept types as generic concepts.
- Preserve unknown fields when editing or rewriting.
- Tolerate broken links.
- Derive display titles from `title`, then heading, then filename.
- Derive descriptions from `description`, then first paragraph summary.

Consumers should:

- Synthesize indexes when missing.
- Extract directed graph edges from Markdown links.
- Extract typed auxiliary edges from OpenSymphony extension metadata.
- Redact or hide private concepts according to visibility and token scope.

## 8. Catalog And Index Boundary

The OKF bundle is durable source material. The catalog is derived.

The memory catalog may still store:

- concept ID
- type
- title
- description
- tags
- timestamp
- visibility
- scope refs
- source refs
- path or document-store reference
- extracted links
- citations
- body text for lexical search
- freshness and capture warnings

The catalog must be rebuildable from the bundle plus allowed external source snapshots. Read APIs should not mutate schema state.

## 9. CLI And API Surface

Add OKF-aware commands:

```bash
opensymphony memory lint --okf
opensymphony memory reindex --from-okf
opensymphony memory export-okf --visibility public
opensymphony memory export-okf --visibility private --output memory-bundle.zip
opensymphony memory import-okf path/to/bundle
```

The implemented first slice supports:

```bash
opensymphony memory lint --okf [bundle-root]
```

The optional `bundle-root` is resolved with the same repository-containment
check used by memory admin file arguments. Absolute paths are allowed only when
their canonical path remains inside the repository root. Relative paths are
resolved from the repository root. The memory MCP admin surface accepts the same
operation through `memory.lint` with `okf: true` and optional `bundleRoot`.

Existing commands keep their behavior:

```bash
opensymphony memory capture COE-123
opensymphony memory context --issue COE-456
opensymphony memory related --area openhands-runtime
opensymphony memory sync-docs --since-last-sync
```

The memory MCP server should expose equivalent admin tools where appropriate. The rich client should use gateway DTOs or memory server DTOs rather than walking private files directly.

## 10. Validation

`opensymphony memory lint --okf` should classify diagnostics.

Errors:

- Non-reserved Markdown concept lacks frontmatter.
- Frontmatter is not parseable YAML.
- Frontmatter lacks non-empty `type`.
- Concept path escapes the bundle root.
- Reserved file has invalid structure.
- Public export includes a private concept.

Warnings:

- Missing recommended fields.
- Unknown type.
- Broken Markdown link outside code spans, code fences, and HTML comments.
- Wiki-only link with no Markdown equivalent outside code spans, code fences,
  and HTML comments.
- Missing generated index.
- Citation section missing for source-backed claims.
- Unknown OKF version.

Info:

- Synthesized title.
- Synthesized description.
- Retained legacy fields.
- OpenSymphony extension metadata.

Generated `log.md` files must use ISO `## YYYY-MM-DD` headings in newest-first
order. OpenSymphony derives the date from completion time, then capture time, and
uses a stable ISO sentinel for malformed legacy timestamps so regeneration does
not depend on wall-clock time.

## 11. Migration Plan

### Phase 1: Enrich Existing Documents

- Keep current local memory paths.
- Add missing recommended OKF fields to issue capsules and milestone nodes.
- Generate `Citations` sections.
- Replace wiki-only links with Markdown links while preserving optional wiki aliases.
- Fix generated `log.md` files to use date headings.
- Add OKF lint fixtures for current capsule types.

### Phase 2: Introduce Bundle Roots

- Add a bundle-root `index.md` with `okf_version: "0.1"`.
- Move or mirror concepts into the logical bundle layout.
- Keep aliases for legacy capsule paths.
- Make DuckDB reindex from OKF concepts.
- Add public and private export modes.

### Phase 3: Graph And Hosted Integration

- Expose graph DTOs derived from OKF concepts.
- Add hosted visibility filtering.
- Add import/export interoperability tests.
- Add graph viewer deep links to concept IDs.

## 12. Test Plan

Required tests:

- OKF conformance fixtures for each concept type.
- Legacy capsule migration fixture.
- Root and nested `index.md` generation.
- Date-grouped `log.md` generation.
- Markdown link extraction for absolute, relative, broken, and external links.
- Unknown frontmatter preservation.
- Public export redaction.
- Catalog rebuild from bundle.
- MCP and CLI parity for lint, export, import, and reindex.
- Hosted token scope tests once hosted memory APIs exist.

## 13. Acceptance Criteria

- A captured issue can be exported as part of an OKF-conformant bundle.
- `opensymphony memory lint --okf` returns no errors for generated bundles.
- The catalog can be deleted and rebuilt from the OKF bundle.
- Public export excludes private concepts and private source material.
- Existing memory query commands continue to work.
- The graph viewer can build nodes and edges from the OKF bundle without OpenSymphony-private parsing rules.

## 14. Open Questions

1. Should the local document store move to the OKF bundle layout immediately, or should the first release mirror legacy capsules into a generated OKF export?
2. Should issue capsules use current kebab-case `type` values forever, or should a future major migration switch to more human-readable type strings?
3. Should redirects for moved concept IDs be stored in frontmatter, a generated redirect concept, or only in the catalog?
4. How much private source snapshot material should be eligible for private OKF export?

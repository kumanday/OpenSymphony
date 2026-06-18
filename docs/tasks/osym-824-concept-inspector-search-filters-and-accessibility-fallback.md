---
id: OSYM-824
title: Concept Inspector, Search, Filters, And Accessibility Fallback
milestone: "M11.5: LLM Wiki Graph View"
priority: 2
estimate: 8
blockedBy: ["OSYM-821", "OSYM-822"]
blocks: ["OSYM-826"]
areas:
  - frontend
  - graph-view
  - accessibility
parent: null
---

## Summary

Implement the human-readable concept inspector, command/search workflows, filters, keyboard navigation, and accessible list fallback for the graph view.

## Scope

### In scope

- Build left rail, right inspector, bottom status strip, mode controls, filters, and command/search flows.
- Render frontmatter as primary, relationship, resource, source support, system, advanced, and raw sections.
- Add keyboard navigation between visible graph nodes and neighbors.
- Provide semantic HTML list/table summaries for visible nodes and selected graph state.

### Out of scope

- Editing concept frontmatter or Markdown body.
- Writing memory links from the graph UI.

## Deliverables

- Inspector and filter UI components.
- Accessible list/table fallback.
- Keyboard and screen-reader summary behavior.

## Acceptance Criteria

- [ ] Users can select a concept and read human-friendly frontmatter without opening raw YAML.
- [ ] Filters cover bundle, concept type, tag, area, project, milestone, issue, repository, visibility, freshness, warning status, source kind, link kind, and community.
- [ ] Keyboard navigation and focus order are predictable.
- [ ] Canvas color is never the only signal for type, status, or community.

## Test Plan

- Run frontend component and accessibility tests.
- Run Playwright checks for keyboard navigation and inspector rendering.
- Run reduced-motion checks for layout stabilization behavior.

## Context

- Builds on OSYM-821 and OSYM-822.
- Read `docs/llm-wiki-graph-view-spec.md` sections 10, 11, 12, and 13.
- UI should remain dense, calm, and operational.

## Definition of Ready

- [ ] Hidden assumptions from prior discussion are written down.
- [ ] Required files, docs, and dependencies are explicitly referenced.
- [ ] A coding agent could begin execution without additional planning context.

## Notes

Raw YAML is available behind a toggle, not the default reading experience.

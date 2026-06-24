# TUI Dependency Gutter Specification

Status: draft

Reader: an OpenSymphony engineer updating the terminal operator UI.

Post-read action: implement compact project grouping and dependency signals in
the issue list without increasing the per-issue row count.

## 1. Summary

The FrankenTUI issue list should show the near-term dependency shape directly in
the existing one-row-per-issue list. The goal is not to render a full graph. The
goal is to answer, at a glance:

- which project each task belongs to when a project set is displayed
- which visible issues are active roots
- which visible Todo issues are waiting on active work
- which issue unlocks the next issue in the current dispatch chain
- whether the selected issue is ready, blocked, or blocking follow-on work

The modification should preserve the current vertical density. It may add one
compact group header per project represented in the issue list, but it must not
add extra rows per issue. The existing detail pane remains the place for
expanded metadata.

## 2. Goals

1. Keep one issue per rendered issue-list row.
2. Add a one-line project group header for each visible project in project-set
   mode.
3. Add a compact dependency gutter before the issue identifier.
4. Add a short inline dependency suffix after the title when width allows.
5. Use only unfinished active dependencies in the list-level signal.
6. Move completed blocker detail to the selected issue detail pane.
7. Keep narrow-terminal behavior predictable by truncating dependency suffixes
   before titles become unreadable.
8. Preserve read-only TUI behavior. Project and dependency display must not
   mutate tracker, scheduler, or Linear state.

## 3. Non-Goals

- Do not render a complete project dependency graph in the issue list.
- Do not repeat the project name on every issue row.
- Do not add a second line under blocked or blocking issues.
- Do not add new keyboard interactions for the first slice.
- Do not infer dependencies from title text.
- Do not use color as the only dependency signal.
- Do not let dependency rendering affect orchestrator scheduling decisions.

## 4. Source Data

The issue list may derive dependency display from the same control-plane issue
snapshot used for scheduling and detail rendering.

Required fields:

- issue identifier
- project identifier or slug
- project display name, when available
- repository or workspace label, when available
- runtime state
- tracker state
- title
- `blocked_by` issue identifiers and their terminal status, when available
- inverse `blocks` relationships, when available

If inverse relationships are not already present, the TUI may derive them from
the visible issue snapshot by reversing `blocked_by` edges. Missing inverse
edges should degrade to no downstream suffix rather than an error.

The view must distinguish these cases:

| Case | Meaning |
| --- | --- |
| no blockers | issue has no known upstream blockers |
| completed blockers only | issue is ready from a dependency perspective |
| unfinished blocker visible | issue is blocked by work currently shown in the list |
| unfinished blocker hidden | issue is blocked, but the blocker is outside the current visible window |
| downstream visible | issue blocks one or more visible active/Todo issues |
| downstream hidden | issue blocks work outside the current visible window |

## 5. Project Grouping

When the control-plane snapshot represents a project set, the issue list should
group visible issues by project and render a single project header before each
group.

Recommended header shape:

```text
== opensymphony-bootstrap | OpenSymphony | issues=12 running=2 todo=3 ==
```

The header should fit on one row and should prefer these fields, in order:

1. stable project slug or short identifier
2. human project name
3. repository or workspace label, when available
4. compact visible counts such as `running=2 todo=3 blocked=1`

If the terminal is too narrow, trim from right to left:

```text
== opensymphony-bootstrap | running=2 todo=3 ==
== opensymphony-bootstrap ==
```

In explicit project-set mode, render the header even when only one project is
currently visible. In single-project mode outside a project set, the header may
be omitted to preserve space.

Project headers are not selectable issue rows. Selection should skip them, and
row-windowing should keep enough context that the selected issue's project
header remains visible when feasible.

## 6. Issue Row Format

Current issue rows remain one visual line. The dependency version adds two
optional regions:

```text
== opensymphony-bootstrap | OpenSymphony | issues=12 running=2 todo=3 ==
+-- COE-411 [running / In Progress] Task Graph Editor And Runtime Overlay UI -> COE-417
|   COE-414 [idle   / Todo]        Diff, Validation, Approval, And Run Action Views <- COE-412
`-- COE-412 [running / In Progress] Runtime Timeline And Terminal/Log Association -> COE-414
```

The row shape is:

```text
<sel><gutter> <issue> [<runtime> / <tracker>] <title><suffix>
```

Where:

- `<sel>` is the existing selection marker.
- `<gutter>` is a fixed-width dependency marker.
- `<suffix>` is a compact dependency hint shown only when width allows.

The gutter should fit in three cells. A recommended first set:

| Marker | Meaning |
| --- | --- |
| `+--` | active root or issue with visible downstream work |
| `|  ` | issue is blocked by a visible unfinished issue |
| `` `--`` | active root ending a visible chain |
| `   ` | no active list-level dependency signal |

The marker is intentionally graph-like but not a full tree renderer. It gives
shape to the active chain while preserving one-row density.

## 7. Dependency Suffix

The suffix should be short and stable:

| Suffix | Meaning |
| --- | --- |
| `-> COE-417` | issue directly unlocks one visible issue |
| `-> COE-417 -> COE-418` | issue starts a short visible chain |
| `<- COE-412` | issue is blocked by one visible unfinished issue |
| `<- COE-411 -> COE-418` | issue is blocked and also blocks a follow-on issue |
| `<- 2 hidden` | blockers exist outside the visible window |
| `-> 3 hidden` | downstream blocked work exists outside the visible window |

Suffixes should prefer visible active/Todo issues over completed or backlog
issues. Completed dependencies should not appear in the issue-list suffix.

Suggested priority when space is tight:

1. visible unfinished upstream blocker, such as `<- COE-412`
2. visible downstream Todo issue, such as `-> COE-414`
3. hidden unfinished blocker count
4. hidden downstream count
5. longer downstream chain

When the row cannot fit both title and suffix, keep the issue identifier,
runtime/tracker state, and as much title as possible. Drop the suffix before
dropping the title below a useful minimum.

## 8. Selected Issue Detail

The detail pane should expand the same information without consuming issue-list
rows.

Examples:

```text
project: opensymphony-bootstrap | repo: OpenSymphony
deps: ready | blocks COE-414 | downstream COE-414 -> COE-430
```

```text
deps: blocked by COE-412 | blocks COE-430, COE-422
```

```text
deps: ready | completed blockers COE-402, COE-399, COE-405 | blocks COE-417
```

The detail pane may include completed blockers because it has room and the user
has explicitly selected the issue. The issue list should stay focused on
unfinished scheduling pressure.

## 9. Active Chain Example

Given this active set:

```text
COE-411 In Progress, blocks COE-417
COE-412 In Progress, blocks COE-414
COE-414 Todo, blocked by COE-412
COE-417 Todo, blocked by COE-411, blocks COE-418
COE-418 Todo, blocked by COE-417
```

The issue list should be able to render:

```text
== opensymphony-bootstrap | OpenSymphony | issues=12 running=2 todo=3 ==
+-- COE-411 [running / In Progress] Task Graph Editor And Runtime Overlay UI -> COE-417 -> COE-418
+-- COE-412 [running / In Progress] Runtime Timeline And Terminal/Log Association -> COE-414
|   COE-414 [idle    / Todo]        Diff, Validation, Approval, And Run Action Views <- COE-412
`-- COE-417 [idle    / Todo]        Planning Workspace UI <- COE-411 -> COE-418
    COE-418 [idle    / Todo]        Linear Draft Preview And Publish Flow <- COE-417
```

If the terminal is too narrow, the same list may collapse to:

```text
== opensymphony-bootstrap ==
+-- COE-411 [running / In Progress] Task Graph Editor And Runtime Overlay UI
+-- COE-412 [running / In Progress] Runtime Timeline And Terminal/Log Association
|   COE-414 [idle    / Todo]        Diff, Validation, Approval, And Run Action Views
`-- COE-417 [idle    / Todo]        Planning Workspace UI
    COE-418 [idle    / Todo]        Linear Draft Preview And Publish Flow
```

The gutter survives before the suffix because it provides useful shape at lower
width cost.

If multiple projects are visible, render each project as a separate compact
group:

```text
== opensymphony-bootstrap | OpenSymphony | running=2 todo=3 ==
+-- COE-411 [running / In Progress] Task Graph Editor And Runtime Overlay UI -> COE-417
|   COE-417 [idle    / Todo]        Planning Workspace UI <- COE-411 -> COE-418
    COE-418 [idle    / Todo]        Linear Draft Preview And Publish Flow <- COE-417

== companion-agent | Companion Agent | running=1 todo=1 ==
+-- CA-088  [running / In Progress] Normalize repo bootstrap events -> CA-091
|   CA-091  [idle    / Todo]        Show bootstrap warnings in dashboard <- CA-088
```

## 10. Ordering And Visibility Rules

Dependency markers should annotate the current issue order; they should not
silently reorder the list.

Recommended ordering is:

1. project group, using the project-set order from the control-plane snapshot
2. active or running issues inside that project
3. retry-queued issues
4. idle Todo issues
5. recently terminal issues

Within that order, the dependency display can make a visible chain apparent.
If a later design adds dependency-aware sorting, it should be a separate
operator-visible mode.

Hidden blockers and hidden downstream issues should be counted only when they
are unfinished and relevant to the selected issue or active chain. Do not count
completed blockers in the compact list.

## 11. Rendering Constraints

- Use ASCII markers so the display remains legible in constrained terminals and
  logs.
- Keep project headers to one row each.
- Keep the gutter width stable across all rows.
- Measure display width using terminal cell width, not byte count.
- Strip or normalize control characters before fitting rows.
- Never let suffix text cross the pane separator.
- Preserve the selected row marker and focus styling.
- Preserve existing row-windowing behavior that keeps the selected issue
  visible.

Color may reinforce status, but the ASCII markers and arrows must be sufficient
without color.

## 12. Acceptance Criteria

- The issue list still renders one visual line per issue.
- Project-set mode renders a one-line project header for every visible project.
- Project headers identify the project without repeating the project on every
  issue row.
- Existing active issue rows can show a dependency gutter without reducing the
  number of visible issue rows.
- A Todo issue blocked by a visible In Progress issue shows a compact upstream
  hint when width allows.
- An In Progress issue that unlocks visible Todo work shows a compact downstream
  hint when width allows.
- Completed blockers are omitted from the compact issue-list suffix.
- The selected issue detail pane shows expanded dependency information,
  including completed blockers when available.
- Narrow terminal rendering drops suffixes before it corrupts titles or pane
  separators.
- Missing dependency data degrades to blank markers and no suffix.
- Reducer and rendering tests cover ready, blocked, downstream, hidden, and
  narrow-width cases.
- Reducer and rendering tests cover single-project-set and multi-project-set
  grouping, including selection skipping project headers.

## 13. Open Questions

1. Should hidden blocker counts include Backlog issues, or only active/Todo
   issues?
2. Should selected rows show a longer suffix than unselected rows?
3. Should dependency-aware sorting become a separate optional issue-list mode?
4. Should the control plane eventually expose precomputed `blocks` edges, or
   should the TUI continue deriving inverse edges from `blocked_by`?
5. What is the shortest stable project identifier to show when a project slug is
   long or visually noisy?

# Linear Query Coverage Matrix

This matrix tracks which Linear GraphQL fields are queried by the OpenSymphony
Linear client (`crates/opensymphony-linear`) and whether they are normalized
into domain types and exposed via the gateway task graph.

## Entity Coverage

| Entity          | Fields Covered                                                                                          | Query Assets                                          | Normalized | Task Graph |
|-----------------|---------------------------------------------------------------------------------------------------------|-------------------------------------------------------|------------|------------|
| Project         | `id`, `name`, `slugId`, `url`, `content`                                                                | `PROJECT_BY_SLUG_QUERY`                               | Yes        | No         |
| ProjectMilestone| `id`, `name`                                                                                            | `ISSUES_BY_STATE_QUERY`, `ISSUE_BY_IDENTIFIER_QUERY`   | Yes        | No         |
| Issue           | `id`, `identifier`, `url`, `title`, `description`, `priority`, `createdAt`, `updatedAt`                 | `ISSUES_BY_STATE_QUERY`, `ISSUE_BY_IDENTIFIER_QUERY`   | Yes        | Yes        |
| IssueState      | `id`, `name`, `type`                                                                                    | All issue queries                                     | Yes        | Yes        |
| Parent          | `id`, `identifier`, `url`, `title`, `state.name`                                                        | `ISSUES_BY_STATE_QUERY`, `ISSUE_BY_IDENTIFIER_QUERY`   | Yes        | Yes        |
| Children        | `id`, `identifier`, `url`, `title`, `state.name`                                                        | `ISSUES_BY_STATE_QUERY`, `ISSUE_BY_IDENTIFIER_QUERY`   | Yes        | Yes        |
| Labels          | `name` (paginated)                                                                                      | `ISSUES_BY_STATE_QUERY`, `ISSUE_BY_IDENTIFIER_QUERY`, `ISSUE_LABELS_QUERY` | Yes | Yes |
| InverseRelations| `type`, `issue.id`, `issue.identifier`, `issue.title`, `issue.state`                                    | `ISSUES_BY_STATE_QUERY`, `ISSUE_BY_IDENTIFIER_QUERY`, `ISSUE_INVERSE_RELATIONS_QUERY` | Yes | Yes |
| Comments        | `id`, `body`, `updatedAt`, `resolvedAt` (paginated)                                                     | `ISSUE_COMMENTS_QUERY`                                | Yes        | No         |
| Assignee        | Not yet queried                                                                                         | —                                                     | No         | No         |
| Estimate        | Mapped via `priority` field (Linear does not expose separate estimate in free tier)                      | —                                                     | N/A        | N/A        |
| Attachments     | Not yet queried                                                                                         | —                                                     | No         | No         |
| Team            | `id`, `key`, `name`, `states`                                                                           | `ISSUE_TEAM_STATES_QUERY` (Linear skill only)         | No         | No         |

## Query Inventory

| Constant Name                    | Type      | Purpose                                                        |
|----------------------------------|-----------|----------------------------------------------------------------|
| `ISSUES_BY_STATE_QUERY`          | Query     | Paginated issues filtered by project and state names            |
| `ISSUE_BY_IDENTIFIER_QUERY`      | Query     | Single issue lookup by identifier (including archived)          |
| `ISSUE_STATES_BY_IDS_QUERY`      | Query     | Paginated issue state refresh by explicit issue IDs             |
| `ISSUE_LABELS_QUERY`             | Query     | Paginated label expansion for a single issue                    |
| `ISSUE_INVERSE_RELATIONS_QUERY`  | Query     | Paginated inverse relation expansion for a single issue         |
| `ISSUE_COMMENTS_QUERY`           | Query     | Paginated comments for a single issue                           |
| `ISSUE_ARCHIVE_MUTATION`         | Mutation  | Archive an issue                                                |
| `PROJECT_BY_SLUG_QUERY`          | Query     | Lookup project by slug ID                                       |
| `PROJECT_UPDATE_CONTENT_MUTATION`| Mutation  | Update project description content                              |

## Gaps (Out of Scope for This Ticket)

- **Assignee**: Not queried. Would require adding `assignee { id, name }` to issue
  fragments. Deferred to a follow-up ticket.
- **Attachments**: Not queried. Would require `attachments(first: N) { nodes { ... } }`.
  Deferred to a follow-up ticket.
- **Team-level reads**: Only available via the Linear skill introspection queries,
  not the Rust client. Sufficient for current needs.

## Schema Drift

Schema drift validation is handled by `opensymphony_linear::schema_drift`. It
performs GraphQL introspection against the live Linear API to verify that
required fields exist on the expected types. See `src/schema_drift.rs` for
the field matrix and validation logic.

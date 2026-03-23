# Changes Needed in Upstream Elixir Implementation for Hierarchy-Aware Task Selection

This document describes the specific changes needed in `~/dev/symphony/elixir` to add parent/sub-issue hierarchy support to the reference implementation.

## Files to Modify

### 1. `lib/symphony_elixir/linear/issue.ex`

Add `sub_issues` and `parent_id` fields to the Issue struct:

```elixir
defstruct [
  :id,
  :identifier,
  :title,
  :description,
  :priority,
  :state,
  :branch_name,
  :url,
  :assignee_id,
  :parent_id,          # NEW: parent issue ID (if this is a sub-issue)
  blocked_by: [],
  sub_issues: [],       # NEW: list of sub-issue references
  labels: [],
  assigned_to_worker: true,
  created_at: nil,
  updated_at: nil
]
```

Update the type spec:

```elixir
@type t :: %__MODULE__{
    id: String.t() | nil,
    identifier: String.t() | nil,
    title: String.t() | nil,
    description: String.t() | nil,
    priority: integer() | nil,
    state: String.t() | nil,
    branch_name: String.t() | nil,
    url: String.t() | nil,
    assignee_id: String.t() | nil,
    parent_id: String.t() | nil,           # NEW
    sub_issues: [sub_issue_ref()],          # NEW
    labels: [String.t()],
    assigned_to_worker: boolean(),
    created_at: DateTime.t() | nil,
    updated_at: DateTime.t() | nil
  }

@type sub_issue_ref :: %{
    id: String.t(),
    identifier: String.t(),
    state: String.t()
  }
```

### 2. `lib/symphony_elixir/linear/client.ex`

Extend the GraphQL query to fetch child issues:

```elixir
@query """
query SymphonyLinearPoll($projectSlug: String!, $stateNames: [String!]!, $first: Int!, $relationFirst: Int!, $after: String) {
  issues(filter: {project: {slugId: {eq: $projectSlug}}, state: {name: {in: $stateNames}}}, first: $first, after: $after) {
    nodes {
      id
      identifier
      title
      description
      priority
      state {
        name
      }
      branchName
      url
      assignee {
        id
      }
      labels {
        nodes {
          name
        }
      }
      parent {
        id
      }
      children(first: 50) {
        nodes {
          id
          identifier
          state {
            name
          }
        }
      }
      inverseRelations(first: $relationFirst) {
        nodes {
          type
          issue {
            id
            identifier
            state {
              name
            }
          }
        }
      }
      createdAt
      updatedAt
    }
    pageInfo {
      hasNextPage
      endCursor
    }
  }
}
"""
```

Add extraction functions:

```elixir
defp extract_sub_issues(%{"children" => %{"nodes" => sub_issues}})
     when is_list(sub_issues) do
  sub_issues
  |> Enum.map(fn issue ->
    %{
      id: issue["id"],
      identifier: issue["identifier"],
      state: get_in(issue, ["state", "name"])
    }
  end)
  |> Enum.reject(&is_nil(&1.id))
end

defp extract_sub_issues(_), do: []

defp extract_parent_id(%{"parent" => %{"id" => parent_id}})
     when is_binary(parent_id), do: parent_id
defp extract_parent_id(_), do: nil
```

Update `normalize_issue`:

```elixir
defp normalize_issue(issue, assignee_filter) when is_map(issue) do
  assignee = issue["assignee"]

  %Issue{
    id: issue["id"],
    identifier: issue["identifier"],
    title: issue["title"],
    description: issue["description"],
    priority: parse_priority(issue["priority"]),
    state: get_in(issue, ["state", "name"]),
    branch_name: issue["branchName"],
    url: issue["url"],
    assignee_id: assignee_field(assignee, "id"),
    parent_id: extract_parent_id(issue),           # NEW
    sub_issues: extract_sub_issues(issue),           # NEW
    blocked_by: extract_blockers(issue),
    labels: extract_labels(issue),
    assigned_to_worker: assigned_to_worker?(assignee, assignee_filter),
    created_at: parse_datetime(issue["createdAt"]),
    updated_at: parse_datetime(issue["updatedAt"])
  }
end
```

### 3. `lib/symphony_elixir/orchestrator.ex`

Add hierarchy check function:

```elixir
defp parent_issue_blocked_by_incomplete_children?(
       %Issue{sub_issues: sub_issues},
       terminal_states
     )
     when is_list(sub_issues) and length(sub_issues) > 0 do
  Enum.any?(sub_issues, fn
    %{state: sub_state} when is_binary(sub_state) ->
      !terminal_issue_state?(sub_state, terminal_states)

    _ ->
      # If we can't determine sub-issue state, assume it's blocking
      true
  end)
end

defp parent_issue_blocked_by_incomplete_children?(_issue, _terminal_states), do: false
```

Update `should_dispatch_issue?` to include hierarchy check:

```elixir
defp should_dispatch_issue?(
       %Issue{} = issue,
       %State{running: running, claimed: claimed} = state,
       active_states,
       terminal_states
     ) do
  candidate_issue?(issue, active_states, terminal_states) and
    !todo_issue_blocked_by_non_terminal?(issue, terminal_states) and
    !parent_issue_blocked_by_incomplete_children?(issue, terminal_states) and  # NEW
    !MapSet.member?(claimed, issue.id) and
    !Map.has_key?(running, issue.id) and
    available_slots(state) > 0 and
    state_slots_available?(issue, running) and
    worker_slots_available?(state)
end
```

Update `sort_issues_for_dispatch` to prefer leaf issues:

```elixir
defp sort_issues_for_dispatch(issues) when is_list(issues) do
  Enum.sort_by(issues, fn
    %Issue{} = issue ->
      # Prefer: higher priority (lower number), earlier created, fewer sub-issues, identifier
      sub_issue_count = length(issue.sub_issues || [])
      
      {priority_rank(issue.priority), 
       issue_created_at_sort_key(issue), 
       sub_issue_count,           # Prefer leaves (0 sub-issues) over parents
       issue.identifier || issue.id || ""}

    _ ->
      {priority_rank(nil), issue_created_at_sort_key(nil), 0, ""}
  end)
end
```

## Testing Changes

### Add to `test/symphony_elixir/linear/client_test.exs`:

Test sub-issue extraction:

```elixir
test "normalize_issue extracts sub-issues" do
  raw_issue = %{
    "id" => "issue-1",
    "identifier" => "TEAM-1",
    "title" => "Parent Issue",
    "children" => %{
      "nodes" => [
        %{
          "id" => "sub-1",
          "identifier" => "TEAM-2",
          "state" => %{"name" => "In Progress"}
        },
        %{
          "id" => "sub-2", 
          "identifier" => "TEAM-3",
          "state" => %{"name" => "Done"}
        }
      ]
    }
  }
  
  issue = Client.normalize_issue_for_test(raw_issue)
  
  assert length(issue.sub_issues) == 2
  assert Enum.any?(issue.sub_issues, &(&1.identifier == "TEAM-2"))
  assert Enum.any?(issue.sub_issues, &(&1.state == "Done"))
end
```

### Add to `test/symphony_elixir/orchestrator_test.exs`:

Test hierarchy blocking:

```elixir
test "parent issue with open sub-issues is not dispatched" do
  # Setup: create parent issue with sub-issues in active states
  # Verify: should_dispatch_issue? returns false
end

test "parent issue with all sub-issues terminal is dispatched" do
  # Setup: create parent issue with all sub-issues in Done/Cancelled
  # Verify: should_dispatch_issue? returns true (when other conditions met)
end

test "leaf issues are preferred over parent issues" do
  # Setup: create mix of parent and leaf issues with same priority
  # Verify: sort_issues_for_dispatch orders leaves first
end
```

## Migration Notes

- This change is backward compatible: issues without sub-issues have `sub_issues: []`
- Existing tests will pass because empty sub-issues list doesn't block dispatch
- The GraphQL query change adds a new field but doesn't remove existing fields

## Performance Considerations

- Sub-issues are fetched inline with the main issue query (no N+1)
- The hierarchy check is O(n) where n = number of sub-issues (typically small)
- Sorting now includes an additional field but complexity remains O(n log n)

---
tracker:
  project_slug: "example-project"
  active_states:
    - Todo
    - In Progress
  terminal_states:
    - Done
    - Cancelled
agent:
  max_turns: 3
openhands:
  conversation:
    reuse_policy: per_issue
    max_iterations: 32
---

{% if attempt is defined and attempt and attempt.continuation %}
## Continuation

You are continuing issue `{{ issue.identifier }}` after attempt {{ attempt.number }}.
Resume from the current workspace state and prior conversation history.
Do not restate the original assignment unless the workspace indicates it is missing.
{% else %}
# Assignment

You are working on issue `{{ issue.identifier }}`.

## Task
- Title: {{ issue.title }}
- State: {{ issue.state }}

{% if issue.description %}
## Description
{{ issue.description }}
{% endif %}
{% endif %}

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

# Assignment

You are working on issue `{{ issue.identifier }}`.

## Task
- Title: {{ issue.title }}
- State: {{ issue.state }}

{% if issue.description %}
## Description
{{ issue.description }}
{% endif %}

{% if attempt %}
## Continuation
Resume from attempt {{ attempt.number }} using the current workspace state.
{% endif %}

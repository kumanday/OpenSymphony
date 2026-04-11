# Raw GraphQL Fallback

Use this path when the required Linear operation is not covered by MCP and no
in-session `linear_graphql` tool is available.

Linear's public developer docs document this fallback path:

- GraphQL endpoint: `https://api.linear.app/graphql`
- auth: personal API key in the `Authorization` header
- schema discovery: the docs link directly to the live GraphQL schema and
  introspection is available when you need exact input shapes
- uploads: `fileUpload` returns a signed `uploadUrl` plus required headers for
  a server-side `PUT`

## Auth sources

Preferred auth inputs:

- `LINEAR_API_KEY`
- or `~/.config/opensymphony/secrets/linear-api-key.txt`

If the session already exposes `linear_graphql`, prefer that instead of reading
raw credentials from disk.

## Minimal Python transport

Use one GraphQL operation per call.

```bash
python - <<'PY'
import json
import os
import urllib.request

query = """
query Viewer {
  viewer {
    id
    name
    email
  }
}
"""

variables = {}

request = urllib.request.Request(
    "https://api.linear.app/graphql",
    data=json.dumps({"query": query, "variables": variables}).encode(),
    headers={
        "Content-Type": "application/json",
        "Authorization": os.environ["LINEAR_API_KEY"],
    },
    method="POST",
)

with urllib.request.urlopen(request, timeout=30) as response:
    print(response.read().decode())
PY
```

## Rules

- Keep exactly one operation per request.
- Use variables instead of interpolating values into the GraphQL string.
- Treat a top-level `errors` array as a failed Linear operation even if the HTTP
  request succeeded.
- Ask only for the fields needed by the current step.
- Reuse `tracker.project_slug` as Linear `Project.slugId`.
- If a mutation or input shape is unclear, introspect it before guessing.

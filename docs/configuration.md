# Configuration

This document covers target-repo bootstrap, generated files, and the runtime
configuration that `opensymphony run` expects.

## Bootstrap

Use `opensymphony init` from the target repository root:

```bash
cd /path/to/target-repo
opensymphony init
```

`opensymphony init` is the primary setup path for existing repositories. It:

- fetches the current starter files from the template repo's raw GitHub URLs
- copies missing files into the target repo
- merges an existing `AGENTS.md`
- prompts before overwriting other conflicting files
- fills the `WORKFLOW.md` clone hook from `git remote` when possible
- offers to fill the Linear project slug/key in `WORKFLOW.md`
- can optionally scaffold OpenHands AI PR review

The template repository is still the upstream source of those starter assets,
but it is an implementation detail of `opensymphony init`, not a required
manual setup step:

- [kumanday/OpenSymphony-template](https://github.com/kumanday/OpenSymphony-template)
- [Raw template base](https://raw.githubusercontent.com/kumanday/OpenSymphony-template/refs/heads/main/WORKFLOW.md)

## Files Added By `init`

Core bootstrap payload:

- `WORKFLOW.md`
- `AGENTS.md`
- `config.yaml`
- `.gitignore`
- `.agents/skills/`
- `.github/CODEOWNERS`
- `.github/pull_request_template.md`
- `docs/tasks/README.md`

Optional AI PR review scaffolding:

- `.github/workflows/ai-pr-review.yml`
- `.agents/skills/custom-codereview-guide.md`
- `docs/ai-pr-review-human-setup.md`

## Labels

If you want the template's GitHub label conventions, create them once per
repository:

```bash
gh label create "symphony" --description "PR created by OpenSymphony" --color "1f77b4" || true
gh label create "review-this" --description "Trigger AI PR review" --color "d73a4a" || true
```

## Review The Generated Workflow

After `init`, review `WORKFLOW.md` and `config.yaml`.

Important fields:

| Field | Description | Env Var | Example |
|-------|-------------|---------|---------|
| `tracker.project_slug` | Your Linear project identifier | - | `my-team/my-project` |
| `workspace.root` | Where to store per-issue workspaces | - | `~/.opensymphony/workspaces` |
| `openhands.conversation.agent.llm.model` | LLM model to use | `LLM_MODEL` | `openai/accounts/fireworks/models/glm-5p1` |

## Environment Variables

OpenSymphony uses standard OpenHands environment variable names.

Fireworks example via the OpenAI-compatible provider adapter:

```bash
export LLM_MODEL="openai/accounts/fireworks/models/glm-5p1"
export LLM_API_KEY="fw-..."
export LLM_BASE_URL="https://api.fireworks.ai/inference/v1"
```

The workflow supports `${VAR}` syntax for environment variable substitution in
the front matter:

```yaml
openhands:
  conversation:
    agent:
      llm:
        model: ${LLM_MODEL}
```

## Conversation Condensation

Optional conversation condensation is enabled by default per workflow to reduce
long-history context pressure before the agent-server hits the model window:

```yaml
openhands:
  conversation:
    agent:
      condenser:
        max_size: 240
        keep_first: 2
```

OpenSymphony forwards an OpenHands `LLMSummarizingCondenser` that reuses the
conversation agent's LLM settings. The condenser is enabled by default with
`max_size: 240` and `keep_first: 2`. To disable it, set `enabled: false`.

## Runtime Config

`opensymphony init` also copies a starter `config.yaml` next to the target
repository `WORKFLOW.md`.

Minimal local-supervised example:

```yaml
control_plane:
  bind: 127.0.0.1:2468

openhands:
  tool_dir: /absolute/path/to/OpenSymphony/tools/openhands-server
```

When your workflow points at an external OpenHands agent-server with
`openhands.transport.session_api_key_env`, `config.yaml` can omit
`openhands.tool_dir`.

Use [examples/target-repo/config.yaml](../examples/target-repo/config.yaml) as
the starting template if you want to inspect the checked-in example.

[examples/configs/local-dev.yaml](../examples/configs/local-dev.yaml) is for
`opensymphony doctor` against the OpenSymphony checkout. It is not the runtime
config that `opensymphony run` looks for in a target repo.

## OpenHands PR Review

If you opt into OpenHands PR review during `init`, finish the GitHub Actions
setup described in [ai-pr-review-human-setup.md](ai-pr-review-human-setup.md).

# OpenSymphony

OpenSymphony is a Rust implementation of the [OpenAI Symphony](https://github.com/openai/symphony) specification for orchestrating AI coding agents. It connects to [Linear](https://linear.app) for issue tracking and uses [OpenHands](https://github.com/OpenHands/OpenHands) as the agent runtime.

## What is OpenSymphony?

OpenSymphony automates software development workflows by:

1. **Polling Linear** for issues in active states (Todo, In Progress, etc.)
2. **Creating isolated workspaces** for each issue with lifecycle hooks
3. **Dispatching AI agents** via OpenHands to work on issues autonomously
4. **Managing retries, reconciliation, and cleanup** based on issue state changes
5. **Providing a terminal UI** (FrankenTUI) for monitoring and operator control

### Key Features

- **Hierarchy-aware scheduling**: Parent issues wait for sub-issues to complete
- **WebSocket-first runtime**: Real-time agent updates with REST reconciliation
- **Per-issue workspaces**: Deterministic, isolated directories with lifecycle hooks
- **Linear MCP integration**: Agent-side Linear writes (comments, state transitions)
- **Conversation reuse policies**: Default per-issue reuse with optional fresh-per-run resets
- **Local-first MVP**: Trusted-machine deployment with optional hosted mode

## Quick Start

### Using the Template for New Projects

For new projects that want to use OpenSymphony, use the template repository:

**[kumanday/OpenSymphony-template](https://github.com/kumanday/OpenSymphony-template)**

Click "Use this template" to create a new repository with:
- Pre-configured `WORKFLOW.md` with placeholders
- All required skills (`commit`, `push`, `pull`, `land`, `linear`, `convert-tasks-to-linear`, `create-implementation-plan`)
- GitHub workflows for AI PR review
- `AGENTS.md` template for project-specific context
- Labels: `symphony` and `review-this`

### Prerequisites

- Rust toolchain (stable)
- Python 3.12+ with `uv` for OpenHands server
- Linear API key (for tracker integration)
- LLM API key (any LiteLLM-compatible provider: OpenAI, Anthropic, Fireworks, etc.)

### Installation

```bash
# Clone the repository
git clone https://github.com/kumanday/OpenSymphony.git
cd OpenSymphony

# Install the CLI as `opensymphony`
cargo install --path .

# Inspect the command surface
opensymphony --help
```

### Configuration

For new projects, use the [OpenSymphony-template](https://github.com/kumanday/OpenSymphony-template) repository as a starting point.

For existing projects, copy these files from the template repo:

```bash
# From your target repository:

# 1. WORKFLOW.md (orchestration configuration)
curl -o WORKFLOW.md https://raw.githubusercontent.com/kumanday/OpenSymphony-template/main/WORKFLOW.md

# 2. Skills directory (commit, push, pull, land, linear, convert-tasks-to-linear, create-implementation-plan)
mkdir -p .agents/skills
for skill in commit land pull push linear convert-tasks-to-linear create-implementation-plan; do
  mkdir -p ".agents/skills/$skill"
  curl -o ".agents/skills/$skill/SKILL.md" \
    "https://raw.githubusercontent.com/kumanday/OpenSymphony-template/main/.agents/skills/$skill/SKILL.md"
done

# 3. GitHub workflows (AI PR review)
mkdir -p .github/workflows
curl -o .github/workflows/ai-pr-review.yml \
  https://raw.githubusercontent.com/kumanday/OpenSymphony-template/main/.github/workflows/ai-pr-review.yml

# 4. PR template and CODEOWNERS
curl -o .github/pull_request_template.md \
  https://raw.githubusercontent.com/kumanday/OpenSymphony-template/main/.github/pull_request_template.md
curl -o .github/CODEOWNERS \
  https://raw.githubusercontent.com/kumanday/OpenSymphony-template/main/.github/CODEOWNERS

# 5. Create required labels
gh label create "symphony" --description "PR created by OpenSymphony" --color "1f77b4" || true
gh label create "review-this" --description "Trigger AI PR review" --color "d73a4a" || true
```

Then edit `WORKFLOW.md` to set your project details:

| Field | Description | Env Var | Example |
|-------|-------------|---------|---------|
| `tracker.project_slug` | Your Linear project identifier | - | `my-team/my-project` |
| `workspace.root` | Where to store per-issue workspaces | - | `~/.opensymphony/workspaces` |
| `openhands.conversation.agent.llm.model` | LLM model to use | `LLM_MODEL` | `openai/gpt-5.4` |

**Environment Variables**

OpenSymphony uses standard OpenHands environment variable names:

```bash
# Required: LLM configuration
export LLM_MODEL="openai/gpt-5.4"
export LLM_API_KEY="sk-..."

# Optional: Custom base URL for non-OpenAI providers (e.g., Fireworks)
export LLM_BASE_URL="https://api.fireworks.ai/inference/v1"
```

The workflow supports `${VAR}` syntax for environment variable substitution in the front matter:

```yaml
openhands:
  conversation:
    agent:
      llm:
        model: ${LLM_MODEL}
```

Optional conversation condensation is enabled by default per workflow to reduce long-history context pressure before the agent-server hits the model window:

```yaml
openhands:
  conversation:
    agent:
      condenser:
        max_size: 240
        keep_first: 2
```

OpenSymphony forwards an OpenHands `LLMSummarizingCondenser` that reuses the conversation agent's LLM settings. The condenser is enabled by default with `max_size: 240` and `keep_first: 2`. To disable it, set `enabled: false`.

Add a `config.yaml` file next to your target repository `WORKFLOW.md`. A minimal local-supervised config looks like this:

```yaml
control_plane:
  bind: 127.0.0.1:3000

openhands:
  tool_dir: /absolute/path/to/OpenSymphony/tools/openhands-server
```

When your workflow points at an external OpenHands agent-server with `openhands.transport.session_api_key_env`, `config.yaml` can omit `openhands.tool_dir`.

See [`examples/target-repo/config.yaml`](examples/target-repo/config.yaml) for a checked-in example.

### Running the Orchestrator

```bash
# Run preflight checks from the OpenSymphony checkout
opensymphony doctor --config examples/configs/local-dev.yaml

# Start the orchestrator from the target repository
cd /path/to/target-repo
opensymphony run

# Or point at an explicit runtime config file
opensymphony run --config ./config.yaml

# Resume the persisted OpenHands conversation for one issue
opensymphony debug COE-284

# Rehydrate a conversation (recreate with current API key, preserving history)
opensymphony rehydrate COE-284 --reason "API key rotation"

# Bulk rehydrate all conversations during doctor check
opensymphony doctor --config examples/configs/local-dev.yaml --rehydrate

# Optional: Start the TUI for monitoring
opensymphony tui --url http://127.0.0.1:3000/
```

The legacy `opensymphony daemon` command is still available as a demo control-plane publisher for smoke tests, but it is not the real orchestrator entrypoint.

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                     OpenSymphony Daemon                      │
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────────────┐  │
│  │ Orchestrator│  │   Linear    │  │   OpenHands Client  │  │
│  │  Scheduler  │  │   Adapter   │  │  (REST + WebSocket) │  │
│  └──────┬──────┘  └──────┬──────┘  └──────────┬──────────┘  │
│         │                │                    │             │
│  ┌──────▼────────────────▼────────────────────▼──────────┐  │
│  │              Workspace Manager                        │  │
│  │   (per-issue directories, hooks, manifests)         │  │
│  └───────────────────────────────────────────────────────┘  │
│                           │                                 │
│  ┌────────────────────────▼────────────────────────────┐  │
│  │           Control Plane API (read-only)              │  │
│  │     GET /healthz, /api/v1/snapshot, /api/v1/events  │  │
│  └────────────────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────────────┘
         │                           │
         ▼                           ▼
┌─────────────┐              ┌─────────────────┐
│   Linear    │              │  OpenHands      │
│   (Issues)  │              │  Agent-Server    │
└─────────────┘              └─────────────────┘
         ▲                           ▲
         │                           │
    ┌────┴────┐                 ┌────┴────┐
    │   MCP   │                 │  Agent  │
    │  Tools  │                 │ Runtime │
    └─────────┘                 └─────────┘
```

### Component Overview

| Component | Responsibility |
|-----------|----------------|
| `opensymphony-orchestrator` | Poll loop, scheduling, retries, state machine |
| `opensymphony-linear` | GraphQL client for Linear read operations |
| `opensymphony-linear-mcp` | MCP server for agent-side Linear writes |
| `opensymphony-openhands` | REST/WebSocket client for agent runtime |
| `opensymphony-workspace` | Workspace lifecycle, hooks, containment |
| `opensymphony-control` | Control plane API and snapshot derivation |
| `opensymphony-tui` | FrankenTUI operator client |
| `opensymphony-cli` | CLI entrypoints: run, daemon (demo), tui, doctor, linear-mcp |

## Deployment Modes

### Local Supervised Mode (MVP)

The default mode for individual developers:

- One OpenHands server subprocess managed by the daemon
- Host filesystem access (process-level isolation)
- Loopback-only binding
- No auth by default

```yaml
openhands:
  transport:
    base_url: http://127.0.0.1:8000
```

### External Local Mode

For debugging or CI with a manually managed server:

```yaml
openhands:
  transport:
    base_url: http://127.0.0.1:8000
    session_api_key_env: OPENHANDS_API_KEY
```

### Hosted Remote Mode (Future)

For organizational deployment with stronger isolation:

```yaml
openhands:
  transport:
    base_url: https://agent-server.example.com
    session_api_key_env: OPENHANDS_API_KEY
  websocket:
    auth_mode: header
```

See [docs/deployment-modes.md](docs/deployment-modes.md) for full details.

## Workspace Lifecycle

Each issue gets a deterministic workspace:

```
<workspace_root>/<issue_identifier>/
├── .opensymphony/
│   ├── issue.json              # Issue metadata
│   ├── conversation.json       # Conversation registry and launch profile
│   └── openhands/
│       └── create-conversation-request.json
├── .opensymphony.after_create.json  # Hook receipt
├── <repo_files>                # Cloned repository
└── logs/                       # Execution logs
```

## Debugging Sessions

Use `opensymphony debug <issue-id>` to reopen the OpenHands conversation that OpenSymphony used for that issue:

```bash
cd /path/to/target-repo
opensymphony debug COE-284
```

The command resolves the issue reference to its managed workspace, reads
`.opensymphony/conversation.json`, and resumes the same `conversation_id` from the
original working directory. The conversation registry persists the issue reference,
stable OpenHands conversation ID, timestamps, transport details, and the launch
profile that created the session so a missing-but-recoverable thread can be
rehydrated without losing continuity.

When the workflow uses the local supervised OpenHands server, `opensymphony debug`
targets the same configured base URL as the orchestrator. If a ready server is
already listening there, the debug command reuses it; otherwise it starts a local
server for the session. For the most predictable behavior, prefer the
orchestrator-managed server and avoid leaving unrelated standalone `openhands`
CLI sessions bound to the same port.

### Lifecycle Hooks

- `after_create`: Clone repository, setup environment
- `before_run`: Pre-execution checks
- `after_run`: Post-execution cleanup
- `before_remove`: Final cleanup before workspace deletion

## Testing

```bash
# Unit tests
cargo test --workspace

# Static validation
cargo run -p opensymphony-cli -- doctor

# Live tests (requires OpenHands server)
OPENSYMPHONY_LIVE_OPENHANDS=1 cargo test -p opensymphony-openhands

# Smoke test
./scripts/smoke_local.sh

# Live E2E test
OPENSYMPHONY_LIVE_OPENHANDS=1 ./scripts/live_e2e.sh
```

## Documentation

- [Architecture](docs/architecture.md) - High-level design and component interactions
- [Deployment Modes](docs/deployment-modes.md) - Local vs hosted deployment
- [Testing and Operations](docs/testing-and-operations.md) - Test strategy and local ops
- [AGENTS.md](AGENTS.md) - Repository guidelines for coding agents
- [Development Guide](docs/DEVELOPMENT.md) - Contributing and development details

## Safety and Security

**Local Mode**: The MVP runs with process-level isolation on trusted developer machines. Agent code executes on the host filesystem. This is suitable for:
- Solo development on trusted repositories
- Local experimentation
- CI on controlled runners

**Hosted Mode** (future): Will provide stronger isolation with container-backed workspaces and mandatory auth.

## Version Pinning

OpenSymphony pins exact versions for reproducibility:

- `openhands-agent-server==1.14.0`
- `openhands-sdk==1.14.0`
- Rust stable toolchain

See `tools/openhands-server/` for the pinned environment.

## License

[LICENSE](LICENSE)

## Acknowledgments

- [OpenAI Symphony](https://github.com/openai/symphony) - The specification this implements
- [OpenHands](https://github.com/OpenHands/OpenHands) - The agent runtime
- [FrankenTUI](https://github.com/Dicklesworthstone/frankentui) - Terminal UI framework

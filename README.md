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
- **Conversation reuse**: Persistent OpenHands conversations across retry attempts
- **Local-first MVP**: Trusted-machine deployment with optional hosted mode

## Quick Start

### Prerequisites

- Rust toolchain (stable)
- Python 3.12+ with `uv` for OpenHands server
- Linear API key (for tracker integration)
- OpenAI API key (for agent runtime)

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

Copy `WORKFLOW.example.md` from this repository to your target repository as `WORKFLOW.md` and modify the values:

```bash
# From your target repository:
cp /path/to/OpenSymphony/WORKFLOW.example.md ./WORKFLOW.md
```

Key values to customize:

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
│   ├── conversation.json       # OpenHands conversation ID
│   └── openhands/
│       └── create-conversation-request.json
├── .opensymphony.after_create.json  # Hook receipt
├── <repo_files>                # Cloned repository
└── logs/                       # Execution logs
```

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

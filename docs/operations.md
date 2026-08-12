# Operations

This document covers the current local operator workflow for OpenSymphony.

Packaging note: crates.io publishes one package, `opensymphony`. The internal
`crates/opensymphony-*` directories are module trees inside that package, not
separately published dependencies.

## 1. Core commands

Recommended CLI commands:

- `opensymphony init`
- `opensymphony update`
- `opensymphony run`
- `opensymphony app` or `opensymphony desktop`
- `opensymphony debug <issue-id>`
- `opensymphony tui`
- `opensymphony doctor`
- `opensymphony rehydrate <issue-id> --reason "..."`

## 2. First-run flow

```bash
cargo install opensymphony
opensymphony install openhands
opensymphony --help

cd /path/to/target-repo
opensymphony init
opensymphony update
opensymphony run
```

If you already run an external OpenHands agent-server, you can skip
`opensymphony install openhands`.

The desktop launcher is intentionally lazy. `opensymphony app` and its visible
alias `opensymphony desktop` verify and launch a cached desktop bundle from
`~/.opensymphony/desktop/<version>/` without making the normal Cargo install
compile Tauri, npm, or platform desktop dependencies. On a cache miss, it reads
`opensymphony-desktop-release-index.json` from the versioned GitHub release for
the running CLI, downloads the compatible archive, verifies the archive and
installed manifest, then promotes the bundle into the versioned cache. Existing
cached bundles check the latest release index for newer compatible updates
before launch. Set `OPENSYMPHONY_DESKTOP_RELEASE_INDEX_URL` to test a fake
release server or use a private mirror. For early local testing, pass
`--bundle-dir <path>` or set
`OPENSYMPHONY_DESKTOP_BUNDLE_DIR` to a bundle directory containing
`opensymphony-desktop-manifest.json`. The manifest records the OpenSymphony
version, platform, architecture, relative executable path, and executable
SHA-256. Local bundle materialization copies regular files and directories;
symlinked bundle entries should be packaged by the downloaded archive path
instead of this local smoke path.
When no compatible prebuilt asset is available or a release download fails, a
normal run attempts the source-build fallback after checking Rust/Cargo,
Node/npm, source archive extraction, and platform desktop/Tauri prerequisites.
Interactive update prompts use `Update before launch? [Y/n]`; pressing Enter
accepts the update, and non-interactive runs update by default unless
`--no-update` is supplied.

Maintainers can build the current release bundle assets from a checkout:

```bash
npm run build --workspace=@opensymphony/desktop
npm run package:release --workspace=@opensymphony/desktop
```

The package command writes
`dist/desktop-release/opensymphony-desktop-v<VERSION>-<PLATFORM>-<ARCH>.tar.gz`
and `dist/desktop-release/opensymphony-desktop-release-index.json`. Upload the
archive first and the release index last. The index is the CLI-consumable
metadata file; it should never be published before the referenced archive is
available. If an index already exists in the output directory, the package
command preserves entries for other platform/architecture assets and replaces
only the current asset entry while keeping unknown top-level metadata. It also
fails before writing metadata when the desktop package, Tauri crate, or Tauri
config version does not match the release version, or when Cargo lockfile drift
would change the desktop dependency graph.
Use `--install-path <dir>` or `OPENSYMPHONY_DESKTOP_INSTALL_PATH` to choose a
custom install root. That root contains versioned bundles such as
`<dir>/<version>/`; it is not the bundle directory itself. `--dry-run` remains
read-only and never starts source-build prerequisite installation. Download
metadata, auto-update prompting, fallback order, and path-safety rules are defined in
[Desktop App Installer And Auto-Update Spec](specs/desktop-app-installer-auto-update-spec.md).

Important `init` behavior:

- fetches the current template payload
- leaves an existing `AGENTS.md` untouched and writes starter guidance to
  `AGENTS-example.md` during first-time setup
- prompts before overwriting repo-owned files
- optionally scaffolds AI PR review assets
- can configure GitHub Actions variables, the `review-this` label, and the
  optional AI review secret automatically when `gh` is installed and can access
  the target repository
- prompts whether to commit and push the generated OpenSymphony files; when
  accepted, it stages only files it wrote, commits `chore: bootstrap
  OpenSymphony`, and pushes `HEAD` to the detected remote
- supports `--non-interactive` for automation; pass explicit flags for prompt
  decisions and unresolved existing-file conflicts fail before any files are
  written
- copies `.agents/skills/` recursively so helper scripts, query files, and
  reference docs all arrive together
- keeps bootstrap guidance in CLI output and the central OpenSymphony docs
  instead of copying `docs/` files into the target repository

Automation-friendly target repo provisioning can run without stdin prompts:

```bash
cargo install opensymphony
opensymphony install openhands

cd /path/to/target-repo
opensymphony init \
  --non-interactive \
  --linear-project-slug my-linear-project \
  --conflict-policy overwrite \
  --commit-and-push
```

For scripts that scaffold AI PR review too, add the review flags explicitly:

```bash
opensymphony init \
  --non-interactive \
  --ai-pr-review \
  --configure-github \
  --ai-review-provider-kind openai-compatible \
  --ai-review-model-id accounts/fireworks/models/glm-5p1 \
  --ai-review-base-url https://api.fireworks.ai/inference/v1 \
  --ai-review-require-evidence true \
  --ai-review-secret-env LLM_API_KEY \
  --linear-project-slug my-linear-project \
  --conflict-policy overwrite
```

If `--configure-github` is omitted, init still writes the AI PR review files
when `--ai-pr-review` is present, but it prints the manual `gh` commands instead
of mutating repository variables, secrets, or labels. If a non-interactive run
finds an existing generated file and `--conflict-policy` was not supplied, it
fails before applying the template.
When `--ai-review-secret-env` is used, the named environment variable must be
present and non-empty; init fails rather than setting a blank GitHub secret.

For already-initialized repositories, `opensymphony update` is the fast
maintenance path:

- checks the latest published `opensymphony` version and skips
  `cargo install opensymphony --locked` when the running CLI is already current
- refreshes changed or new template-managed files under `.agents/skills/`
- leaves `WORKFLOW.md`, `AGENTS.md`, `.github/*`, and repo-local extra skills
  alone

OpenSymphony 2.11.0 raises the minimum supported Rust version to 1.97.1. An
older CLI may invoke Cargo through a checkout-local toolchain override, so use
this one-time upgrade path when moving from a release before 2.11:

```bash
rustup update stable
cargo +stable install opensymphony --locked
```

Use workflow settings mode when only the managed branch or review-provider
markers need to change:

```bash
opensymphony update --target-branch develop
opensymphony update --target-branch main
opensymphony update --target-branch release/next
opensymphony update --target-branch release/next --code-review openhands
```

Settings mode updates managed `WORKFLOW.md` markers, rewrites known legacy
branch-control phrases when the target branch changes, and skips the CLI
reinstall, template skill refresh, and memory bootstrap. `--code-review
openhands` records the marker and attempts to enable an existing
`.github/workflows/ai-pr-review.yml` through `gh workflow` but does not install
or repair a missing workflow file; `codex` and `none` record the marker and
attempt to disable an existing OpenHands review workflow. If `gh` is
unavailable, unauthorized, or cannot access Actions, verify or adjust the
workflow state manually.

Normal user installs use bundled DuckDB. This keeps `cargo install
opensymphony` and `opensymphony update` turnkey even when the memory database is
enabled.

Power users who want to avoid compiling bundled DuckDB may install a system
DuckDB development package and build without default features. On the
macOS/Homebrew development host, install and pin DuckDB once:

```bash
brew install duckdb
brew pin duckdb
```

Homebrew currently provides `duckdb`, not a versioned `duckdb@...` formula.
Pinning keeps the verified local version from moving during routine Homebrew
upgrades. The expected version for this release line is DuckDB `1.5.3`. To
build manually against that system library:

```bash
export DUCKDB_LIB_DIR="$(brew --prefix duckdb)/lib"
export DUCKDB_INCLUDE_DIR="$(brew --prefix duckdb)/include"
export DYLD_LIBRARY_PATH="$DUCKDB_LIB_DIR${DYLD_LIBRARY_PATH:+:$DYLD_LIBRARY_PATH}"
cargo install opensymphony --no-default-features --features duckdb-prebuilt
```

On Linux, set `DUCKDB_LIB_DIR`, `DUCKDB_INCLUDE_DIR`, and `LD_LIBRARY_PATH` to
the matching DuckDB installation. On Windows, set `DUCKDB_LIB_DIR`,
`DUCKDB_INCLUDE_DIR`, and add the DuckDB DLL directory to `PATH` before running
the same Cargo install command. This is a manual optimization path: verify a
memory command after installation, and expect to keep the runtime library
available anywhere the installed binary runs.

To update a power-user system-linked install, run the same Cargo install command
with the same environment first. Then run `opensymphony update` from a target
repository only to refresh template-managed agent assets. Starting with
`opensymphony update` may reinstall the default bundled build when a newer
release exists.

## 3. Recommended validation commands

For fast iterative development inside this repository on the macOS/Homebrew
host, use the system-linked developer aliases:

```bash
cargo fmt --check
cargo check-system-duckdb
cargo test-system-duckdb
cargo test-system-duckdb --test memory
cargo clippy-system-duckdb
```

If system DuckDB is unavailable, use the portable downloaded fallback aliases:

```bash
cargo check-dev
cargo test-dev
cargo clippy-dev
```

The system aliases set `DUCKDB_LIB_DIR`, `DUCKDB_INCLUDE_DIR`, and
`DYLD_LIBRARY_PATH` for the aliased command. The fallback aliases set
`DUCKDB_DOWNLOAD_LIB=1` only for the aliased command. Both alias families use
`--no-default-features --features duckdb-prebuilt`. If a downloaded fallback
command must override `CARGO_TARGET_DIR`, use an absolute path. Release-
sensitive, packaging, and dependency work should still include the default
bundled-mode checks so `cargo install opensymphony` remains turnkey for users:

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo test --test init
cargo test --test help
cargo test --test update
./scripts/smoke_local.sh
```

Dependency audit notes:

- COE-429 adds `jsonschema = 0.46.5` as the runtime validator for installed
  Codex app-server JSON Schema payload checks. Release provenance was checked
  against the `Cargo.lock` crates.io source/checksum entries for `jsonschema`
  and its called-out transitive crates (`fancy-regex`, `fluent-uri`, and
  `fraction`), the dependency tree was reviewed with `cargo tree -p jsonschema
  --depth 2`, and `cargo audit` exited successfully against the current lockfile
  on 2026-06-21. Re-run those checks when upgrading `jsonschema`.

Useful runtime checks:

```bash
curl http://127.0.0.1:2468/healthz
curl http://127.0.0.1:2468/api/v1/snapshot
curl http://127.0.0.1:2468/api/v1/capabilities
curl http://127.0.0.1:2468/api/v1/dashboard/snapshot
opensymphony tui --url http://127.0.0.1:2468/ --exit-after-ms 1200
```

## 4. Doctor expectations

`opensymphony doctor` is a real preflight tool.

It is optional troubleshooting/preflight help, not the primary install path for
managed local OpenHands. The normal setup flow is `cargo install opensymphony`
followed by `opensymphony install openhands`.

Current scope:

- loads and resolves the target repo `WORKFLOW.md`
- renders the workflow prompt with a synthetic issue
- validates required local tools
- validates bundled OpenHands tooling
- probes the configured OpenHands transport
- can create a temp conversation and verify runtime readiness

Expected checks include:

- config parses
- target repo exists
- `WORKFLOW.md` resolves cleanly
- required env-backed config values exist
- `cargo`, `curl`, `git`, and `uv` are on `PATH`
- the pinned OpenHands toolchain is present
- loopback/local safety warnings are surfaced

When the configured transport uses managed local OpenHands, `doctor` can
bootstrap the pinned tooling into the configured `openhands.tool_dir` before
continuing the rest of its checks.

## 4.0 Code Graph repository indexing

Trigger a target-branch repository snapshot through the gateway or desktop
native mirror:

```bash
curl -X POST http://127.0.0.1:2468/api/v1/code/repos/opensymphony/index
```

The server reads the branch marker in `WORKFLOW.md` (default `develop`) and
resolves the commit from `origin/<branch>` or the local branch. It reads Git
objects without running repository code, applies the configured Tree-sitter
limits, and writes immutable revision membership in bounded batches. Repeated
indexing of a later commit reuses unchanged paths and records deletions without
removing older revisions. Requests return an accepted report; inspect the event
journal for progress and the terminal completion/failure event. Concurrent
requests are serialized by the index writer.

The operator-facing equivalent is the Code Graph empty state: select the
configured repository and choose `Index repository`. It is safe to start from
an empty `.opensymphony/memory/memory.duckdb`; the repository row is exposed
with zero counts until the job begins. `accepted` and `progress` reports show
coverage, `failed` and `unavailable` reports show diagnostics with a retry
action, and `code_graph_updated` causes the shell to refresh the baseline. If
the event stream is silent during an accepted/progress job, the shell polls the
repository summary and refreshes as soon as an indexed baseline is visible.
The provenance strip should show the configured target revision and whether a
view is baseline, workspace-composed, stale, truncated, or partially analyzed.

For a production transport smoke, use the gateway endpoint rather than the
fixture workbench:

```bash
curl http://127.0.0.1:2468/api/v1/code/repos
curl -X POST http://127.0.0.1:2468/api/v1/code/repos/<repo-id>/index
curl 'http://127.0.0.1:2468/api/v1/code/repos/<repo-id>/graph?mode=atlas'
```

## 4.1 Subscription Credential Operations

OpenAI ChatGPT/Codex subscription mode is explicit and feature-gated. Build or
install OpenSymphony with `--features openhands-subscription-credentials`, then
configure the target repo workflow with
`openhands.conversation.agent.llm.credential_mode: openai_subscription`.

Credential establishment belongs to the documented OpenHands SDK flow or to a
future hosted credential broker. For local or self-hosted use, run the
OpenHands SDK browser or device-code login in the environment that owns the
credential store, keep refresh material in the selected auth directory, and
export only the short-lived access-token reference expected by the workflow
before starting `opensymphony run`. Do not place OAuth JSON files, access
tokens, or refresh tokens inside issue workspaces or repository files.
`auth_directory_env`, `auth_method`, `open_browser`, and `force_login` are
operator/bootstrap metadata for that credential setup step; they are preserved
for status and diagnostics, while the runtime conversation request resolves only
the short-lived access token and optional account identity header.

Validation for subscription mode should include:

- mocked subscription request construction tests
- redaction checks for manifests, diagnostics, and debug output
- live integration only when a valid subscription credential and pinned SDK
  support are available

Codex app-server subscription readiness is separate from the OpenHands SDK auth
directory. The gateway reports local Codex readiness through model settings by
running supported Codex CLI checks only:

```bash
codex --version
codex app-server --help
codex login status
```

When `codex login status` is logged out or expired, run
`codex login --device-auth`. Some ChatGPT accounts require enabling
**Security and login -> Enable device code authorization for Codex** before the
device-code flow succeeds. To revoke local Codex access, run `codex logout` and
use ChatGPT account settings for account-side revocation. OpenSymphony must not
read private Codex credential files or copy access/refresh material into
workspaces, logs, workflow files, Linear comments, or browser payloads. Gateway
readiness checks are cached briefly and have bounded per-command timeouts so
operator UI polling cannot hang on a stalled local Codex command.

The local Codex app-server harness path launches
`codex --dangerously-bypass-hook-trust app-server --stdio` and is advertised as
available when clients read `/api/v1/capabilities`. Before starting a run,
OpenSymphony generates the JSON Schema from the installed Codex CLI and
validates its full-automation `thread/start`, `thread/resume`, rollback
`thread/list`, `thread/archive`, `thread/unarchive`, and `turn/start` payloads. A new issue starts a thread; a
workspace with its canonical manifest resumes it. If the first manifest write
fails after a start, OpenSymphony archives that newly created thread and does
not start a turn. If the installed schema rejects any lifecycle payload, update
Codex before running the Codex harness. Unsupported or logged-out Codex
installations must fail with the readiness guidance above instead of partially
starting an issue. Loopback WebSocket and hosted Codex worker pools remain
non-production paths.

For cross-harness route testing, run `opensymphony run --dry-run`.
OpenSymphony will still poll Linear and prepare workspaces, but the worker
returns a route preview instead of launching a model-backed harness. The preview
is recorded as a `routing.decision` runtime event and includes the selected
harness, model, and model profile. To force a local process override without
editing workflow config, start the daemon with `OPENSYMPHONY_HARNESS`, and pass
`OPENSYMPHONY_MODEL` / `OPENSYMPHONY_MODEL_PROFILE` when a launcher wants to use
the active model profile selected in the desktop or web UI.

The Codex local stdio route executes the configured Codex binary with
`cwd == issue_workspace_path`. `OPENSYMPHONY_CODEX_BIN` is a trusted local
operator override and must not be treated as a hosted or multi-tenant input.
Approval requests are surfaced through normalized runtime events and shared
approval-center data models, but approval decisions are not yet forwarded from
the operator action plane into a live Codex stdio session in this alpha route.

The alpha model configuration panel exposed by the web and desktop shells uses
the shared model profile state store, but those entrypoints currently construct
it without durable storage. Treat profile edits as session-local until a
desktop secure-settings backend or hosted settings service is wired in. The UI
may keep model strings, routing hints, subscription bootstrap metadata, and
stored credential references in memory, but raw provider keys and OAuth refresh
material must stay in the selected keychain, OpenHands auth directory, or
hosted secret store.

## 5. Linear operational model

OpenSymphony 1.0.0 is GraphQL-only for agent-side Linear operations.

Operational implications:

- there is no separate local Linear bridge process to start
- initialized target repos rely on `LINEAR_API_KEY`
- operators may set `LINEAR_CLIENT_ID` and `LINEAR_CLIENT_SECRET` instead of
  relying on a long-lived `LINEAR_API_KEY`; `opensymphony run` mints a Linear
  OAuth client-credentials token at startup and uses it for scheduler and
  worker Linear calls
- `opensymphony run` keeps its local worker/snapshot tick every 5s, while
  Linear reads use cheaper internal cadences: running state every 30s,
  dispatch discovery every 60s, terminal cleanup every 5 minutes, and full
  issue details hourly after startup/dispatch
- if Linear returns a long rate-limit reset, the scheduler pauses all Linear
  reads behind one shared cooldown but continues processing worker updates; the
  Linear client only sleeps inline for short rate-limit retry windows up to the
  lower of `tracker.retry_policy.max_backoff` and 30 seconds
- the checked-in helper lives at
  `.agents/skills/linear/scripts/linear_graphql.py`
- checked-in query files under `.agents/skills/linear/queries/` are the
  supported mutation/query surface
- issue creation, issue rewrite passes, blocker relations, comments, PR
  attachments, and project updates should all use those checked-in assets

Smoke test:

```bash
cd /path/to/target-repo
python3 .agents/skills/linear/scripts/linear_graphql.py \
  --query-file .agents/skills/linear/queries/viewer.graphql
```

## 6. Project memory

Project memory stores policy and learned structure in
`.opensymphony/memory/memory.yaml` and private runtime artifacts under
`.opensymphony/memory/`. `opensymphony run` captures terminal issue transitions
automatically when `memory.auto_capture` is enabled in `config.yaml`:

```yaml
memory:
  auto_capture: true
  auto_archive: false
```

With a central instance config, automatic capture uses the configured catalog
even when the selected checkout has no repository-local memory YAML. After a
capsule write, the reload falls back to the normal default policy lookup rather
than treating the absent local file as an explicit required path.

Manual commands remain available for setup, backfill, inspection, and guarded
archive operations:

```bash
opensymphony memory init
opensymphony memory capture COE-123
opensymphony memory status
opensymphony memory brief COE-123
opensymphony memory related --paths crates/opensymphony-openhands
opensymphony memory sync-docs --since-last-sync
opensymphony memory lint --public-docs
opensymphony memory lint --okf
opensymphony memory reindex --from-okf
opensymphony memory export-okf --visibility public --output public-okf
opensymphony memory import-okf public-okf
```

Add `--dry-run` to write commands when an operator wants a non-writing preview.

Use `opensymphony memory import --source-file completed.yaml` only for
deterministic imports, migrations, tests, or external exports. Failed Linear or
GitHub access should be fixed before live capture is retried.

`memory capture` creates or refreshes issue capsules, updates
`.opensymphony/memory/memory.duckdb`, and refreshes markdown indexes when
enabled. Normal builds use DuckDB's bundled native library so operators do not
need to install DuckDB separately, at the cost of heavier Rust compile time and
a larger binary. Repository development can opt into the `duckdb-prebuilt`
feature through the system-linked `cargo check-system-duckdb`,
`cargo test-system-duckdb`, and `cargo clippy-system-duckdb` aliases, or the
downloaded fallback `cargo check-dev`, `cargo test-dev`, and `cargo clippy-dev`
aliases. Treat that native dependency as part of the hosted deployment threat
model before enabling memory in a multi-tenant service.
Memory capture does not archive Linear issues.

Read commands such as `memory status`, `memory brief`, `memory related`, and
`memory context` open the DuckDB index in read-only mode and do not run schema
migrations. Capture, import, OKF import/export, docs sync, reindex, archive,
automatic terminal capture, and code-intelligence persistence acquire the
instance coordination lock before writing. The local MCP server holds that
same lock for its lifetime, so migration and direct writers cannot copy or
index a torn catalog. Prefer the CLI or MCP admin surface for maintenance;
direct file or DuckDB access is an offline recovery and diagnostics fallback
only.

Each `opensymphony run` claims the configured state and workspace roots before
constructing tracker, memory, or workspace services. A second live process
using either root fails without polling Linear or creating a workspace. The
ownership marker records the process ID and, when available, a process-start
incarnation; stale markers are atomically quarantined before the root is
reclaimed, and a reused PID does not keep an old marker live when its
incarnation differs. The marker is released when the run shuts down, including
legacy single-repository runs.

When a worker outcome schedules a retry, the run manifest records the retry's
scheduled time, due deadline, reason, and redacted error summary. Restart
recovery restores those values and waits for the original deadline. Failed
stall-stop requests remain attached to the running execution until a later
stop attempt is acknowledged, so a remote worker cannot be forgotten after a
transient interrupt failure.
An interrupted `Preparing` or `Prepared` run with no conversation manifest is
recovered as a retry using its persisted retry count; once the configured
retry limit is reached it is parked as exhausted instead of being dispatched
as a fresh attempt.

For worker or tool access, `opensymphony run` starts the read-only memory server
when memory is initialized and `memory.serve` is not disabled. The supervised
server binds to loopback on an ephemeral port by default, reports the endpoint
through the control-plane recent events, and passes
`OPENSYMPHONY_MEMORY_ENDPOINT` into managed local OpenHands workers. Manual
operation is also available with `opensymphony memory serve --addr
127.0.0.1:8765`, which exposes MCP-style `initialize`, `tools/list`, and
`tools/call` JSON-RPC methods at `/mcp`. Set `OPENSYMPHONY_MEMORY_TOKEN` or
pass `--token` to require bearer-token access for read tools. Admin tools
(`memory.capture`, `memory.sync_docs`, `memory.lint`, `memory.reindex`,
`memory.export_okf`, `memory.import_okf`, and `memory.ingest_code_intel`)
require `OPENSYMPHONY_MEMORY_ADMIN_TOKEN` or `--admin-token`. When only the
admin token is configured, it also gates read tools; do not inject that token
into ordinary worker or shared managed-server environments. A configured
read token is likewise injected only as the per-conversation worker grant, not
into the shared managed-server environment. Authenticated operator calls using
the configured read or admin bearer may still perform ordinary read calls on a
supervised server; only unauthenticated unscoped reads are rejected as worker
requests. When `code_intel.enabled` is true,
`tools/list` exposes the read-only `code.graph.context` indexed discovery tool;
when `code_intel.ast.enabled` is true, it also exposes `code.ast.*` inspection
tools. The graph tool is bounded and can use the
server-resolved run workspace overlay; it never accepts a client filesystem
root or source-snippet override. The ad hoc
`code.ast.query` tool is available for local trusted use without tokens, and is
admin-gated when an admin token is configured. AST work runs off the async
server thread, enforces configured file/match/capture limits, rejects paths and
symlinks outside the repo root, skips generated/vendor/build/cache directories
during traversal and oversized files with trace warnings, and never executes
target-repo code. Direct file requests inside skipped directory names still pass
through containment and resource checks. See
[`docs/code-intelligence.md`](code-intelligence.md) for agent and operator
usage.

Linear archival is a separate command and is guarded by captured memory:

```bash
opensymphony linear archive --issues COE-123
```

For explicit issue selectors, the archive command captures live Linear and
GitHub evidence before evaluating the guard. It blocks issues that have no
capsule or unresolved capture warnings unless `--force` is supplied. Normal mode
resolves Linear credentials from `WORKFLOW.md` and calls the Linear GraphQL
archive mutation.

If the repo uses managed local OpenHands, archive also moves the issue's
persisted OpenHands conversation into the repo-scoped `archived/` store. Archive
uses the workspace `.opensymphony/conversation.json` manifest when present and
falls back to scanning managed conversation `meta.json` files for a matching
`workspace.working_dir` issue key, so legacy flat conversations and repo-scoped
`active/` conversations can still be moved even when workspace metadata is
stale. Normal orchestrator runs use the sibling `active/` store, while
`opensymphony debug COE-123` searches active and archived stores and starts the
managed server against the store containing the requested conversation. If
another OpenHands server is already bound to the configured port with a
different store, stop it and retry the debug command.

For issues last run through the local Codex app-server harness,
`opensymphony debug COE-123` reads the recorded Codex thread id, unarchives it
when terminal reconciliation archived it, and then runs `codex resume
<thread-id>` from that exact issue workspace. Set `OPENSYMPHONY_CODEX_BIN` to
override the Codex binary. Use `opensymphony debug COE-123 --app` to unarchive
and print `codex://threads/<thread-id>` without launching interactive Codex.

See [Project Memory](memory.md) for the full command surface, import YAML
schema, and troubleshooting notes.

## 7. Rehydration

Rehydration is the explicit recreation of an OpenHands conversation while
preserving enough history for continuation.

Use it for:

- API key rotation
- broken persisted conversation state
- intentional provider/model changes

Examples:

```bash
opensymphony rehydrate COE-123 --config ~/.opensymphony/config.yaml --reason "API key rotation"
opensymphony doctor --config ./config.yaml --rehydrate
```

## 8. Local safety

- prefer loopback-only OpenHands targets for local development
- treat target repos and prompts as trusted local input
- do not keep unrelated OpenHands servers running on the same configured port
- stop `opensymphony run` with Ctrl-C so the orchestrator can terminate its
  managed OpenHands process tree; Ctrl-Z only suspends the orchestrator and can
  leave the server bound to the configured port
- do not store provider secrets in checked-in files

## 9. Migration note

Central configuration migration is an explicit operator action. Use
`opensymphony migrate preflight --repo <path>` to inspect legacy
`config.yaml`/`WORKFLOW.md` without changing files, then use `migrate apply`
with the same paths to create a staged central config. The generated config
uses `legacy_single`, so migration does not activate strict multi-repository
routing. It records a config generation and an activation marker, preserves
the workflow body as repository implementation guidance, and keeps a backup
under `.opensymphony/migration/backups/`. Reports contain only paths, hashes,
field names, and boolean risk indicators; literal secret values and
credential-bearing remote values are never printed or serialized.

If apply is interrupted after staging or replacement, run
`opensymphony migrate rollback --config <central-config>` once the strict-run
marker is absent. Rollback restores the backed-up runnable generation and
leaves the backup evidence in place. Migration rejects repository-creation
hooks, query/fragment-bearing remotes, literal credentials embedded in hook
commands, and ambiguous credential expressions before activation; hook
credentials must use environment indirection. Existing repository-local
memory entries are copied into
the central catalog; identical repeat applies are idempotent, while divergent
entries fail as a recoverable conflict instead of silently keeping stale data.
The memory server marks its catalog as active while running, so read-only
preflight can inspect a live legacy writer without copying or writing anything.
Apply and
the server claim the same atomic `.opensymphony/memory.migration.lock` before
reading or copying the catalog; the server holds it for its lifetime. The lock
and activity marker record an owner PID and process incarnation, so stale
ownership from an unclean exit can be reclaimed while a live owner still
blocks migration/startup.
Stale lock recovery atomically renames the old lock to a unique quarantine file
before removing it; it never removes a newly-created owner lock at the shared
path. Project-set central configs are supported by doctor modes; probes use the
selected repository policy and do not inspect an unrelated launch-directory
checkout. Strict attach still requires a compatible verified checkout and
runtime envelope.
After front matter is moved, `doctor`, `debug`, and `rehydrate` load the central
policy so operational recovery continues to use the migrated OpenHands and
tracker settings.
For `legacy_single`, the same central policy resolves the selected repository's
`instructions.path` beneath its checkout instead of silently reverting to the
checkout root `WORKFLOW.md`.

Strict repository-bound runs publish checkout generations under the configured
workspace root. Operators should treat a generation manifest as immutable
provenance: it records the canonical remote fingerprint, target branch and
commit, instruction hash/source commit, and verification state. Drift or
partial publication is quarantined and retried as a new generation; do not
manually reset a quarantined checkout into service. The runtime envelope also
records that current local containment is process `cwd` containment on a
trusted host, not a sandbox boundary.

Strict `opensymphony rehydrate` also derives the desired repository, harness,
model, and generation envelope from the current central routing inventory before
creating a replacement conversation. If that envelope differs from the
persisted run, rehydration stops and leaves the existing conversation intact
until the checkout is rematerialized or the configuration is reconciled.
When several configured aliases identify the same repository, recovery first
preserves and validates the alias recorded in the persisted binding; an alias
that now resolves to a different repository is rejected rather than silently
rewritten.

Terminal OpenHands archival uses the retry verification mode for the checkout.
That mode still requires the recorded generation, repository binding, ancestry,
and instruction provenance, while permitting ordinary worker commits or dirty
worktree changes that a terminal worker legitimately left behind.
When a route switches between OpenHands and Codex, the previous session remains
active until the replacement manifest has been durably written with the expected
runtime envelope and conversation binding; a failed replacement therefore does
not destroy the session needed for recovery. Before that replacement starts, its
previous conversation manifest is also recorded in
`.opensymphony/superseded-conversations.json`. Restart recovery and terminal
cleanup use that durable evidence to archive the old OpenHands conversation or
Codex thread, and successful retirement clears the evidence without overwriting
the replacement manifest.

Rollback refuses to proceed when the central catalog fingerprint differs from
the activation marker. This deliberate safety stop keeps captures made after
migration visible instead of restoring a legacy config that would hide them;
remove or reconcile the divergent catalog only through an explicit recovery
operation.

Central-config `opensymphony run` holds a destination-hashed
`.opensymphony/migration/strict-run-<destination>.active` marker until the
process exits. Rollback claims that same marker for its full restore, so it
cannot replace the active generation underneath a running instance, and stale
markers are reclaimed only after owner liveness is disproved. Graceful run
shutdown awaits the memory-server task before returning, ensuring its activity
marker and coordination lock are released.
Lock ownership treats permission-denied Unix PIDs as live, compares the
recorded process incarnation when available, and uses native process creation
times alongside `tasklist` for stale-lock recovery on Windows. Restart recovery
preserves successful
terminal workspaces according to the configured retention policy, rejects
pending retries that exceed a newly lowered retry limit, and redacts
credential-shaped diagnostics before persisting them in `run.json`. If a
terminal or nonterminal tracker transition cannot stop the remote harness,
the scheduler retains the execution and retries the interrupt on the next
reconciliation; terminal recovery also honors `workspace.retain_failed`.
Malformed central-only memory configuration is detected before legacy fallback
so it fails validation rather than polling or creating a workspace.

Activation markers are namespaced by the absolute central-config destination,
so separate instances cannot overwrite or consume one another's rollback
record.

If an older target repo still contains `openhands.mcp`, remove that block.
OpenSymphony 1.0.0 expects Linear access through `LINEAR_API_KEY` and the
repo-local GraphQL helper assets copied by `opensymphony init`.

<!-- BEGIN OPENSYMPHONY MANAGED MEMORY SYNC -->

## Current model

- COE-461 contributed: PR #164: Expose memory graph DTOs and endpoints (merge `762cec5`)
- COE-464 contributed: PR #165: Derive OKF memory graph metrics and communities (merge `7d58035`)
- COE-465 contributed: PR #166: Add shared frontend graph package (merge `21281d0`)
- COE-467 contributed: PR #171: Add Knowledge Graph renderer (merge `1337903`)
- COE-468 contributed: PR #169: Add Knowledge Graph inspector surface (merge `960541a`)
- COE-469 contributed: PR #170: Wire live memory graph privacy gates (merge `11ac876`)

## Important invariants

- Preserve the behavior described in the recent captured changes unless current code and tests show it has changed.
- Use capsule source refs to inspect the original PR or Linear issue when context is ambiguous.

## Operational flow

- No generated diagram requested for this sync.

## Known gotchas

- No area-specific gotchas were inferred from the selected memory.

## Recent changes

- COE-461: Memory Graph DTOs And Gateway Endpoints
- COE-464: Graph Extraction, Metrics, And Community Pipeline
- COE-465: Shared Graph Frontend Package And Reducers
- COE-467: Three.js Graph Renderer And Worker Layouts
- COE-468: Concept Inspector, Search, Filters, And Accessibility Fallback
- COE-469: Live Memory Graph Integration And Privacy Gates
- COE-471: Graph Scale, Visual Regression, And Web/Desktop Hardening
- COE-498: Tree-sitter Provider Skeleton And Rust Parsing
- COE-499: Memory Context AST Provider Integration
- COE-500: Query Packs For Supported Agent Languages
- COE-501: Code Intelligence Persistence And Ingestion
- COE-502: Read-Only AST MCP And CLI Tools
- COE-503: Code Intelligence Performance Docs And Hardening
- COE-505: Add scheduler-side Codex stdio interrupt channel
- COE-506: Invert CodeIntelIndex trait ownership after AST memory integration
- COE-507: Deduplicate query-pack assets for grammar variants
- COE-508: Cache code-intel parsers and compiled query packs
- COE-520: Route desktop Knowledge Graph through native gateway commands
- COE-521: Workflow Target Branch Model And Init Customization
- COE-522: Init Target Branch Prompt And Flag
- COE-523: Update Workflow Settings Mode
- COE-524: Template Docs And Settings Hardening
- COE-525: Desktop Installer Contract And Release Metadata
- COE-526: Desktop Release Bundle Pipeline
- COE-527: Source Build Fallback And Prerequisites
- COE-528: App Download Install And Launch Flow
- COE-529: Desktop Auto-Update Flow
- COE-530: Installer Docs And End-To-End Validation
- COE-531: Workspace Shell Graph Hero And Surface State
- COE-532: Symbol Identity Container Chain And Code Read Model
- COE-533: Code Graph DTOs Gateway Routes And Native Commands
- COE-534: Code Graph Frontend Surface Adapters And Inspector
- COE-535: Run Diff Symbol Navigation And Code Overlay
- COE-536: Cross Graph Code Memory And Work Chips
- COE-537: Code Graph Scale Accessibility And Parity Hardening
- COE-540: Canonical Codex Thread Reuse And Workspace Retention
- COE-541: Durable Codex Thread Archive And Debug Recovery
- COE-542: Target Branch Code Index And Revision Snapshots
- COE-543: Workspace Code Overlay And Composite Graph
- COE-544: Indexed Agent Code Context And Retrieval
- COE-545: Edge Delta And Module Topology Diff
- COE-546: Code Graph Bootstrap UX And End-To-End Validation
- COE-547: Central Multi-Repository Config And Safe Migration
- COE-548: Canonical Repository Binding And Task Propagation
- COE-549: Verified Checkouts Instructions And Harness Envelopes
- COE-550: Per-Instance Memory Catalog And Source Migration
- COE-551: Scoped Cross-Repository Memory And Leaf Overlays

## Source refs

- COE-461
- COE-464
- COE-465
- COE-467
- COE-468
- COE-469
- COE-471
- COE-498
- COE-499
- COE-500
- COE-501
- COE-502
- COE-503
- COE-505
- COE-506
- COE-507
- COE-508
- COE-520
- COE-521
- COE-522
- COE-523
- COE-524
- COE-525
- COE-526
- COE-527
- COE-528
- COE-529
- COE-530
- COE-531
- COE-532
- COE-533
- COE-534
- COE-535
- COE-536
- COE-537
- COE-540
- COE-541
- COE-542
- COE-543
- COE-544
- COE-545
- COE-546
- COE-547
- COE-548
- COE-549
- COE-550
- COE-551

<!-- END OPENSYMPHONY MANAGED MEMORY SYNC -->

# Multi-repository release gate and isolated rollout

Strict multi-repository routing stays disabled for production projects until the
hermetic gate and one disposable non-production run pass at the same immutable
commit, with each selected central config identified by its SHA-256.

## Hermetic gate

Run:

```bash
OPENSYMPHONY_RELEASE_CONFIG=/absolute/path/to/candidate-config.yaml \
  scripts/hermetic_multi_repo_lifecycle.sh
```

The gate uses local temporary repositories and fake tracker, provider, and
harness implementations. The workspace fixture creates three local bare remotes
with different instructions, retains child generations, and prepares parent
worktrees from child object stores. The three-repository scheduler scenario
carries its repository IDs, child merge identities, and retained generation
handles through one selected repair, provider PR and merge, final verification,
capture, and bottom-up cleanup. The workspace fixture separately verifies Git
worktree detachment and generation-bound cleanup. Scheduler fixtures serialize
durable state around intents and receipts and reconcile provider state before
repeating a side effect.

The output `release-evidence.json` records the exact Git commit, selected config
path and SHA-256, completion time, log path, and a `production_activation:false`
claim. A dirty working tree is useful during development, but rollout evidence
is accepted only when the recorded commit is the checked-out clean `HEAD` and CI
passes for that commit.

Copy `docs/tasks/evidence/osym-895-release-evidence.template.json` into the
release record and fill it from the generated hermetic and live artifacts. The
release reviewer checks the candidate commit and config hashes, green CI,
requested-change repair, parent merge, teardown receipt, explicit
non-production project set, and `production_activation:false` before activation.

## Numbered lifecycle and fault matrix

| Boundary | Durable transition or side effect | Hermetic evidence | Injected failures and typed result |
|---|---|---|---|
| H01 | Resolve the selected central config and routing inventory | `central_config_*` | unknown fields, overlapping roots, invalid secrets/paths, unsupported provider and project scope fail validation |
| H02 | Preflight, apply, activate, and roll back legacy state | migration tests plus `apply_and_rollback_restore_legacy_files` | conflicts, edited inputs, interrupted catalog copy, active writer, invalid backup and rollback drift remain recoverable |
| H03 | Claim the runtime/state/catalog roots | `runtime_root_ownership_*`, `strict_run_marker_*`, memory coordination tests | a live owner blocks a second process; stale or malformed ownership is handled without stealing a live root |
| H04 | Resolve terminal-child repository bindings | scheduler binding tests and config routing tests | missing, unknown, multiple, disallowed, parent, out-of-scope, stale, and changed bindings remain distinct blocked/superseded states |
| H05 | Materialize and run three child checkouts | `parent_execution_root_reuses_three_repositories_and_preserves_children` plus harness/session suites | wrong remote, dirty checkout, stale generation, unsafe Git configuration, instruction mismatch, and malformed runtime evidence block before attach |
| H06 | Record provider-confirmed child merges, freeze hierarchy, and acquire ancestor leases | hierarchy/scheduler parent admission tests | missing or stale merge evidence and post-freeze hierarchy changes block without releasing required or higher-owner leases |
| H07 | Prepare contained parent integration worktrees from retained generations | three-repository workspace fixture | no fresh clone, no instruction leak, no path escape, no stale handle, and no cleanup while an ancestor lease is active |
| H08 | Run bounded parent checks and authorize memory | parent runtime-envelope, memory-grant, overlay, and base-commit tests | process timeout/indeterminate cleanup is retryable; unrelated live overlay and widened repository/work-item scope are denied |
| H09 | Persist one affected-repository repair intent, branch, push, and PR | `parent_repair_*` side-effect ledgers | provider outage, external closure, force push, stale receipt, and persistence loss remain resumable without duplicate branch, push, or PR |
| H10 | Apply requested changes, review, checks, and merge | requested-change and current-head provider tests | failed checks, rejected/stale review, exhausted review budget, missing merge intent, and merge conflict block merge without duplicating review or merge writes |
| H11 | Refresh every repository and record final verification/capture | repair refresh, exact run/commit receipt, capture retry, and memory provenance tests | stale target, missing child reachability, wrong conversation, malformed evidence, and capture failure preserve the controller for retry |
| H12 | Detach Git worktrees, release leases bottom-up, and delete eligible roots | subtree cleanup and three-repository cleanup tests | cleanup failure resumes only incomplete receipts; stale generations and active or higher-ancestor leases prevent deletion |
| H13 | Project state through domain, gateway, TUI, web, and desktop | Rust projection/round-trip tests and TypeScript contract/build checks | unavailable facts remain absent, secrets and exact host paths stay private, and blocked/releasing states do not render as completed |

Every side-effecting scheduler path persists an intent before the write and a
receipt after it. The gate pairs each boundary with restart/reconciliation tests:
branch, push, PR, review, merge, capture, refresh, detach, lease release, and
cleanup counters remain one after replay. The workspace fixture also recovers an
interrupted refresh transaction before accepting the parent root.

## Disposable non-production run

The live gate runs the hermetic suite against its generated central config and
records that config hash before starting the isolated orchestrator:

```bash
OPENSYMPHONY_LIVE_MULTI_REPO=1 \
OPENSYMPHONY_LIVE_GITHUB_OWNER=<disposable-owner> \
OPENSYMPHONY_LIVE_LINEAR_TEAM_ID=<non-production-team-id> \
OPENSYMPHONY_LIVE_MODEL=<bounded-model> \
  scripts/live_multi_repo_rollout.sh
```

The script creates uniquely named private repositories, a uniquely named Linear
project and hierarchy, isolated state/workspace/catalog roots, and a unique
loopback port. It caps runtime, task count, retry count, and model turns. The
alpha fixture uses a required GitHub Actions check to exercise a
failed-check rework loop on the same PR branch. Repository code review follows
the configured automated OpenHands or Codex integration; the fixture does not
require a separate reviewer account. The rollout controller publishes child
edits after a successful worker run using its own GitHub credential; the
worker never receives the checkout credential. Resource names are recorded
before creation and reconciled during teardown if a provider response is lost.
The script tears down processes,
branches, pull requests,
issues, project, repositories, port ownership, credential copies, and local
roots, then writes a teardown inventory. A missing cleanup receipt fails the
gate.

The active project set in that generated config contains only the disposable
Linear project and the three disposable repositories. The release evidence must
record its exact config hash before `opensymphony run` starts. Production project
sets remain unchanged.

## Rollback and staged expansion

Stop the isolated instance, verify it has no unresolved multi-repository run,
and switch its selected central config back to `legacy_single`. If the config
came from migration, run `opensymphony migrate rollback --config <path>` only
after the strict-run marker is absent. Preserve the catalog, controller state,
manifests, provider receipts, retained generations, and teardown report until
rollback verification completes.

Expansion uses a new reviewed config generation for each project set. Re-run the
hermetic gate and one disposable live run at the exact candidate commit/config
hash, enable one explicit non-production project set, observe a complete clean
lifecycle, and then make a separate operator decision for any additional set.
Production activation is outside this gate.

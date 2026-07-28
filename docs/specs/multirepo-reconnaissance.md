# OpenSymphony multi-repo evolution: consolidated reconnaissance and review plan

**Source aliases used below**

- `upstream`: `sources/01-upstream-develop`, pinned at `d72bb0a409c006e20f2b0d766a0def218f94cd41` on the v2.10.1 release line.
- `fork-main`: `sources/02-fork-main`, pinned at `a3ef1615c0f70b9aa24fc4dba91e64efce4b732a`, containing fork PRs 1 through 18.
- `pr19`: `sources/03-pr19`, pinned at `62e0af916e066d7f74f984f1514ae90a6f2a1aef`.
- `pr20`: `sources/04-pr20`, pinned at `42ef3f6c5f2a9bdb967340bef8d8857d2b4d834d`.

The source manifest records all four snapshots as clean at their stated commits. The extracted archive does not contain Git metadata, so branch ancestry and commit cleanliness cannot be re-executed locally. File-level comparisons are independently reproducible from the supplied trees, while historical lineage and PR state are taken from the frozen task record and manifest.

## 1. Executive assessment

### Overall verdict

The delegated handoff is **not safe to release or mechanically merge into current upstream**. It contains useful domain and validation work, but the executable runtime has two P0 failures and several lifecycle defects that defeat both legacy operation and the requested multi-repository model.

The two immediate release blockers are:

1. **Legacy single-repository mode dispatches no leaf work at all.** The CLI treats an absent project-set file as valid legacy mode, but constructs an empty repository inventory. The scheduler then requires every leaf to resolve through that inventory and releases every leaf as `MissingRepo` before workspace creation.
2. **Migration can activate multi-repository routing while preserving a hardcoded legacy clone hook.** An issue can resolve to repository B while `after_create` still clones repository A. The worker then runs in a valid Git checkout of the wrong repository, a silent workspace-integrity failure.

The suspected parent gap is confirmed. In `fork-main`, every issue with children is permanently released as `ParentDeferred`, even after all children are terminal. No parent worker, integration workspace, retained-child discovery, target-branch refresh, cross-repository verification, fix branch, pull request, review, merge, or subtree cleanup state exists. This also regresses the current upstream selection behavior, where a parent becomes dispatchable after all children are terminal.

The delegated implementation should therefore be treated as a **source of semantic components and test cases, not as a port-ready branch**. Retain the typed repository identity concepts, project-set validation, planner and converter propagation, path-containment work, and no-shell clone invocation. Redesign the configuration boundary, runtime instruction loading, repository provenance, checkout lifecycle, parent state machine, retention ownership, credential handling, migration, and operator schemas against current upstream.

### Direct answers to the user intent

- **Orchestrator-only configuration is not cleanly separated.** A second central file, `.opensymphony/project-set.yaml`, holds some orchestration data, while bootstrap `WORKFLOW.md` still owns hooks, workspace, agent, OpenHands, and prompt behavior. The runtime loads one bootstrap repository workflow and reuses it for all repositories.
- **A leaf can receive a checkout selected by `repo:<slug>`, but it does not reliably receive that checkout's instructions.** The worker's `cwd` can be the selected repository, while its prompt, launch profile, hooks, and agent settings come from the bootstrap repository's already-loaded workflow.
- **Exactly-one repository binding is represented, but resolution errors are collapsed.** Missing label, unknown slug, and multiple labels all become `None` and surface as the same release reason.
- **Parent execution is absent.** Parent issues are permanently deferred, and PR 20 explicitly tests that no parent workspace exists.
- **Child reuse is absent and current cleanup is destructive.** Terminal cleanup calls a production backend that deletes the directory directly, bypassing the workspace manager's retention policy and `before_remove` lifecycle.
- **Restart-safe provenance is absent.** Persisted manifests do not bind a workspace or conversation to repository identity, remote fingerprint, branch, commit, instruction hash, or lease generation.
- **Target-branch refresh and follow-up repair loops are absent.** The clone is shallow, existing `.git` state is accepted without validation, and there is no fetch/reset, branch, pull-request, review, or merge workflow for parent fixes.

### Release recommendation

Reject `fork-main` plus PR 19 and PR 20 as a release candidate. Start with a bounded semantic-port review against current upstream, fix the P0 regressions before enabling any migration, and implement the parent lifecycle only after durable repository identity, per-repository instructions, atomic checkout, provenance, and workspace leases are in place.

## 2. Scope and lineage reconstruction

### Frozen states reviewed

| State | Frozen revision | Recorded status | Material relationship |
|---|---|---|---|
| Current upstream | `d72bb0a409c006e20f2b0d766a0def218f94cd41` | v2.10.1 `develop` line | Current port target and behavioral baseline |
| Delegated fork main | `a3ef1615c0f70b9aa24fc4dba91e64efce4b732a` | Merged fork PRs 1 through 18 | Candidate implementation body |
| Fork PR 19 head | `62e0af916e066d7f74f984f1514ae90a6f2a1aef` | Recorded closed, not merged | One tracing-output change relative to `fork-main` |
| Fork PR 20 head | `42ef3f6c5f2a9bdb967340bef8d8857d2b4d834d` | Recorded closed, not merged | Live multi-repo E2E script change relative to `fork-main` |

The task records rewritten commit identities and no Git merge base, but an identical logical v1.9.2 tree at tree object `1b0d75f36fb4ff5a9b067eaef5c6d706bfbd16a6`. It also records the delegated `fork-main` delta from that logical base as 98 files changed, 16,336 insertions, and 392 deletions. Those numbers describe the fork's internal development delta, not a direct comparison to current upstream.

The correct review has two axes:

1. **Internal correctness relative to the logical v1.9.2 base.** On this axis, the project-set and repository-routing work introduces substantial functionality, but also breaks legacy dispatch and creates unsafe migration and lifecycle behavior.
2. **Semantic port to current upstream v2.10.1.** On this axis, a direct merge or broad cherry-pick would misclassify independent upstream evolution and overwrite newer scheduler, routing, OpenHands, gateway, code-intelligence, and Codex work.

### PR 19 and PR 20 relationship

A file-level comparison of the supplied snapshots establishes:

- `pr19` differs from `fork-main` only in `crates/opensymphony-cli/src/lib.rs`. Its `init_tracing` change adds `with_ansi(false)` and writes tracing output to standard error at `pr19/crates/opensymphony-cli/src/lib.rs:2389-2401`.
- `pr20` differs from `fork-main` only in `scripts/multirepo_live_linear_e2e.sh`.
- PR 20 is not a cumulative state containing PR 19. The two heads are independent additions to `fork-main`, and no supplied tree contains both changes.

The candidate handoff scope is therefore ambiguous. The frozen task states that both PRs were closed and unmerged on July 21, 2026, and that PR 20 lacked Stage 4 live OpenHands evidence. Their code should be assessed separately, not represented as a single validated branch.

### Evidentiary boundaries

All runtime and architecture conclusions below are grounded in the supplied source trees and tests. No moving branch head was substituted. The archive has no local Git object database, so commit-parent topology, review comments, and unprovided workflow runs were not independently reconstructed. The review does not infer successful live execution where the source record supplies no evidence.

## 3. Implemented capability map

| Capability | What is implemented | Primary evidence | Assessment |
|---|---|---|---|
| Typed project-set model | Project, repository, tracker, polling, agent, and inventory types, plus a resolved `slug -> RepoRef` inventory | `fork-main/crates/opensymphony-workflow/src/model.rs:419-492`, `fork-main/crates/opensymphony-workflow/src/model.rs:571-661`, `fork-main/crates/opensymphony-workflow/src/resolve.rs:1294-1506` | Useful semantic foundation. Identity and ownership need redesign. |
| Strict central/repo-local field boundary | In project-set mode, moved orchestration fields are rejected in the workflow front matter; hooks, workspace, OpenHands, and prompt remain in the workflow | `fork-main/crates/opensymphony-workflow/src/resolve.rs:57-118`, `fork-main/crates/opensymphony-workflow/src/resolve.rs:130-186` | Partial separation only. It leaves significant orchestrator policy in a repository-local artifact. |
| Secret indirection for tracker token | Linear API key can be loaded by environment-variable reference | `fork-main/crates/opensymphony-workflow/src/resolve.rs:1337-1390` | Correct direction for tracker credentials. Repository URL credentials are not given equivalent treatment. |
| Repository label resolver | Leaf issues can resolve one `repo:<slug>` label through the inventory; parents intentionally resolve no repository | `fork-main/crates/opensymphony-orchestrator/src/repo_resolver.rs:1-75` | Useful primitive, but its `Option` result erases distinct invalid states and breaks legacy mode. |
| Normalized execution repository | `NormalizedIssue` carries an `execution_repo_ref`, and the workspace descriptor receives it | `fork-main/crates/opensymphony-cli/src/orchestrator_run/backends.rs:1038-1080` | Useful runtime plumbing, but not durably persisted or checked during recovery. |
| Workspace path containment | The manager normalizes the root, rejects unsafe or symlinked paths, and contains hook working directories | `fork-main/crates/opensymphony-workspace/src/manager.rs:90-120`, `fork-main/crates/opensymphony-workspace/src/paths.rs:41-67`, `fork-main/crates/opensymphony-workspace/src/manager.rs:916-967` | Worth retaining and extending to all orchestration workspaces and reused child roots. |
| No-shell static clone command | `opensymphony workspace clone` builds a fixed `git` argument vector instead of interpolating the URL into `sh -c` | `fork-main/crates/opensymphony-cli/src/workspace_clone.rs:95-120`, `fork-main/crates/opensymphony-cli/src/workspace_clone.rs:243-267` | Worth retaining as a low-level primitive after adding atomicity, credentials, verification, and refresh. |
| Repository environment injection | The workspace manager injects selected repository URL, key, and branch into the `after_create` hook environment | `fork-main/crates/opensymphony-workspace/src/manager.rs:779-797`, `fork-main/crates/opensymphony-workspace/src/manager.rs:1231-1248` | Correct derivation mechanism, but arbitrary legacy shell hooks can ignore it. |
| Planner/task propagation | Task front matter, graph validation, generator metadata, and Linear conversion carry repository identity and enforce one managed repo label | `fork-main/crates/opensymphony-planning/src/graph_validate/frontmatter.rs:45-68`, `fork-main/crates/opensymphony-planning/src/graph_validate/manifest.rs:200-320`, `fork-main/crates/opensymphony-planning/src/generator/domain.rs:63-113`, `fork-main/.agents/skills/convert-tasks-to-linear/scripts/convert_tasks_to_linear.py:678-773`, `fork-main/.agents/skills/convert-tasks-to-linear/scripts/label_merge.py:89-163` | Substantial reusable work, subject to canonical-ID and error-code cleanup. |
| Repository facets in memory | Memory records and filters expose repository facets, with compatibility fallback to legacy paths | `fork-main/crates/opensymphony-memory/src/lib.rs:120-155`, `fork-main/crates/opensymphony-memory/src/lib.rs:652-680`, `fork-main/crates/opensymphony-memory/src/index.rs:432-503`, `fork-main/crates/opensymphony-memory/src/query.rs:815-870` | Useful indexing concept. Project-set/project filters are not enforced by the core matching helper. |
| Issue context artifact types | Workspace APIs define paths for repository workflow and agent instruction context | `fork-main/crates/opensymphony-workspace/src/models.rs:511-535`, `fork-main/crates/opensymphony-workspace/src/manager.rs:537-548` | Dormant abstraction. Production code does not call `write_issue_context`. |
| Init/update/doctor support | Fresh init can install a static clone hook; update can migrate existing repositories; doctor and tests cover parts of the configuration | `fork-main/crates/opensymphony-cli/src/init_repo.rs:1635-1665`, `fork-main/crates/opensymphony-cli/src/update_repo.rs:172-204`, `fork-main/crates/opensymphony-cli/src/project_set_migration.rs:317-485` | Broad surface area, but migration is default-on, non-transactional, and unsafe for recognized legacy hooks. |
| Parent dispatch gate | Parents with children are emitted as `ParentDeferred` before workspace creation | `fork-main/crates/opensymphony-orchestrator/src/selection.rs:54-70`, `fork-main/crates/opensymphony-orchestrator/src/scheduler.rs:662-691` | Explicitly implements absence of parent execution, contrary to target behavior. |
| Live E2E harness | PR 20 adds a multi-stage Linear/OpenHands script with bounded waits and opt-in live operations | `pr20/scripts/multirepo_live_linear_e2e.sh:1-110`, `pr20/scripts/multirepo_live_linear_e2e.sh:741-803` | Useful scenario inventory, but asserts the wrong parent contract and relies on impossible hook evidence. |

The capability map supports a selective-port strategy. The strongest portions are types, validation, propagation, containment, and fixed-argv process construction. The weakest portions are integration among those components and durable lifecycle semantics.

## 4. Actionable code-review findings

### P0-1: Legacy single-repository mode cannot dispatch any leaf issue

**Evidence.** The CLI documents an absent `.opensymphony/project-set.yaml` as preserving legacy flow and returns no project set at `fork-main/crates/opensymphony-cli/src/orchestrator_run/config.rs:243-269`. `SchedulerConfig::from_workflow` initializes an empty inventory at `fork-main/crates/opensymphony-orchestrator/src/scheduler.rs:45-91`, and the builder comment claims that an empty inventory preserves legacy behavior at `fork-main/crates/opensymphony-orchestrator/src/scheduler.rs:94-99`. The production caller passes the resolved inventory or `unwrap_or_default()` at `fork-main/crates/opensymphony-cli/src/orchestrator_run/mod.rs:237-253`. The resolver explicitly returns no repository when the inventory is empty or when no single known label exists at `fork-main/crates/opensymphony-orchestrator/src/repo_resolver.rs:33-57`. Every ready leaf is then gated before workspace creation and released as `MissingRepo` at `fork-main/crates/opensymphony-orchestrator/src/scheduler.rs:637-716`.

**Affected path.** `opensymphony run` without `.opensymphony/project-set.yaml` -> runtime config -> empty inventory -> issue normalization -> repository resolver -> leaf dispatch gate.

**Failure scenario and impact.** An existing single-repository installation upgrades, retains its ordinary unlabelled Linear issues, and runs the orchestrator. Every active leaf repeatedly becomes `Released(MissingRepo)`. No workspace is created and no worker starts. This is a complete service outage for the documented compatibility mode.

**Test gap.** Scheduler tests that reach dispatch supply a repository inventory and matching labels in their fixtures, for example `fork-main/crates/opensymphony-orchestrator/tests/scheduler.rs:27-66`. The CLI legacy test at `fork-main/crates/opensymphony-cli/tests/run.rs:434-492` only establishes that old front matter does not trigger a stale-field configuration error; it exits before proving dispatch, workspace creation, or worker launch.

**Smallest credible remediation.** Make the runtime mode explicit. In legacy mode, derive one canonical repository from `target_repo` and bind all leaves to it without requiring labels. In project-set mode, require strict exactly-one routing. Encode this as an enum such as `RepositoryRoutingMode::{LegacySingle(RepoRef), ProjectSet(Inventory)}`, not an empty map sentinel. Add an end-to-end scheduler test proving an unlabelled legacy leaf reaches `ensure_workspace` and worker start.

### P0-2: Migration can silently clone the wrong repository for a correctly routed issue

**Evidence.** The supplied upstream `WORKFLOW.md` uses a hardcoded clone hook for the OpenSymphony repository at `upstream/WORKFLOW.md:27-30`. Fresh fork init rewrites the template to the static `opensymphony workspace clone` hook at `fork-main/crates/opensymphony-cli/src/init_repo.rs:1635-1665`, and the fresh `fork-main/WORKFLOW.md:27-30` has that safe shape. Existing-repository migration, however, removes only the enumerated moved fields and serializes all other front matter, including arbitrary hooks, at `fork-main/crates/opensymphony-cli/src/project_set_migration.rs:227-305`. Its preservation test covers an already-static hook rather than the hardcoded legacy hook at `fork-main/crates/opensymphony-cli/src/project_set_migration.rs:738-795`. The workspace manager injects the routed `RepoRef` through environment variables at `fork-main/crates/opensymphony-workspace/src/manager.rs:779-797` and `fork-main/crates/opensymphony-workspace/src/manager.rs:1231-1248`, but a hardcoded shell command never reads those variables.

**Affected path.** `opensymphony update` on an existing repository -> project-set migration -> strict `repo:<slug>` resolution -> workspace `after_create` hook -> worker launch in resulting checkout.

**Failure scenario and impact.** The bootstrap repository is A. Migration adds inventory entries and issues are labelled so a leaf resolves to B. The preserved legacy hook still runs `git clone ...A .`. Because A is a valid Git repository, clone succeeds, the workspace is considered healthy, and the agent edits and may push A while the issue, logs, and scheduler say B. This is silent cross-repository data corruption and can produce a pull request against the wrong codebase.

**Test gap.** Fresh-init tests exercise the new static hook. Migration tests assert preservation of unrelated hooks, but do not migrate the hardcoded clone shape present in the supplied legacy workflow and then dispatch an issue to another repository. No runtime assertion compares the selected canonical repository to the checkout's actual remote.

**Smallest credible remediation.** Before enabling project-set routing, recognize known legacy clone-hook forms and replace them with a structured clone action or the fixed static command. Reject unknown repository-creating `after_create` hooks unless the operator explicitly acknowledges them. Independently verify the checkout after bootstrap by comparing a canonicalized `remote.origin.url` or provider repository ID to the expected repository. Treat mismatch as a hard workspace-integrity error before the agent starts.

### P1-1: Every parent issue is permanently deferred, so cross-repository integration cannot occur

**Evidence.** The ordinary selector blocks a parent only while any child is nonterminal at `fork-main/crates/opensymphony-orchestrator/src/selection.rs:12-26`. The fork then adds a second predicate that returns true for every issue with children and describes deep parent behavior as out of scope at `fork-main/crates/opensymphony-orchestrator/src/selection.rs:54-70`. Dispatch evaluates that gate before workspace creation and releases the issue as `ParentDeferred` at `fork-main/crates/opensymphony-orchestrator/src/scheduler.rs:662-691`. Current upstream has no permanent parent gate and explicitly tests that a parent is ready when all children are terminal at `upstream/crates/opensymphony-orchestrator/src/selection.rs:12-26` and `upstream/crates/opensymphony-orchestrator/src/selection.rs:168-181`.

**Affected path.** Tracker hierarchy -> selection -> issue normalization -> dispatch gate.

**Failure scenario and impact.** Multiple child pull requests merge across repositories. The parent becomes eligible under the ordinary child-completion rule, but the permanent gate releases it without a workspace or worker. Cross-repository integration, verification, repair, and final cleanup never happen. Operators may see a release event but no useful completion semantics.

**Test gap.** Fork tests encode permanent deferral as the expected result. PR 20 reinforces it by failing if a parent workspace exists at `pr20/scripts/multirepo_live_linear_e2e.sh:794-803`. No test represents the requested lifecycle after child merges.

**Smallest credible remediation.** Remove permanent deferral. Introduce a persisted parent integration state machine and a dedicated orchestration workspace that references retained child checkouts. Parent dispatch must be gated on child terminal status plus configured merge-completion evidence, not merely on child issue status.

### P1-2: Terminal cleanup bypasses retention policy and destroys inputs needed by ancestors

**Evidence.** Recovery deletes a recovered terminal workspace at `fork-main/crates/opensymphony-orchestrator/src/scheduler.rs:512-541`; tracker reconciliation requests terminal cleanup at `fork-main/crates/opensymphony-orchestrator/src/scheduler.rs:559-591`; and `release_issue` delegates cleanup to the backend at `fork-main/crates/opensymphony-orchestrator/src/scheduler.rs:970-1001`. The CLI configures the workspace manager with `remove_terminal_workspaces: false` at `fork-main/crates/opensymphony-cli/src/orchestrator_run/backends.rs:420-436`. That apparent retention setting is ineffective in production because `RuntimeWorkspaceBackend::cleanup_workspace` directly calls `fs::remove_dir_all` for terminal workspaces at `fork-main/crates/opensymphony-cli/src/orchestrator_run/backends.rs:598-616`. The manager's own cleanup path, including policy evaluation and `before_remove`, is implemented separately at `fork-main/crates/opensymphony-workspace/src/manager.rs:243-302`.

**Affected path.** Terminal tracker transition or restart recovery -> scheduler release -> runtime workspace backend cleanup.

**Failure scenario and impact.** A child finishes and its tracker state becomes terminal. The directory is recursively deleted before a parent can inspect or refresh it. The delete also bypasses `before_remove`, hook records, manager policy, and any future lease or reference-count check. On restart, recovered terminal children are similarly deleted before parent recovery could claim them.

**Test gap.** Unit coverage of workspace-manager cleanup does not cover the production backend's direct delete. Parent tests expect no parent reuse, so they never assert child retention through parent completion or restart.

**Smallest credible remediation.** Make the workspace manager the sole deletion authority. Replace the boolean terminal flag with a durable retention decision based on ownership leases. A terminal child should release its worker lease but retain the checkout while any ancestor integration lease exists. Final deletion must be idempotent, bottom-up, policy-controlled, and recorded through the manager lifecycle.

### P1-3: Ordinary update performs a default-on, non-transactional migration that can strand repositories

**Evidence.** `UpdateArgs` defaults both `migrate_only` and `skip_migration` to false at `fork-main/crates/opensymphony-cli/src/update_repo.rs:21-41`. Plain `opensymphony update` runs migration for a detected target repository unless explicitly skipped at `fork-main/crates/opensymphony-cli/src/update_repo.rs:172-204`. The comments call the operation atomic at `fork-main/crates/opensymphony-cli/src/update_repo.rs:198-201` and `fork-main/crates/opensymphony-cli/src/update_repo.rs:415-423`, but the migration selects a Git remote and can hard-fail when none or multiple are suitable at `fork-main/crates/opensymphony-cli/src/project_set_migration.rs:317-360`, then writes the project-set and workflow as separate filesystem updates without rollback at `fork-main/crates/opensymphony-cli/src/project_set_migration.rs:429-485`. Workflow front matter is round-tripped through `serde_yaml::Value` and `serde_yaml::to_string` when moved fields exist at `fork-main/crates/opensymphony-cli/src/project_set_migration.rs:227-305`, so comments and formatting are not transactionally preserved even though the prompt body is kept.

**Affected path.** Any ordinary CLI self-update executed inside an existing target repository.

**Failure scenario and impact.** An operator intends to update the binary or skills. The command unexpectedly attempts a configuration migration and may fail because of remote ambiguity, aborting the entire update. A failure after writing one of the two target files can leave project-set and workflow state inconsistent. A successful migration can also activate strict routing before existing Linear issues are labelled, immediately exposing the legacy-dispatch and wrong-hook failures.

**Test gap.** Tests emphasize idempotent reruns and individual error cases. They do not inject a write failure between the two output files, preserve YAML comments/order, prove rollback, or stage a fleet of unlabelled existing issues before enabling strict mode.

**Smallest credible remediation.** Make migration opt-in or a separately confirmed command. Add a preflight report that validates remotes, recognized hooks, issue-label readiness, credentials, and all output bytes before mutation. Commit outputs through a transactional directory/file replacement protocol with backups and rollback. Do not activate project-set mode until a readiness gate passes.

### P1-4: All implementation workers use the bootstrap repository's workflow and prompt

**Evidence.** The CLI resolves one `target_repo`, loads one workflow, and optionally loads one project-set at `fork-main/crates/opensymphony-cli/src/orchestrator_run/config.rs:88-154`. It passes a single `Arc<ResolvedWorkflow>` into the worker backend at `fork-main/crates/opensymphony-cli/src/orchestrator_run/mod.rs:237-242`. The backend stores that shared workflow and derives one runner configuration at `fork-main/crates/opensymphony-cli/src/orchestrator_run/backends.rs:619-651`, then clones the same workflow into every spawned task and passes it to `run_with_observer` at `fork-main/crates/opensymphony-cli/src/orchestrator_run/backends.rs:675-733`. OpenHands launch configuration is derived from that workflow while the process `cwd` points at the selected workspace at `fork-main/crates/opensymphony-openhands/src/session.rs:1946-1983`. The full prompt is rendered from the same workflow at `fork-main/crates/opensymphony-openhands/src/session.rs:2303-2315` and sent at `fork-main/crates/opensymphony-openhands/src/session.rs:1599-1617`. The loader reads the body from only the selected workflow file at `fork-main/crates/opensymphony-workflow/src/loader.rs:7-41`, and its template globals contain issue and attempt data, not a loaded target-repository instruction document, at `fork-main/crates/opensymphony-workflow/src/template.rs:5-22`.

**Affected path.** Run configuration -> worker backend construction -> OpenHands profile and prompt generation.

**Failure scenario and impact.** Repository A is the bootstrap/control repository and repository B is selected for a leaf. The checkout and `cwd` can be B, but the agent receives A's build commands, test procedure, style rules, prompt, hooks, model settings, and possibly review instructions. The agent can run invalid commands, violate B's conventions, or generate misleading results.

**Test gap.** Multi-repository tests check selected repository keys and workspace markers, not conflicting instruction markers. The defined `IssueContextArtifact` and `write_issue_context` APIs at `fork-main/crates/opensymphony-workspace/src/models.rs:511-535` and `fork-main/crates/opensymphony-workspace/src/manager.rs:537-548` have no production caller, so their existence does not prove instruction delivery.

**Smallest credible remediation.** Split the runtime envelope into central orchestration policy and repository-local implementation instructions. After checkout verification, load the configured instruction path from the selected repository, validate containment, record its path and content hash, and inject that text into the leaf prompt. Keep global model, scheduler, retry, tracker, and workspace policy in orchestrator-owned config. Make instruction precedence explicit and test it with contradictory markers in two repositories.

### P1-5: Recovery does not bind a workspace or conversation to repository provenance

**Evidence.** `AfterCreateBootstrapReceipt` records issue identity, workspace key/path, and timestamp but no repository at `fork-main/crates/opensymphony-workspace/src/models.rs:216-235`. `IssueManifest`, `RunManifest`, and `ConversationManifest` likewise omit canonical repository ID, remote fingerprint, branch, commit, and instruction hash at `fork-main/crates/opensymphony-workspace/src/models.rs:315-429`. Runtime recovery reconstructs a normalized issue with `execution_repo_ref: None` at `fork-main/crates/opensymphony-cli/src/orchestrator_run/backends.rs:1038-1066`. On the next active tracker poll, the scheduler re-normalizes current labels and attaches the recovered workspace directly at `fork-main/crates/opensymphony-orchestrator/src/scheduler.rs:526-531`. The workspace manager treats an existing issue state as already created and skips `after_create` at `fork-main/crates/opensymphony-workspace/src/manager.rs:105-171`.

**Affected path.** Process restart -> manifest scan -> scheduler recovery -> active issue re-normalization -> workspace and conversation reuse.

**Failure scenario and impact.** A leaf originally routed to A creates a workspace. Before restart, the issue is relabelled to B or the inventory remaps the slug. Recovery attaches the old A workspace to the now-B issue and skips cloning. The resumed OpenHands conversation can continue editing A while runtime state claims B. Similar ambiguity occurs after remote URL or target-branch changes.

**Test gap.** Recovery tests match issue and path identity but do not mutate repository labels, inventory mappings, remotes, branches, or instruction content between runs. There is no invariant to assert.

**Smallest credible remediation.** Persist immutable checkout provenance: canonical repository ID, credential-free remote fingerprint, configured target branch, checked-out commit, workspace generation, instruction path/hash, central policy hash, and conversation binding. On recovery, compare desired and persisted provenance before reattachment. A mismatch must enter an explicit superseded, repair, or quarantine transition, never silently reuse.

### P1-6: Clone is non-atomic and partial failure poisons retries

**Evidence.** The clone command targets `.` in the final workspace directory at `fork-main/crates/opensymphony-cli/src/workspace_clone.rs:95-120` and invokes `git clone` there at `fork-main/crates/opensymphony-cli/src/workspace_clone.rs:243-267`. Directory classification treats any `.git` entry as `AlreadyCloned`, an empty directory as cloneable, and any nonempty directory without `.git` as `Partial` at `fork-main/crates/opensymphony-cli/src/workspace_clone.rs:140-176`. `execute` returns permanent error for `Partial` and accepts `AlreadyCloned` without validation at `fork-main/crates/opensymphony-cli/src/workspace_clone.rs:211-233`.

**Affected path.** Workspace `after_create` -> static clone helper -> subsequent retry or restart.

**Failure scenario and impact.** A network failure, process kill, disk-full event, or authentication failure occurs after Git creates files but before a valid checkout is complete. The next attempt sees a nonempty non-Git directory and refuses to proceed forever, or sees a partial `.git` directory and incorrectly reports success. The issue cannot recover without manual deletion and may run against corrupt state.

**Test gap.** Classification unit tests cover static directory shapes, not failure injection during clone, atomic publication, concurrent ensure calls, corrupt `.git`, or restart between filesystem operations.

**Smallest credible remediation.** Clone into a unique sibling staging directory, verify repository identity and checked-out state, fsync/close as appropriate, then atomically rename into the final generation path. Persist a creation intent and generation token. On restart, safely remove or quarantine abandoned staging directories. Never infer validity from `.git` existence alone.

### P1-7: Gate evaluation can release running executions without aborting their workers

**Evidence.** The scheduler evaluates the parent and repository gates before checking whether an existing execution is claimed or running at `fork-main/crates/opensymphony-orchestrator/src/scheduler.rs:637-730`. Both gate paths call `release_issue` with no abort reason at `fork-main/crates/opensymphony-orchestrator/src/scheduler.rs:672-715`. `release_issue` aborts a current run only when an abort reason is present, then transitions the execution to `Released` at `fork-main/crates/opensymphony-orchestrator/src/scheduler.rs:970-1001`. Active released executions are reopened on a later tracker poll unless their last outcome is one of two terminal abort failures at `fork-main/crates/opensymphony-orchestrator/src/scheduler.rs:936-967`.

**Affected path.** Normal poll/reconciliation while a worker is active, especially after label or hierarchy changes.

**Failure scenario and impact.** A repository label is removed or changed while a worker is running. The next dispatch pass releases the execution as `MissingRepo` but leaves the worker task and index alive. Subsequent worker events target an execution in an incompatible state, while later polls can reopen and re-release it. This risks orphaned work, scheduler state errors, duplicate work, and misleading operator status.

**Test gap.** Gate tests cover pre-dispatch rejection, not mutation of routing or hierarchy during `Claimed` or `Running`, and not late worker events after a gate release.

**Smallest credible remediation.** Check existing execution state before pre-launch gates. Treat routing changes during an active run as a typed supersession event with an explicit abort and cleanup/retention policy. Make release reasons state-specific, remove the worker index atomically, and test all late-event races.

### P1-8: Literal repository URLs can leak credentials across process, log, error, and memory surfaces

**Evidence.** Project-set repository URLs are literal strings in `RepoEntry` at `fork-main/crates/opensymphony-workflow/src/model.rs:458-468`, while the tracker token has explicit environment indirection at `fork-main/crates/opensymphony-workflow/src/model.rs:470-480`. Resolution copies the URL into serializable `RepoRef` values at `fork-main/crates/opensymphony-workflow/src/resolve.rs:1417-1506` and `fork-main/crates/opensymphony-domain/src/repo.rs:11-20`. The clone helper prints the full URL on success at `fork-main/crates/opensymphony-cli/src/workspace_clone.rs:179-196` and includes the complete Git argument vector in an error at `fork-main/crates/opensymphony-cli/src/workspace_clone.rs:243-264`. The manager exports the URL in the hook environment at `fork-main/crates/opensymphony-workspace/src/manager.rs:1231-1248`. Memory requests include `executionRepoRef.url` at `fork-main/crates/opensymphony-openhands/src/session.rs:2962-2976`.

**Affected path.** Config parsing, clone subprocess invocation, hook environment, tracing/error output, and memory service requests.

**Failure scenario and impact.** An operator uses an HTTPS remote containing userinfo or a token. The token becomes visible in child process arguments, standard error, failure messages, and memory payloads, and can be retained by external process monitoring or log aggregation. Successful `after_create` output is not persisted in `run.json` by the current production backend, but that does not remove the other disclosure channels.

**Test gap.** No secret-canary test exercises URLs with userinfo and asserts redaction across subprocess, logs, errors, manifests, snapshots, and memory requests.

**Smallest credible remediation.** Store only a canonical, credential-free remote locator. Resolve credentials at execution time through SSH agent, Git credential helper, provider app installation token, or another typed credential provider. Reject URL userinfo, redact all remote text, and avoid serializing a clone URL into memory unless the receiving contract requires a safe repository identifier.

### P2-1: Existing checkouts are accepted without repository, branch, freshness, cleanliness, or integrity checks

**Evidence.** Any `.git` entry yields `AlreadyCloned` at `fork-main/crates/opensymphony-cli/src/workspace_clone.rs:148-176`, and `execute` immediately returns success for that state at `fork-main/crates/opensymphony-cli/src/workspace_clone.rs:211-225`. The initial clone is always shallow with `--depth 1` at `fork-main/crates/opensymphony-cli/src/workspace_clone.rs:107-113`. No code in that helper verifies `remote.origin.url`, `HEAD`, configured target branch, clean status, fetch result, merged child commit, or required history.

**Affected path.** Retry, restart, reused child workspace, and future parent integration.

**Failure scenario and impact.** A workspace is stale, points to the wrong remote, has a detached or dirty HEAD, or lacks the merged child commit. The helper declares success. A parent cannot reliably refresh or branch from the configured target, and an implementation worker can modify unintended state.

**Test gap.** Tests classify directory entries but do not initialize real repositories with wrong remotes, dirty state, branch drift, missing history, or force-pushed targets.

**Smallest credible remediation.** Introduce a checkout state machine with explicit verification and repair. For reuse: verify canonical remote, fetch target, deepen or unshallow when needed, refuse or quarantine dirty/unpushed state according to policy, reset a dedicated integration worktree to the configured remote target, and persist the resulting commit. Do not overload clone creation with refresh semantics.

### P2-2: Repository resolution collapses missing, unknown, and ambiguous bindings into one outcome

**Evidence.** The resolver returns `None` for no repo label, an unknown slug, multiple labels, and parent issues at `fork-main/crates/opensymphony-orchestrator/src/repo_resolver.rs:33-57`. The leaf gate turns all leaf `None` values into `ReleaseReason::MissingRepo` at `fork-main/crates/opensymphony-orchestrator/src/scheduler.rs:694-715`.

**Affected path.** Issue normalization, dispatch diagnostics, migration readiness, and operator remediation.

**Failure scenario and impact.** An operator cannot distinguish an unlabelled task from a typo, stale inventory entry, or conflicting labels. Automation cannot safely decide whether to wait, repair metadata, or alert. The single reason also conceals configuration drift.

**Test gap.** Resolver tests can distinguish inputs internally, but the public result and scheduler state intentionally discard the distinction.

**Smallest credible remediation.** Return a typed result such as `Resolved`, `NotRequiredForParent`, `MissingBinding`, `UnknownBinding`, and `MultipleBindings`. Persist it in scheduler and operator schemas with safe canonical repository IDs and actionable details.

### P2-3: Operator surfaces omit repository identity and misclassify gate releases as completion

**Evidence.** Domain release reasons document `MissingRepo` and `ParentDeferred` as human-visible state at `fork-main/crates/opensymphony-domain/src/runtime.rs:905-958`. The CLI snapshot mapper maps a released execution without a failure outcome to `Completed` and does not carry the release reason at `fork-main/crates/opensymphony-cli/src/orchestrator_run/snapshot.rs:90-111`; its output fields omit repository identity and release reason at `fork-main/crates/opensymphony-cli/src/orchestrator_run/snapshot.rs:126-179`. The control-plane issue schema also omits both at `fork-main/crates/opensymphony-domain/src/control_plane.rs:85-128`. The Rust and TypeScript gateway release-reason enums do not include the two fork reasons at `fork-main/crates/opensymphony-gateway-schema/src/run.rs:197-206` and `fork-main/packages/gateway-schema/src/run.ts:11-17`, and gateway run detail has no repository identity at `fork-main/crates/opensymphony-gateway-schema/src/run.rs:23-81`.

**Affected path.** Scheduler snapshot -> CLI/control plane -> gateway, TUI, web, and desktop consumers.

**Failure scenario and impact.** A leaf blocked for routing can appear completed, and a parent deferred forever can be indistinguishable from successful work. Operators cannot answer which repository, remote fingerprint, branch, or commit a worker actually used.

**Test gap.** Domain snapshots preserve normalized issue and runtime release reason, but downstream schema tests do not assert lossless projection of the new fields.

**Smallest credible remediation.** Extend one versioned operator contract with canonical repository ID, safe display name, target branch, checked-out commit, workspace generation, routing status, and full release reason. Map blocked/deferred states distinctly from success. Generate or contract-test Rust and TypeScript schema parity.

### P2-4: Project-set onboarding fragments one logical inventory across repository-local files

**Evidence.** Fresh project-set writing constructs a project set around the current target repository and emits one repository entry at `fork-main/crates/opensymphony-cli/src/project_set_writer.rs:333-396`. Migration uses a sentinel project-set slug and derives one repository from the selected local remote at `fork-main/crates/opensymphony-cli/src/project_set_migration.rs:317-385`. Runtime reads exactly one `.opensymphony/project-set.yaml` under the selected config root at `fork-main/crates/opensymphony-cli/src/orchestrator_run/config.rs:243-269`. `config.yaml` contains the target repository and runtime settings but no explicit project-set file selector or authoritative inventory reference at `fork-main/crates/opensymphony-cli/src/orchestrator_run/config.rs:17-58`.

**Affected path.** Init and migration across multiple repositories, followed by orchestration from one bootstrap repository.

**Failure scenario and impact.** Running init or migration in A and B produces independent local inventories, while the orchestrator reads only the file under the chosen config root. Operators can believe both repositories were onboarded while only one is routable. Copies can drift in URLs, branches, labels, and policy.

**Test gap.** Tests exercise one target repository at a time and do not construct a multi-repository fleet with conflicting local inventories.

**Smallest credible remediation.** Select one explicit orchestration root. Prefer making `config.yaml` the sole structured machine configuration, including project-set selection and repository inventory. If an external inventory remains, `config.yaml` must reference it explicitly and no duplicated values may be authoritative elsewhere.

### P2-5: Declared project-set and project memory filters are not enforced by the matching helper

**Evidence.** `MemoryScopeFilter` exposes project-set, project, repository, and related fields at `fork-main/crates/opensymphony-memory/src/lib.rs:652-680`. The core issue-scope matcher checks issue, milestone, area, and repository but does not evaluate project-set or project at `fork-main/crates/opensymphony-memory/src/query.rs:815-840`. Repository matching then uses the repository facet with a legacy-path fallback at `fork-main/crates/opensymphony-memory/src/query.rs:843-870`.

**Affected path.** Any memory query that expects project-set or project scoping, especially a shared index.

**Failure scenario and impact.** A caller supplies a project-set or project filter and receives records outside that requested scope because the fields are no-ops at the matching boundary. Repository facets alone do not establish multi-tenant access control.

**Test gap.** Repository facet tests do not demonstrate exclusion across two project sets or projects with overlapping repository or issue identifiers.

**Smallest credible remediation.** Add canonical project-set/project facets to indexed records and enforce every declared filter. Separate search relevance filters from authorization. Add cross-project-set negative tests and document whether the memory store is single-tenant or enforces tenant boundaries.

### P2-6: PR 20 tests the wrong parent contract and relies on hook evidence that production does not persist

**Evidence.** The harness requires that a parent workspace not exist at `pr20/scripts/multirepo_live_linear_e2e.sh:794-803`, directly contradicting the desired parent integration lifecycle. It expects `opensymphony workspace clone` standard error to appear under `run.json` hook records and greps for `key=<repo-key>` at `pr20/scripts/multirepo_live_linear_e2e.sh:741-791`. In production, the workspace backend calls manager `ensure` and discards the returned `after_create` hook result at `fork-main/crates/opensymphony-cli/src/orchestrator_run/backends.rs:546-563`. `RunManifest` starts with no hook records at `fork-main/crates/opensymphony-workspace/src/models.rs:357-390`, and `start_run` records run hooks rather than retroactively serializing `after_create` at `fork-main/crates/opensymphony-workspace/src/manager.rs:174-240`.

**Affected path.** Candidate validation and handoff evidence, not only test code.

**Failure scenario and impact.** A correct clone can fail the marker assertion because the expected text is never persisted there. Conversely, a harness that passes its parent check proves the absence of the requested behavior. The script can therefore block valid leaf behavior while endorsing a structurally incomplete parent model.

**Test gap.** This is the test gap. The frozen task also records no Stage 4 live OpenHands evidence, so no external run fills it.

**Smallest credible remediation.** Rewrite the test as a hermetic scenario. Assert checkout identity through a structured manifest field or a direct safe Git remote/commit probe, then require parent start after child merge, retained-child reuse, target refresh, a per-repository fix branch and review loop, and final subtree cleanup. Keep live execution as a separate bounded release gate.

### P2-7: Closed PR heads do not form one reproducible handoff state

**Evidence.** File-level comparison shows PR 19 changes only `crates/opensymphony-cli/src/lib.rs`, while PR 20 changes only `scripts/multirepo_live_linear_e2e.sh`. PR 20 does not include PR 19. The frozen task records both as closed and unmerged.

**Affected path.** Review, release, and any attempt to claim the harness was validated with the tracing change.

**Failure scenario and impact.** A reviewer may test PR 20 assuming it contains PR 19's stderr/no-ANSI behavior, or port both and accidentally combine states that were never reviewed or run together.

**Test gap.** There is no supplied canonical commit containing `fork-main` plus both heads.

**Smallest credible remediation.** Define one immutable candidate commit or patch series, with a machine-readable provenance manifest and validation results tied to that exact tree. Treat PR 19 as an optional logging patch requiring reevaluation against current upstream, and treat PR 20 as requirements input rather than passing evidence.

### P3-1: Repository identity and related validation abstractions need consolidation

**Evidence.** `RepoRef` documentation implies a potentially namespaced key while inventory construction uses a bare slug at `fork-main/crates/opensymphony-domain/src/repo.rs:11-20` and `fork-main/crates/opensymphony-workflow/src/resolve.rs:1488-1502`. `RepoEntry.path` is parsed and carried but dropped when building `RepoRef` and inventory at `fork-main/crates/opensymphony-workflow/src/model.rs:458-468` and `fork-main/crates/opensymphony-workflow/src/resolve.rs:1456-1502`. Repository label parsing is duplicated in orchestrator and memory code at `fork-main/crates/opensymphony-orchestrator/src/repo_resolver.rs:68-75` and `fork-main/crates/opensymphony-memory/src/index.rs:484-503`. Rust graph validation emits machine-readable snake-case codes, while the Python converter documents hyphenated error classes but emits prose-only diagnostics at `fork-main/crates/opensymphony-planning/src/graph_validate/manifest.rs:255-317` and `fork-main/.agents/skills/convert-tasks-to-linear/scripts/convert_tasks_to_linear.py:692-735`.

**Affected path.** Configuration validation, planner conversion, memory indexing, diagnostics, and future schema compatibility.

**Failure scenario and impact.** Bare slugs can collide across organizations or providers; dead `path` metadata implies behavior that does not exist; duplicated parsers and diagnostic-taxonomy drift produce inconsistent automation and operator guidance.

**Test gap.** Components test their own local representations, not one shared canonical repository identifier and one generated error taxonomy.

**Smallest credible remediation.** Define a single provider-qualified canonical repository ID and shared parser/value type. Generate label parsing and validation codes from one contract. Remove `RepoEntry.path` unless a concrete, contained local-mirror feature is implemented.

### P3-2: Dormant context APIs and misleading cleanup settings should be removed or wired end to end

**Evidence.** Issue-context artifact and write APIs exist at `fork-main/crates/opensymphony-workspace/src/models.rs:511-548`, but production search shows no caller that writes target-repository workflow or agent instruction files. The CLI sets `remove_terminal_workspaces: false` at `fork-main/crates/opensymphony-cli/src/orchestrator_run/backends.rs:420-436`, while the production backend deletes directly at `fork-main/crates/opensymphony-cli/src/orchestrator_run/backends.rs:598-616`.

**Affected path.** Developer understanding, configuration review, and future parent-retention implementation.

**Failure scenario and impact.** Reviewers may infer instruction isolation or retention from types and settings that have no effect in the production path.

**Test gap.** Local unit tests can pass without proving the public runtime caller graph uses the abstraction.

**Smallest credible remediation.** Either wire these APIs through one production path with contract tests, or delete them until the owning lifecycle exists. Remove configuration switches that are bypassed, and make the deletion authority structurally unique.

## 5. Context and configuration ownership matrix

### Recommended minimal boundary

Use **one orchestrator-owned structured configuration root** and **one repository-local instruction source per checkout**.

The smallest boundary that satisfies the runtime and migration requirements is:

1. **`config.yaml` is the sole orchestrator-owned machine configuration.** It selects the tracker/project set, contains or explicitly references the canonical repository inventory, and owns scheduler, workspace, model, review, target-branch, credential-provider, retention, and parent-integration policy.
2. **The selected repository checkout supplies implementation instructions.** Prefer an explicitly configured path per repository. A practical compatibility rule is: use configured `instructions.path`; otherwise use `AGENTS.md` when present; otherwise use the Markdown body of repo-local `WORKFLOW.md`. Do not read orchestration front matter from that file in strict multi-repository mode.
3. **Linear/task metadata owns task facts and one canonical leaf repository binding.** It must not contain clone URLs, credentials, workspace paths, model settings, or mutable branch state.
4. **A derived runtime envelope joins the sources.** It contains the normalized task, resolved canonical repository ID, safe remote locator, target branch and commit, workspace generation/path, central policy hash, repository instruction path/hash, and for parents a normalized child-checkout map. It is persisted for recovery and rendered deliberately into the worker prompt.
5. **A central orchestrator policy/prompt artifact owns cross-repository integration procedure.** It can be versioned with the orchestrator or referenced from `config.yaml`. It must not be copied into every repository merely for symmetry.

If `.opensymphony/project-set.yaml` is retained, it must be explicitly referenced by `config.yaml`, have a clearly declared owner, and contain no values duplicated as authoritative in `config.yaml` or repo-local instructions. Given the current runtime reads only one such file and init creates repository-local fragments, folding this data into `config.yaml` is the simpler and safer default.

### Concern/owner/source-of-truth matrix

| Concern | Owner and source of truth | Runtime derivation or injection | Unsafe duplication and stale-data risk |
|---|---|---|---|
| Tracker and project-set selection | Orchestrator `config.yaml` | Resolve tracker credentials through a credential reference; normalize selected project/project set into scheduler scope | Duplicating selection in repository-local workflow or task labels can dispatch the same issue under inconsistent scopes. |
| Repository inventory and canonical identity | Orchestrator `config.yaml`, or one explicitly referenced inventory file | Resolve a provider-qualified immutable ID, safe clone locator, aliases, default target branch, instruction path, and review profile into a typed inventory | Bare slugs collide; per-repository inventory copies drift; task metadata must carry only the canonical binding or a stable alias resolved centrally. |
| Workspace root and cleanup policy | Orchestrator `config.yaml` | Resolve one contained absolute root; derive generation paths; persist leases and cleanup tombstones | Repo-local workspace roots can escape or split the orchestrator's ownership domain. A bypassing backend makes policy decorative. |
| Scheduler, retry, and concurrency settings | Orchestrator `config.yaml` | Build one scheduler policy, with optional centrally declared per-repository quotas | Putting these in repo-local instructions makes operational policy depend on whichever checkout happened to be loaded first. |
| Harness, model, and runtime settings | Orchestrator `config.yaml` and named central profiles | Resolve a model/runtime profile per job using central policy; inject only the chosen nonsecret parameters | Repositories should not silently override provider endpoint, model budget, runtime permissions, or process supervision. |
| Target branch | Canonical repository entry in orchestrator config, with an optional task-level override allowed only by policy | Resolve before checkout; record requested branch, fetched remote branch, and exact commit in the runtime envelope | Inferring from the bootstrap repo or stale local checkout makes parent refresh nondeterministic. Task overrides can become unsafe unless explicitly allowed and audited. |
| Code-review provider and pull-request lifecycle | Central review profile in orchestrator config, referenced by repository entry | Resolve provider, repository installation/account, required checks, reviewers, merge method, and timeout into parent/fix state | Duplicated repo docs and central settings can disagree on whether review is required or what constitutes merged. Credentials must stay in the provider layer. |
| Implementation-agent task procedure | Central agent procedure plus task metadata | Render task facts and bounded central procedure into the prompt; keep it distinct from repository-specific commands | Copying a global procedure into every repo creates stale forks and makes migration impossible to reason about. |
| Repository-specific build, test, and style instructions | Repository-local `AGENTS.md` or configured instruction path in the verified checkout | Load after checkout validation; record path and content hash; inject only for jobs acting on that repository | Loading bootstrap instructions for all repos is unsafe. Copying instructions into central config becomes stale as code evolves. |
| Shared task metadata and leaf repo binding | Linear/task package | Normalize issue ID, hierarchy, blockers, description, acceptance criteria, and exactly one canonical repo binding for leaves | Clone URLs and mutable branch/commit facts in Linear become stale and may leak secrets. Multiple label parsers produce inconsistent routing. |
| Parent integration instructions | Central orchestrator policy artifact, optionally with per-repository extension points referenced by config | Parent receives the child-checkout map, merge evidence, central integration procedure, and each affected repo's local instructions only when operating there | A single repo-local workflow cannot define a cross-repo process. Duplicating the policy across repos leads to contradictory integration and cleanup behavior. |
| Checkout identity and provenance | Derived runtime context and durable manifests | Verify remote identity, target branch, commit, cleanliness, generation, and instruction hash; persist before worker attach | Re-deriving solely from current labels after restart can attach a workspace created for another repo. |
| Parent-child workspace ownership | Durable orchestrator state store | Acquire and release explicit leases/reference counts for each workspace generation; expose owners in diagnostics | Inferring ownership from tracker status deletes inputs too early and cannot survive restart or multiple ancestors. |
| Memory scope and provenance | Derived runtime context plus memory service policy | Store canonical project-set/project/repo IDs, issue ID, source commit, and access domain; enforce filter and authorization separately | Legacy path inference and ignored project filters can mix unrelated memories. Raw clone URLs should not be stored. |

### Precedence and loading contract

A deterministic load order should be specified and tested:

1. Parse `config.yaml` without evaluating repository-local files.
2. Resolve secrets through named credential references and construct the canonical repository inventory.
3. Normalize the task and resolve its repository binding with a typed result.
4. Acquire or create a verified checkout generation for that repository.
5. Load the repository instruction file from within that verified checkout and enforce path containment.
6. Construct and persist the runtime envelope, including policy and instruction hashes.
7. Render the implementation prompt from task metadata, central agent procedure, and the selected repository's instructions.
8. For parents, load the central integration policy and a child-checkout map. Load repo-local instructions separately each time the parent acts in a repository.

No repository-local instruction file should be able to redefine tracker credentials, project scope, workspace root, scheduler behavior, model endpoint, process permissions, cleanup policy, or repository inventory. Conversely, central config should not duplicate mutable build and style guidance that belongs with the code.

## 6. Parent integration and child-workspace gap analysis

### What the fork currently does

The fork's parent behavior stops at selection and release:

- A parent remains blocked while any child is nonterminal, inherited from the base selector at `fork-main/crates/opensymphony-orchestrator/src/selection.rs:12-26`.
- Once all children are terminal, the parent enters the ready list but is then unconditionally classified as deferred at `fork-main/crates/opensymphony-orchestrator/src/selection.rs:54-70`.
- The scheduler releases it as `ParentDeferred` before `ensure_workspace` or worker launch at `fork-main/crates/opensymphony-orchestrator/src/scheduler.rs:662-691`.
- The repository resolver always returns no repository for a parent at `fork-main/crates/opensymphony-orchestrator/src/repo_resolver.rs:8-14` and `fork-main/crates/opensymphony-orchestrator/src/repo_resolver.rs:33-57`.
- Terminal child cleanup can delete each workspace immediately through the direct backend path at `fork-main/crates/opensymphony-cli/src/orchestrator_run/backends.rs:598-616`.
- PR 20 treats parent non-creation as success at `pr20/scripts/multirepo_live_linear_e2e.sh:794-803`.

There is therefore no actual parent `cwd` or prompt to trace. No production object lists child workspaces for a parent, and no ownership relation prevents cleanup. The desired behavior is not partially implemented behind a flag; it is absent by design in this phase of the fork.

### Required semantics before a parent is dispatchable

A parent should not start merely because child tracker states are terminal. It needs a durable completion predicate for every relevant descendant edge:

- the child implementation run reached a terminal orchestrator outcome;
- all child-created pull requests required by policy are merged or explicitly waived;
- the merge target and resulting commit are known;
- the child workspace generation still exists or has an explicit recoverable replacement plan;
- no unresolved child retry, detached worker, cancel failure, review rejection, or merge conflict remains;
- the parent has acquired leases on every required child checkout generation.

This predicate should be calculated from persisted orchestration and provider state, with tracker status as one input. Linear completion alone cannot prove a pull request merged or identify the commit to refresh.

### Recommended parent workspace model

A cross-repository parent should not pretend to have one Git repository as its primary `cwd`. Give it a small **orchestration workspace** containing only durable metadata and generated views, for example:

```text
<workspace-root>/parents/<parent-id>/<generation>/
  parent-manifest.json
  child-checkouts.json
  integration-plan.md
  evidence/
```

`child-checkouts.json` should map canonical repository IDs to verified checkout handles rather than embed arbitrary shell paths in prompts. Each handle should include:

- workspace generation and contained path;
- child issue IDs and lease owners;
- canonical remote fingerprint;
- configured target branch;
- last fetched target commit;
- current branch and HEAD;
- cleanliness or quarantine state;
- instruction path and hash;
- relevant pull-request, review, and merge identifiers.

Commands that act on a repository must use an explicit validated `cwd` selected from this map. Cross-repository verification can run from the parent orchestration workspace only when it invokes repository commands with explicit handles. This avoids treating one child repository as the accidental control repository.

### Workspace leases and reference counts

Use explicit durable leases rather than a cleanup boolean. Suggested lease types are:

- `LeafWorker(issue, attempt)` while an implementation worker is active;
- `Review(issue, pull_request)` while review or merge follow-up still needs the checkout;
- `AncestorIntegration(parent, child)` while a parent or higher ancestor requires the child generation;
- `Repair(parent, repository, repair_attempt)` while a parent fix branch is active;
- `DiagnosticHold(operator, expiry)` for bounded troubleshooting.

A workspace generation is eligible for deletion only when it has no leases, no unresolved provider operation, no pending cleanup retry, and policy does not require retention. Lease acquisition, release, and cleanup intent must be transactional with the parent state transition. A count without owner identities is insufficient because restart recovery must reconstruct why a checkout is retained and which operation may release it.

For a hierarchy deeper than one level, an ancestor can lease the same checkout directly or inherit a durable subtree hold. A directed acyclic graph requires edge-specific ownership rather than assuming a strict tree. This is an open product decision, but the storage model should not preclude multiple parents.

### Proposed parent state machine

The following states are intentionally explicit so every external side effect has a resumable boundary:

1. **`WaitingForChildren`**: at least one child run is not terminal.
2. **`WaitingForChildMerges`**: child runs are terminal, but required pull requests are not yet merged or merge evidence is incomplete.
3. **`AcquiringChildLeases`**: acquire durable leases for all required checkout generations. Missing generations enter a controlled restoration path, not silent reclone.
4. **`RefreshingRepositories`**: for each canonical repository, verify remote identity, fetch the configured target branch, deepen history if required, resolve dirty-state policy, and reset a dedicated integration worktree to the merged target commit.
5. **`Integrating`**: run the central cross-repository integration procedure with the complete child map and recorded commits.
6. **`Fixing(repository_id)`**: create a fresh fix branch in one affected repository, load that repository's instructions, make and validate changes, and record commits.
7. **`AwaitingFixReview(repository_id, pull_request_id)`**: execute the configured review flow and respond to requested changes through new repair attempts.
8. **`AwaitingFixMerge(repository_id, pull_request_id)`**: wait for required checks and merge; record the resulting target commit.
9. **`RefreshingAfterFixes`**: refresh all affected repositories again after merges so final verification runs against merged targets, not feature branches.
10. **`FinalVerification`**: execute the central integration suite and repository-specific required checks against the exact recorded commits.
11. **`CleaningSubtree`**: release ancestor leases bottom-up, run lifecycle hooks through the manager, and record idempotent cleanup tombstones.
12. **`Completed`**, **`Blocked`**, or **`Failed`**: terminal parent outcome with structured reason and retained evidence.

Each state needs an idempotency key and a recorded result. Provider actions such as creating a pull request or requesting review must be searched by stable orchestration metadata before retrying after a crash, so restart cannot create duplicate branches or pull requests.

### Refresh and per-repository repair semantics

For each repository used by the parent:

1. Resolve the configured target branch from central repository inventory.
2. Verify the checkout belongs to the expected canonical repository.
3. Preserve or quarantine any dirty/unpushed child state according to explicit policy.
4. Fetch the remote target. If the clone is shallow, deepen or unshallow enough for required merge-base and review operations.
5. Use a dedicated integration worktree or resettable branch, rather than mutating the archived child feature branch in place.
6. Record the exact target commit after child merge.
7. Run repository-local required checks and central integration checks.
8. If a fix is needed, create a fresh branch named from parent ID and repair attempt, not from the child branch.
9. Load the affected repository's current instruction file and record its hash.
10. Commit and push through the configured credential provider, create a new pull request, complete review, wait for merge, then refresh again from target.

A parent may need several independent repair loops across repositories. Persist them as child entities of the parent state rather than a single mutable branch field.

### Restart and cleanup invariants

At minimum, durable state must include:

- parent state and transition version;
- task and hierarchy snapshot version;
- canonical repository ID and remote fingerprint for each checkout;
- workspace generation, contained path, and lease owners;
- target branch, fetched target commit, current branch, and HEAD;
- dirty/quarantine state and any preserved patch or bundle reference;
- central policy version/hash and repository instruction path/hash;
- child run, pull-request, review, check, merge, and resulting-commit identifiers;
- side-effect idempotency keys;
- cleanup intent, hook result, deletion result, and tombstone.

Recovery must reconcile persisted intent with filesystem, Git, tracker, and provider facts. It must never attach a checkout merely because its issue ID matches, and it must never delete a terminal child before reconstructing ancestor leases. Cleanup is complete only when every intended generation has either been deleted through the manager or retained under a documented policy hold.

### Gap summary

| Required behavior | Fork state | Required implementation |
|---|---|---|
| Parent starts after children complete and merge | Permanently deferred | Persisted eligibility predicate and parent state machine |
| Discover all child repositories and workspaces | No parent runtime object | Canonical child-checkout map built from durable child provenance |
| Reuse retained child workspaces | Terminal backend deletes them | Leases/reference ownership and manager-only cleanup |
| Refresh each target after child merge | No fetch/reset lifecycle | Verified fetch, target reset, history deepening, recorded commit |
| Run cross-repo verification | No parent prompt or workspace | Central integration policy and orchestration workspace |
| Repair one affected repository | No parent repair state | Fresh per-repo branch and repository-local instruction loading |
| Open PR and complete review | No provider lifecycle | Typed provider adapter, idempotent PR/review/merge states |
| Final cleanup of the subtree | Per-issue delete only | Bottom-up lease release and idempotent cleanup tombstones |
| Recover after restart | Manifests omit provenance | Durable parent, checkout, provider, lease, and cleanup state |

## 7. Current-upstream compatibility and port strategy

### Why a semantic port is required

Current upstream has evolved independently from the logical v1.9.2 base. The supplied upstream tree includes `opensymphony-code-intel` and `opensymphony-codex` crates absent from the fork, and its scheduler, workflow model, OpenHands integration, gateway, desktop, and review/merge handling have changed. The fork and upstream have no usable Git merge base according to the task record. A broad tree diff would therefore confuse upstream additions with fork deletions and invite regressions.

The port unit should be **behavioral contracts and small typed components**, not commits or whole files. For each retained capability, write an acceptance test against current upstream first, then implement the smallest semantic patch in the current owner module.

### Relevant current-upstream behavior

- Current upstream blocks a parent only while any child is nonterminal and tests readiness after all children are terminal at `upstream/crates/opensymphony-orchestrator/src/selection.rs:12-26` and `upstream/crates/opensymphony-orchestrator/src/selection.rs:168-181`. The fork's permanent deferral should not be ported.
- Current upstream's workflow model includes a newer runtime harness/model routing surface, not repository selection, at `upstream/crates/opensymphony-workflow/src/model.rs:47-68` and `upstream/crates/opensymphony-workflow/src/model.rs:250-327`. Repository-routing work must coexist with that contract rather than overload or replace it accidentally.
- Current upstream still loads one workflow from a configured target repository at `upstream/crates/opensymphony-cli/src/orchestrator_run/config.rs:95-121` and constructs a worker/scheduler around that workflow at `upstream/crates/opensymphony-cli/src/orchestrator_run/mod.rs:229-241`. The per-repository instruction problem remains a design task even when porting to upstream.
- Current upstream contains newer human-review and merging transitions in its scheduler around `upstream/crates/opensymphony-orchestrator/src/scheduler.rs:1093-1228` and dispatch logic around `upstream/crates/opensymphony-orchestrator/src/scheduler.rs:1427-1523`. Parent fix-PR work should extend these current state concepts rather than revive the fork's older scheduler wholesale.

### Reuse, rewrite, and discard map

| Component or idea | Disposition | Port guidance |
|---|---|---|
| Typed `RepoRef` and normalized issue repository field | **Reuse concept, redesign identity** | Introduce a provider-qualified canonical ID and safe display/remote fields in current domain types. Avoid bare-slug identity. |
| `repo:<slug>` label convention | **Reuse with compatibility layer** | Parse through one shared type. Resolve aliases to canonical IDs and emit typed invalid states. Preserve existing labels during migration where unambiguous. |
| Project-set validation and moved-field checks | **Reuse selectively** | Re-express in current upstream workflow/config model. Keep strict ownership validation but move authoritative orchestration policy into `config.yaml`. |
| Planner/task/converter propagation | **Reuse** | Port typed repository metadata, one-binding validation, and managed-label merge behavior. Unify validator codes and parser implementation. |
| Workspace path containment | **Reuse and extend** | Apply to orchestration workspaces, checkout generations, child maps, instruction paths, and cleanup operations. |
| Fixed-argv clone primitive | **Reuse low-level process construction** | Add credential provider, staging directory, verification, provenance, refresh, and structured output. Remove URL logging. |
| Repository environment variables for hooks | **Reuse only for non-identity hooks** | Do not depend on arbitrary shell hooks for core checkout creation. Core clone/refresh should be a typed workspace operation. |
| Repository memory facets | **Reuse after scope fix** | Add project-set/project facets and enforce them. Separate authorization from relevance filtering. |
| Permanent `ParentDeferred` gate | **Discard** | Preserve upstream parent readiness and add the persisted parent integration lifecycle. |
| Direct backend `remove_dir_all` | **Discard** | Make manager-only, lease-aware cleanup the sole deletion path. |
| Empty inventory as legacy sentinel | **Discard** | Use an explicit routing-mode enum with a real single-repository binding. |
| Any `.git` means valid checkout | **Discard** | Replace with verified checkout state and repair/quarantine policy. |
| Default-on migration in ordinary update | **Discard** | Use explicit preflight and transactional activation. |
| Single shared resolved workflow for all repos | **Rewrite** | Split central policy from repo-local instructions and construct a per-job runtime envelope. |
| Recovery by issue/path only | **Rewrite** | Persist repository, checkout, policy, instruction, lease, and provider provenance. |
| PR 19 tracing change | **Re-evaluate** | The no-ANSI stderr behavior is narrow and low-coupling, but compare it with current upstream logging and test needs before applying. |
| PR 20 script | **Rewrite as scenarios** | Retain useful setup ideas and bounded polling, but replace parent absence and `run.json` marker assertions with the target lifecycle contract. |

### Suggested semantic patch sequence against upstream

1. Add contract tests for explicit legacy and project-set routing modes without changing scheduler behavior.
2. Introduce canonical repository identity and typed resolution outcomes in current domain/workflow code.
3. Move inventory and orchestration policy ownership into current `config.yaml` resolution, adapting current routing types.
4. Add atomic verified checkout generations and durable provenance through the current workspace APIs.
5. Load and hash target-repository instructions after checkout; feed a per-job runtime envelope to the current OpenHands/Codex worker interface.
6. Extend current scheduler review/merge concepts with parent integration entities, leases, and restart-safe transitions.
7. Extend current gateway/control-plane schemas and front ends through versioned compatibility tests.
8. Port planner, converter, and memory facets after canonical identity is stable.

Avoid copying the fork's scheduler, workflow model, CLI config loader, or gateway schemas as whole files. Those are high-conflict ownership points where current upstream already has newer functionality.

### Handoff disposition

`fork-main` should become a reference implementation archive with a numbered extraction list. PR 19 should be represented as one optional tracing patch. PR 20 should be converted into a scenario specification and test-fixture backlog. A new current-upstream branch should contain only semantically reviewed changes, with every patch tied to an acceptance test and the exact source concept it replaces.

## 8. Proposed phased execution plan

### Phase 0: Bounded review and semantic port map

**Dependencies:** Frozen source roots, current upstream build/test baseline, named technical owners for config, scheduler, workspace, worker, provider, and schemas.

**Work:**

- Freeze one current-upstream target commit and document the logical-base relationship.
- Create a symbol-level extraction map from fork concepts to current owners.
- Reproduce the two P0 failures with focused tests against the fork and encode desired behavior as tests against current upstream.
- Decide canonical repository identity, configuration boundary, instruction-file precedence, and supported legacy mode.
- Inventory current upstream review/merge states and provider contracts before designing parent repair loops.
- Define versioned runtime-envelope, checkout-provenance, lease, and operator-schema contracts.

**Reuse:** Types and validation concepts, planner propagation rules, containment rules, fixed-argv process pattern.

**Rewrite:** All runtime integration and persistence.

**Acceptance gate:** A reviewed architecture decision record, traceable semantic patch map, passing upstream baseline, and executable failing tests for legacy dispatch, wrong-repo protection, per-repo instructions, and parent lifecycle. No migration or production routing is enabled.

### Phase 1: Correct immediate regressions and establish configuration ownership

**Dependencies:** Phase 0 decisions and canonical identity contract.

**Work:**

- Implement explicit `LegacySingle` and `ProjectSet` routing modes.
- In legacy mode, bind every leaf to the configured target repository without labels.
- In project-set mode, return typed missing, unknown, and multiple-binding results.
- Make `config.yaml` authoritative for inventory and orchestration policy, or explicitly reference one external inventory.
- Move model/runtime, scheduler, workspace, cleanup, target-branch, and review policy out of repo-local workflow front matter.
- Make migration a separate opt-in operation with a read-only preflight.
- Detect recognized hardcoded clone hooks and refuse activation until they are replaced.
- Add issue-label readiness checks and a reversible activation marker.

**Acceptance gate:** Existing single-repo installations dispatch unlabelled leaves; project-set mode blocks each invalid binding distinctly; ordinary update does not mutate project-set state; migration preflight makes no writes and identifies legacy hooks and unlabelled issues; no agent can start when checkout identity is unverified.

### Phase 2: Durable repository identity, atomic leaf checkout, and per-repository instructions

**Dependencies:** Stable config and canonical repository ID.

**Work:**

- Implement typed credential providers and reject credential-bearing repository URLs.
- Create checkout generations through staging directories and atomic publication.
- Verify canonical remote, target branch, HEAD, integrity, and cleanliness.
- Implement explicit refresh/deepen/reset operations separate from clone.
- Persist checkout provenance, policy hash, repository instruction path/hash, and conversation binding.
- Load selected repository instructions after checkout verification and construct a per-job runtime envelope.
- Extend operator snapshots with routing state and safe checkout provenance.
- Define supersession behavior for label, inventory, target-branch, and instruction changes during an active run.

**Acceptance gate:** Two repositories with deliberately conflicting instruction markers each receive the correct checkout and text. Clone crash injection recovers automatically. Wrong remote, corrupt `.git`, stale target, dirty state, and provenance drift are detected before worker attach. Secret canaries are absent from process-visible arguments where the credential method permits, logs, errors, manifests, snapshots, and memory requests.

### Phase 3: Parent integration and hierarchical workspace retention

**Dependencies:** Verified checkout generations, durable provenance, manager-only cleanup, and current upstream scheduler transition model.

**Work:**

- Add durable parent integration records and the explicit state machine from Section 6.
- Persist child run and merge completion evidence.
- Implement owner-identified leases for leaf, review, ancestor, repair, and diagnostic holds.
- Build the parent orchestration workspace and canonical child-checkout map.
- Acquire leases before terminal children become cleanup-eligible.
- Refresh every affected repository from its configured target branch after child merge.
- Run central cross-repository verification with explicit per-repository `cwd` handles.
- Add restart reconciliation at every state and idempotent cleanup tombstones.

**Acceptance gate:** In a two-repository hierarchy, children finish and merge, their checkout generations remain present, the parent starts exactly once, both repositories refresh to recorded merged target commits, restart at every transition preserves state, and cleanup cannot occur while an ancestor lease exists.

### Phase 4: Parent repair branches, pull requests, review, and merge loops

**Dependencies:** Parent state machine, provider adapter, target refresh, repository instruction loading.

**Work:**

- Add per-repository repair-attempt entities.
- Create deterministic fresh branches from the recorded target commit.
- Run the affected repository's current instructions and required checks.
- Commit and push using typed credentials.
- Create or find pull requests idempotently, execute configured review, handle requested changes, wait for checks, merge, and record resulting target commit.
- Support multiple sequential or parallel repository repairs under one parent.
- Refresh after every merge before final verification.

**Acceptance gate:** A forced integration defect in one repository produces one repair branch and one pull request, survives crash/restart without duplication, completes configured review and merge, refreshes the target, and leaves unrelated repositories untouched.

### Phase 5: Memory, diagnostics, and operator provenance

**Dependencies:** Stable canonical IDs and runtime-envelope schema.

**Work:**

- Index project-set, project, repository, issue, commit, and instruction provenance.
- Enforce every declared scope filter and define authorization separately.
- Extend gateway, control-plane, CLI, TUI, web, and desktop contracts with repository and parent-state fields.
- Surface blocked routing outcomes, lease owners, checkout verification, target commits, pull-request/review state, and cleanup state.
- Add structured redaction and audit bundles that exclude secrets.

**Acceptance gate:** Cross-project-set negative tests pass; all operator clients render blocked versus completed honestly; Rust and TypeScript schemas are contract-equivalent; a diagnostic bundle explains repository, branch, commit, instruction hash, and retention owner without revealing credentials.

### Phase 6: Validation hardening and isolated rollout

**Dependencies:** Phases 1 through 5 and a hermetic end-to-end suite.

**Work:**

- Replace PR 20 with deterministic local fixtures and provider fakes.
- Add systematic crash, process-kill, disk, network, provider, and tracker fault injection.
- Run a bounded live test in disposable repositories and a disposable tracker project.
- Capture structured evidence tied to the exact candidate commit and configuration hash.
- Roll out behind an explicit project-set activation flag with a tested rollback to legacy mode.

**Acceptance gate:** All unit, state-machine, fault-injection, hermetic E2E, schema, secret-canary, migration, and live release tests pass at one immutable commit. The live test demonstrates parent reuse, refresh, repair PR/review/merge, final verification, and subtree cleanup, with full teardown evidence.

## 9. Validation plan

### Unit and contract tests

1. **Routing modes**
   - Empty project-set state in `LegacySingle` dispatches an unlabelled leaf to the configured repository.
   - `ProjectSet` distinguishes no label, unknown alias, multiple labels, parent-not-applicable, and exactly-one resolved repository.
   - Canonical repository IDs remain stable across aliases, URL syntax variants, and display-name changes.
   - Bare-slug collisions across provider/organization boundaries fail configuration.

2. **Configuration ownership and precedence**
   - Repo-local instructions cannot override tracker, scheduler, workspace, model, credentials, target branch, review, or cleanup policy.
   - An explicitly configured instruction path must be contained within the verified checkout.
   - Precedence among configured path, `AGENTS.md`, and legacy `WORKFLOW.md` body is deterministic.
   - Environment indirection rejects missing and empty secret values.
   - If an external inventory is supported, `config.yaml` must reference it and duplicate authoritative fields fail validation.

3. **Migration**
   - Plain update performs no migration.
   - Preflight reports no remote, ambiguous remote, literal credential URL, hardcoded clone hook, unlabelled active issues, and conflicting inventory.
   - Recognized legacy hooks are rewritten only after explicit apply.
   - Unknown clone-like hooks block activation.
   - File comments, formatting, prompt body, permissions, and unrelated extensions are preserved according to a documented byte contract.
   - Failure between output writes rolls back to the exact prior state.
   - Repeated apply is idempotent and activation is reversible.

4. **Instruction isolation**
   - Repository A and B contain contradictory unique markers; an A leaf receives only A's repository-local section and a B leaf receives only B's.
   - Central policy appears identically in both prompts but repo-specific build/test commands do not cross over.
   - Instruction path/hash is persisted and a content change follows explicit supersession policy.

5. **Secrets and diagnostics**
   - Canary credentials in all supported providers are absent from logs, errors, traces, manifests, snapshots, memory payloads, Git command displays, and diagnostic bundles.
   - Userinfo in a repository URL is rejected.
   - Safe canonical repository identity remains visible for troubleshooting.

6. **Schema parity**
   - Rust domain, gateway schema, TypeScript schema, CLI projection, and UI decoders preserve every repository, routing, parent-state, lease, and release-reason field.
   - Blocked/deferred states never map to completed.

### Scheduler and state-machine tests

Use a deterministic fake tracker, clock, workspace backend, worker backend, and review provider.

- A ready legacy leaf follows `Unclaimed -> Claimed -> Running` and reaches workspace/worker calls.
- Invalid project-set bindings enter stable typed blocked states without reopen/release churn on every poll.
- Removing or changing a repo label during `Claimed` or `Running` aborts or supersedes once, removes the worker index atomically, and handles late events safely.
- A parent remains in `WaitingForChildren` while any child is active.
- Terminal child issue status without merged pull-request evidence moves the parent to `WaitingForChildMerges`.
- All required merges move it through lease acquisition and refresh exactly once.
- Lease acquisition failure leaves no partial unowned retention state.
- Parent integration can require no repair, one repair, several repairs in one repository, and repairs in multiple repositories.
- Review rejection, check failure, merge conflict, timeout, cancellation, and provider outage produce explicit resumable states.
- Final verification failure returns to an appropriate repair or blocked state without releasing leases prematurely.
- Cleanup proceeds bottom-up and can be retried after partial hook or filesystem failure.

### Git and filesystem fault injection

Use real temporary bare repositories and local transports, with faults injected at operation boundaries:

- process termination before clone starts, after files appear, after `.git` creation, after checkout, before manifest commit, and before atomic rename;
- destination missing, empty, nonempty, symlinked, wrong type, or concurrent creation;
- corrupt `.git`, wrong origin, renamed origin, URL-equivalent origin, dirty worktree, untracked files, unpushed commits, detached HEAD, branch drift, force-pushed target, and shallow history;
- fetch authentication failure, transient network failure, disk full, permission denial, and interrupted cleanup;
- target branch missing or changed in configuration;
- staging-directory recovery and quarantine;
- atomic publication and generation-token collision;
- checkout refresh after child merge, after parent repair merge, and after restart.

Each test must assert both final Git state and durable provenance, including canonical remote fingerprint, target commit, current branch, generation, and lease owners.

### Recovery matrix

Crash and restart at every side-effect boundary:

| Lifecycle area | Required restart assertions |
|---|---|
| Leaf checkout | No duplicate clone; abandoned staging state is removed or quarantined; wrong provenance never reattaches |
| Worker launch | Conversation binds to the same repo/generation/instruction hash; superseded context cannot resume silently |
| Child completion | Parent lease intent survives; terminal cleanup cannot race ahead of reconstructed ownership |
| Parent lease acquisition | Repeated acquisition is idempotent and owner identities remain exact |
| Target refresh | Recorded fetched commit matches filesystem HEAD; interrupted reset cannot be treated as verified |
| Repair branch | Existing deterministic branch is found or safely rejected; no duplicate branch side effect |
| Pull-request creation | Retry finds the existing PR by idempotency metadata; no duplicate PR |
| Review/check wait | Provider state is reconciled and monotonic transitions are preserved |
| Merge | Resulting target commit is discovered exactly once and refresh resumes |
| Final verification | Evidence is tied to exact commits and can be rerun without losing repair history |
| Cleanup | Leases, hooks, delete operations, and tombstones reconcile independently; already-deleted paths are success only with matching generation identity |

### Hermetic end-to-end suite

Build the release-gating E2E around:

- two or more local bare Git repositories with different instruction markers and test commands;
- a fake Linear-compatible tracker with parent/child hierarchy and controlled state transitions;
- a fake pull-request/review provider with configurable checks, requested changes, merge commits, and outages;
- a deterministic fake implementation agent that reads the rendered prompt, writes marker files, and can intentionally introduce or repair integration faults;
- isolated temporary workspace roots and dynamically allocated ports;
- no host-wide process enumeration or unscoped `kill` operations;
- bounded per-stage timeouts and deterministic teardown.

Required scenario:

1. Two leaves route to different repositories and clone the correct canonical remotes.
2. Each leaf receives only its own repository instructions and produces a pull request.
3. Child pull requests merge and child issues become terminal.
4. Child checkout generations remain because the parent holds leases.
5. The parent starts once, receives the complete child map, and refreshes both repositories to the merged target commits.
6. Cross-repository verification finds a deliberate bug in one repository.
7. The parent creates a fresh fix branch there, follows that repository's instructions, opens one pull request, completes a requested-change review loop, and merges.
8. The parent refreshes the affected repository and passes final verification against recorded target commits.
9. Parent completion releases all subtree leases, runs cleanup through the manager, and deletes only eligible generations.
10. Restart injection at every numbered step converges to the same final result without duplicate side effects.

The suite must assert structured provenance rather than scrape incidental log text. In particular, it must not rely on PR 20's assumption that `after_create` stderr appears in `run.json`.

### Live release gate

A live test remains valuable after the hermetic suite passes. It should use disposable GitHub repositories or equivalent provider resources, a disposable tracker project, bounded model and API budgets, explicit credential scopes, and an isolated workspace root. It must capture:

- exact application commit, config hash, and schema versions;
- tracker issue and hierarchy IDs;
- canonical repository IDs and safe remote fingerprints;
- child and parent workspace generations and lease transitions;
- target branch and commit before and after each merge;
- prompt instruction hashes without secret or full proprietary content;
- pull-request, review, check, merge, and cleanup evidence;
- teardown results for repositories, branches, tracker objects, processes, and files.

Live execution is a mandatory final release gate for multi-repository activation, not an opt-in smoke test that can be skipped while claiming lifecycle completion.

## 10. Open ambiguities and decisions required

| Decision | Why it must be resolved | Recommended default |
|---|---|---|
| Canonical repository identity | Bare slugs can collide and URLs change or contain credentials | Provider-qualified immutable ID, with aliases and a credential-free canonical remote fingerprint |
| `config.yaml` only versus external project-set file | Current local project-set files fragment inventory and have no explicit selector | Put structured orchestration config in `config.yaml`; support one referenced external inventory only for a demonstrated reuse need |
| Repository instruction artifact | Current runtime conflates workflow front matter and prompt body | Configured `instructions.path`; fallback to `AGENTS.md`, then legacy `WORKFLOW.md` body for migration compatibility |
| Legacy-mode support duration | Current code claims compatibility but breaks it; migration cannot be forced safely | Keep explicit legacy mode through a measured deprecation window with dispatch and rollback tests |
| Target-branch source and overrides | Parent refresh cannot be deterministic without one owner | Repository inventory default, with centrally authorized task override and persisted resolution |
| Pull-request/review provider ownership | Repo instructions alone cannot safely define credentials and merge policy | Central provider/review profiles referenced by repository entries |
| Parent completion predicate | Tracker terminal state does not prove merged code | Require persisted child run completion plus provider-confirmed required merges and checks |
| Hierarchy shape | Leases differ for a tree versus a DAG or shared descendant | Store owner-identified edge leases and support multiple ancestors even if initial UI presents a tree |
| Dirty or unpushed child state | Refresh can destroy useful work or integrate unreviewed code | Quarantine and block by default; permit explicit, audited preservation or patch export policy |
| Checkout reuse versus fresh integration worktree | Reusing a feature worktree can mutate evidence and conflict with refresh | Retain child checkout generation for evidence, create a verified integration worktree from the same repository object store where feasible |
| Credential mechanism | URL credentials leak and provider operations need scoped rotation | Typed provider credentials, SSH agent/Git helper/app token, never literal userinfo in inventory |
| Cleanup retention and audit evidence | Immediate delete loses parent inputs; indefinite retention leaks storage | Lease-based eligibility plus bounded post-completion evidence retention and explicit diagnostic holds |
| Memory authorization model | Repository facets are filters, not proof of access control | Treat memory as single-tenant until a separate enforced authorization boundary is implemented and tested |
| Instruction-change behavior mid-run | Recovery must not silently continue with materially different instructions | Persist hash; apply explicit continue, restart, or block policy based on change classification |
| Routing-label mutation mid-run | Current gate can release a live worker without abort | Typed supersession with atomic abort, provenance check, and operator-visible reason |
| Migration byte-preservation contract | Current YAML round-trip can reformat front matter | Define exact preservation requirements and use an AST/editor or generated central config rather than claiming byte identity |
| PR 19 disposition | Narrow logging change is independent of PR 20 and current upstream may already differ | Re-evaluate and port only if a current-upstream test requires it |
| PR 20 disposition | It asserts permanent parent absence and impossible evidence | Retain scenario ideas, discard pass/fail contract, and replace with hermetic lifecycle tests |
| Canonical handoff state | No supplied commit contains fork main plus both closed PR heads | Create a new immutable current-upstream candidate commit with provenance and results tied to that exact tree |

### Final decision

Proceed only with a **selective semantic port into current upstream**. Fix explicit routing modes and migration safety first, then establish verified leaf checkouts and per-repository instructions, then implement durable parent integration with leases and restart recovery. Do not enable multi-repository migration or claim parent support until the acceptance gates in Sections 8 and 9 pass on one immutable candidate revision.

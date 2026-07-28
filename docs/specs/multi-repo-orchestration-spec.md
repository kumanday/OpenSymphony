# OpenSymphony multi-repository orchestration specification

**Status:** Draft for implementation planning

**Target:** Current OpenSymphony `develop` line

**Reader:** An engineer preparing and executing the implementation plan

**Post-read action:** Decompose this specification into dependency-ordered implementation slices without reopening the core product model

## 1. Summary

OpenSymphony must orchestrate work across several repositories without treating any repository as the control repository for the whole run.

The design has four ownership boundaries:

1. One selected, orchestrator-owned `config.yaml` defines the orchestrator instance, Linear scope, repository inventory, operational policy, credentials, workspaces, review policy, and parent-integration policy.
2. Linear task metadata binds each terminal child task to exactly one execution repository.
3. Each verified repository checkout supplies its own implementation instructions.
4. Durable orchestrator state records the resolved task, checkout, instructions, leases, provider operations, and parent lifecycle so work can recover safely after restart.

A Linear project may be associated with several repositories. That association limits and validates which repositories its tasks may use; it does not select a default repository for those tasks.

A parent task has no primary repository. It waits for its children to complete and merge, retains their workspace generations through durable leases, and reuses their repository storage for integration work. It performs cross-repository verification from one parent execution root, creates follow-up repair branches and pull requests when required, then releases leases and cleans the subtree only after final validation and finalization.

The delegated fork is a reference source, not a merge candidate. Useful concepts and tests should be ported semantically onto current `develop`; its permanent parent deferral, unsafe migration, shared bootstrap instructions, unverified checkout reuse, and direct cleanup behavior must not be carried forward.

## 2. Goals

- Preserve existing single-repository behavior until an operator explicitly activates multi-repository mode.
- Define one authoritative orchestrator configuration per running instance.
- Let one orchestrator scope include multiple Linear projects and multiple repositories.
- Bind every terminal child task to exactly one repository through task metadata.
- Keep parents repository-neutral and derive their repository set from completed descendants.
- Support arbitrary repository topologies without encoding topology-specific repository roles in the orchestration model.
- Guarantee that a worker runs in a verified checkout of the repository selected for its task.
- Load implementation instructions from that verified checkout, not from the repository used to launch OpenSymphony.
- Retain child workspace generations for parent integration and higher ancestors.
- Refresh parent integration work to the exact merged target commits.
- Support restart-safe integration testing, follow-up repair branches, pull requests, reviews, merges, and final verification.
- Make all external side effects idempotent so restart cannot duplicate a branch, pull request, review request, merge, or cleanup.
- Expose enough sanitized provenance for an operator to explain what repository, commit, instructions, and lifecycle state a task is using.
- Provide a reversible migration and rollout path that does not compromise production work on `develop`.

## 3. Non-goals

- Merging or broadly cherry-picking the delegated fork.
- Making a Linear project select a default execution repository.
- Giving a multi-repository parent a pretend primary Git checkout.
- Storing clone URLs, credentials, workspace paths, or mutable commit state in Linear.
- Allowing repository-local instructions to redefine scheduler, model, credential, workspace, cleanup, or review policy.
- Treating memory repository facets as an authorization boundary.
- Providing hosted sandbox isolation. The initial deployment remains a trusted local environment and must not claim stronger isolation.
- Building a general command broker, background-process supervisor, or service-topology DSL in the first release.
- Supporting arbitrary workflow engines or non-hierarchical orchestration beyond the existing task graph.
- Adding a separate “repository set” abstraction when the repository inventory plus project associations already express the requirement.
- Supporting task-level target-branch overrides in the first release. Each repository's target branch comes from central configuration.

## 4. Terms

### Orchestrator instance

One running OpenSymphony control plane with one selected configuration file, state root, workspace root, scheduler, and active project set.

### Project set

A named orchestration scope containing one or more Linear projects. It determines which tasks the instance polls and which repository associations are valid. It does not route an individual task.

### Repository inventory

The central list of repositories known to the orchestrator. Each entry contains stable identity, safe remote information, target branch, instruction selection, credential reference, and review profile.

### Repository association

The list of repositories that tasks in a Linear project are allowed to reference. It is a validation boundary, not a default.

### Terminal child task

A task with no children in the task graph. Some source material calls this a “leaf.” This specification uses “terminal child” to emphasize that the rule belongs to task metadata, not project configuration.

### Execution-repository binding

The one repository selected in task metadata for a terminal child. The human-facing binding may use an alias; the runtime resolves and persists the canonical repository ID.

### Canonical repository ID

A stable, provider-qualified identity. For providers that expose an immutable repository ID, it combines the provider with that ID. Human names and `owner/repository` coordinates are aliases or locators, not the durable identity.

### Safe remote fingerprint

A credential-free normalized description of the expected remote, such as provider host plus repository coordinate and, when available, provider-native immutable ID. It is safe to persist and compare. It is not a secret or a clone credential.

### Checkout generation

One immutable identity for a workspace checkout created or adopted at a specific time. Replacing or rebuilding a checkout creates a new generation even when the issue and repository are unchanged.

### Runtime envelope

The persisted, per-run record that joins task metadata, central policy, canonical repository identity, checkout provenance, and repository instructions.

### Lease

A durable owner-identified hold that prevents a checkout generation from being deleted. A counter without owner identity is not sufficient.

### Parent orchestration workspace

A non-repository execution root that stores the parent manifest, child-checkout map, integration plan, evidence, and a contained directory of parent-owned integration checkouts. It is the parent's default `cwd`.

### Integration checkout

A verified worktree derived from retained child repository storage and reset to the configured target commit. It lets a parent reuse the child workspace's Git data without mutating the archived child feature branch or cloning the repository again.

### Checkout handle

An opaque, generation-bound reference to one verified checkout. The orchestrator resolves it to canonical repository identity, checkout generation, and contained path. A filesystem path is not a checkout handle.

### Harness execution scope

The default working directory and verified checkout map that the orchestrator authorizes for one worker. A harness adapter translates this scope into its native workspace or sandbox controls and records the effective containment it actually enforced.

### Hermetic test suite

A fully local test environment using temporary Git repositories and fake tracker, worker, and review services. It does not touch production repositories, issues, credentials, or shared ports.

## 5. Architectural invariants

The following are hard requirements:

- The orchestrator is the sole owner of scheduling and parent lifecycle state.
- A running terminal child has exactly one immutable canonical repository binding.
- A parent has no execution-repository binding.
- A parent repository set is derived from its descendant task bindings and recorded run evidence.
- A Linear project-to-repository association never substitutes for task binding.
- No worker starts before checkout identity is verified.
- The worker `cwd` for a terminal child equals its verified checkout path.
- A parent's default `cwd` is its orchestration workspace, not one child repository.
- Every orchestrator-owned repository operation during parent work names a verified checkout handle explicitly.
- Agent-native shell and file access is governed by the selected harness execution profile; checkout handles are not represented as a sandbox boundary.
- The runtime records the effective harness containment and never claims a stronger boundary than the harness enforced.
- Central policy is loaded before and independently of repository-local files.
- Repository-local instructions cannot override central operational policy.
- Active runs use a persisted config generation, repository binding, checkout generation, and instruction hash.
- No checkout with an active lease is deleted.
- Checkout creation, refresh, repair, review, merge, and cleanup are restart-safe and idempotent.
- Core checkout creation and verification are typed workspace operations, not arbitrary shell hooks.
- Cleanup goes through the workspace manager. No scheduler backend or worker may delete a workspace directly.
- Tracker terminal state alone does not prove that required code is merged.
- A parent completes only after final verification passes against recorded merged target commits.
- Public and diagnostic surfaces never expose credentials or credential-bearing remotes.

## 6. Configuration scope and selection

### 6.1 Physical scope

There is one selected `config.yaml` per orchestrator instance.

The default user installation uses:

```text
~/.opensymphony/config.yaml
```

The configuration may set distinct contained state and workspace roots beneath `~/.opensymphony/`. A production, staging, or experimental instance may use a different configuration file selected explicitly. Separate instances must not share a state root or workspace root.

The first implementation does not need nested named profiles inside one file. Separate configuration files selected with `--config` are simpler and avoid ambiguous precedence.

### 6.2 Selection order

Configuration selection is:

1. An explicit `--config <path>`.
2. The default user-level configuration.
3. During the compatibility window only, a repository-local `./config.yaml` when no user-level configuration exists and legacy mode is explicitly allowed.

Strict multi-repository mode never discovers configuration by walking parent directories and never changes behavior based on the current repository.

### 6.3 Repository-local configuration

In strict multi-repository mode:

- repository-local `config.yaml` is not an orchestration source;
- repository-local `WORKFLOW.md` front matter is not an orchestration source;
- repository-local files may supply implementation instructions only;
- duplicate authoritative repository inventories are configuration errors.

Legacy repository-local configuration remains accepted only through the explicit compatibility path until migration is complete.

## 7. Source-of-truth matrix

| Concern | Source of truth | Derived runtime use |
|---|---|---|
| Active project set | Central config | Determines tracker query scope |
| Linear credentials and endpoint | Central credential and tracker profiles | Resolved privately when polling |
| Linear projects | Central config | Namespaced task scope |
| Project-to-repository associations | Central config | Validates terminal child bindings |
| Repository inventory | Central config | Resolves stable identity, remote, branch, instructions, credentials, and review |
| Terminal child repository binding | Task package and Linear metadata | Resolves to one canonical repository ID |
| Parent repository set | Durable descendant run evidence | Builds the parent child-checkout map |
| Workspace and cleanup policy | Central config | Creates generations, leases, retention, and cleanup intent |
| Scheduler, retry, and concurrency | Central config | Produces one orchestrator-owned policy generation |
| Harness, model, and runtime profile | Central config | Produces one per-job launch profile |
| Harness execution scope | Orchestrator run evidence | Gives the selected adapter one authorized root and checkout map |
| Effective execution containment | Harness adapter receipt | Records whether the run was trusted-host or workspace-confined |
| Target branch | Central repository entry | Drives checkout creation, refresh, and merge verification |
| Review and merge policy | Central review profile referenced by repository | Drives provider reconciliation and completion |
| Repository build, test, and style instructions | Verified target checkout | Added to the run prompt with path and hash |
| Generic parent lifecycle | Versioned central policy | Drives eligibility, leases, repair, finalization, and cleanup |
| System integration instructions | Optional project-set central artifact | Supplies topology-specific commands and context without changing orchestration semantics |
| Change-specific integration acceptance | Parent task metadata | States what the parent must prove for this work item |
| Task status and hierarchy | Linear plus durable scheduler snapshot | Determines readiness but not merge truth |
| Pull-request, check, review, and merge facts | Provider plus durable receipts | Determines child merge and repair completion |
| Checkout identity and current Git state | Verified Git facts plus durable provenance | Determines safe attach, refresh, and recovery |
| Memory catalog, private records, and access policy | Per-instance memory service under the central state root | Serves authorized cross-repository retrieval and capture |
| Repository memory policy, public docs, and portable OKF | Verified target checkout | Registers repository-owned sources without making the repository a separate authority |
| Memory provenance and access grant | Runtime envelope | Adds scope refs and bounds each worker's accessible repositories and overlays |

## 8. Configuration model

The following is the normative conceptual shape for the new sections. Existing unrelated `control_plane`, memory, and harness configuration remains in the same central file and should be migrated into the same typed loader rather than duplicated.

```yaml
schema_version: 1

instance:
  id: local-default
  state_root: ~/.opensymphony/state/local-default

routing:
  mode: project_set
  active_project_set: opensymphony-suite

tracker_profiles:
  linear-main:
    provider: linear
    endpoint: https://api.linear.app/graphql
    credential: linear-main
    active_states: [Todo, In Progress, Rework]
    terminal_states: [Done, Canceled]

project_sets:
  opensymphony-suite:
    tracker_profile: linear-main
    integration_instructions: ./integration/opensymphony-suite.md
    projects:
      - opensymphony-core
      - opensymphony-clients

linear_projects:
  opensymphony-core:
    provider_project_id: "<linear-project-id>"
    repositories:
      - github:repository:<provider-id-for-core>
  opensymphony-clients:
    provider_project_id: "<linear-project-id>"
    repositories:
      - github:repository:<provider-id-for-desktop>
      - github:repository:<provider-id-for-web>

repositories:
  github:repository:<provider-id-for-core>:
    aliases: [opensymphony]
    remote:
      provider: github
      locator: kumanday/OpenSymphony
      clone: git@github.com:kumanday/OpenSymphony.git
    target_branch: develop
    credential: github-ssh
    review_profile: github-standard
    instructions:
      path: AGENTS.md

workspace:
  root: ~/.opensymphony/workspaces/local-default
  retain_failed: true
  cleanup_after_parent_finalization: true

scheduler:
  max_concurrent_tasks: 4
  retry:
    max_attempts: 3

integration:
  policy: builtin:multi-repo-v1
  use_shared_git_worktrees: true

memory:
  catalog_root: ~/.opensymphony/state/local-default/memory
  auto_capture: true
  auto_archive: false
  serve: true

review_profiles:
  github-standard:
    provider: github
    credential: github-app
    required_checks: true
    required_review: true
    merge_method: squash

credentials:
  linear-main:
    kind: environment
    variable: LINEAR_API_KEY
  github-ssh:
    kind: ssh-agent
  github-app:
    kind: environment
    variable: GITHUB_TOKEN

compatibility:
  allow_repo_local_config: false
```

The example uses placeholders, not literal provider IDs or secrets.

### 8.1 Required config validation

- `instance.id`, state root, and workspace root must identify one non-overlapping instance.
- The active project set, tracker profile, projects, repositories, credentials, and review profiles must all resolve.
- Repository aliases must be unique within the active project set.
- A repository associated with two projects keeps one canonical inventory entry.
- Project associations may be empty only when the project is intentionally tracker-only and cannot dispatch implementation tasks.
- Repository clone values containing userinfo, tokens, or passwords are rejected.
- Repository instruction paths must be relative and contained.
- Project-set integration instruction paths resolve relative to the selected central config, must not resolve inside an inventory checkout, and are read and hash-pinned before parent execution.
- Target branches must be nonempty and centrally owned.
- State and workspace roots must be absolute after expansion, contained within allowed operator-selected roots, and not symlink escapes.
- The memory catalog root must belong to this instance's state root and must not overlap any repository-local memory store.
- No field may have two authoritative definitions.
- Unknown fields fail validation in strict mode.
- Resolved secret values must not implement ordinary serialization or debug display.

### 8.2 Legacy single-repository mode

Legacy mode uses an explicit configuration variant:

```yaml
routing:
  mode: legacy_single
  repository: github:repository:<provider-id>
```

Every existing task, including tasks without repository labels, routes to that one repository. This mode does not use an empty repository inventory as a sentinel and does not require task migration before it can dispatch.

## 9. Project sets, Linear projects, and task binding

### 9.1 Project-set behavior

- An orchestrator instance activates one project set at a time.
- A project set contains one or more Linear projects.
- Each Linear project lists the repositories its tasks are allowed to use.
- A repository may be associated with several Linear projects.
- A Linear project may be associated with several repositories.
- Neither a project set nor a Linear project declares a default execution repository.

### 9.2 Task authoring contract

In strict multi-repository mode:

- Every terminal child task declares one repository alias in task-package metadata.
- Every parent task omits repository binding.
- The task-package validator resolves aliases against the repository associations of the task's Linear project.
- Conversion to Linear writes one managed repository binding.
- Unmanaged labels remain untouched.

The initial Linear representation remains the existing managed label form:

```text
repo:<alias>
```

The alias is human-facing. The orchestrator resolves and persists the canonical repository ID before claiming the task.

### 9.3 Typed binding outcomes

Binding resolution must return one of:

- `Resolved(repository_id)`
- `MissingBinding`
- `UnknownAlias(alias)`
- `MultipleBindings(aliases)`
- `RepositoryNotAllowedForProject(repository_id, project_id)`
- `ParentBindingNotAllowed`
- `ProjectOutsideActiveSet(project_id)`

These outcomes are stable blocked states, not “completed” releases. They remain visible until task metadata or configuration changes.

### 9.4 Binding immutability

Once a run is claimed:

- its canonical repository binding is immutable;
- its config and inventory generations are recorded;
- changing or removing its Linear binding does not silently move the running worker;
- a conflicting mutation creates a typed supersession event, requests a controlled worker stop, and requires a new run generation;
- late events from the superseded worker cannot mutate the new run.

### 9.5 Parent repository derivation

A parent's repository set is the union of valid descendant run bindings required by its current integration scope. It is:

- derived from durable run and merge evidence;
- grouped by canonical repository ID;
- not stored as a project default;
- not represented by adding several `repo:` labels to the parent;
- recomputed only through a versioned hierarchy reconciliation step.

### 9.6 Hierarchy reconciliation and freeze

- While a parent is waiting for children or child merges, hierarchy changes create a new hierarchy generation and eligibility is recomputed.
- The parent pins its required child edges before acquiring integration leases.
- Once lease acquisition or integration preparation begins, a hierarchy change does not silently add, remove, or replace work. The parent enters `Blocked(HierarchyChanged)` and requires explicit re-planning into a new parent run generation.
- Removed descendants retain their evidence and leases until the blocked generation is reconciled or abandoned safely.
- Late child events carry their hierarchy generation and cannot satisfy a newer parent generation accidentally.

## 10. Repository instruction boundary

### 10.1 Selection order

After checkout verification, OpenSymphony selects one repository instruction entrypoint:

1. The configured `repositories.<id>.instructions.path`.
2. Root `AGENTS.md`.
3. The Markdown body of root `WORKFLOW.md` for migration compatibility.
4. No repository-specific instructions, with an explicit diagnostic.

Strict multi-repository mode never reads orchestration front matter from repository-local `WORKFLOW.md`.

### 10.2 Loading rules

- The selected file must resolve inside the verified checkout.
- Symlink and traversal escapes are rejected.
- Content is read only after remote and checkout verification.
- Path, content hash, source commit, and loading result are persisted.
- The initial run pins the loaded content. Edits made by the worker do not silently rewrite its own active instructions.
- A parent repair attempt reloads instructions from the refreshed target commit and records a new hash.
- If native harness instruction discovery also applies nested `AGENTS.md` files, OpenSymphony records that capability and does not falsely claim the root entrypoint is the only instruction source.

### 10.3 Prompt composition

A terminal child's first prompt contains, in clearly separated sections:

1. bounded central execution procedure;
2. normalized task facts and acceptance criteria;
3. canonical repository display name, target branch, and safe checkout facts;
4. repository-local instructions;
5. permitted runtime capabilities.

Central policy wins on conflicts involving credentials, tracker scope, model, execution containment, workspace, cleanup, review requirements, or repository identity. Repository instructions own code-specific build, test, formatting, and contribution guidance.

A parent prompt contains, in separate sections:

1. the generic central parent lifecycle;
2. the optional hash-pinned project-set integration instructions;
3. the parent task's change-specific acceptance criteria;
4. the structured child-checkout map and harness execution scope;
5. repository-local instructions keyed by canonical repository ID.

The integration artifact may describe any topology or command set, but the domain model does not infer repository roles from it. Repository-local instructions apply only while the parent acts in that repository.

## 11. Checkout creation and verification

### 11.1 Typed checkout operation

Core repository creation is a workspace-manager operation:

1. Allocate a new checkout generation and staging directory.
2. Resolve credentials without embedding them in the remote URL.
3. Clone or materialize repository storage with fixed process arguments.
4. Check out the configured target branch.
5. Verify repository identity, remote fingerprint, branch, HEAD, integrity, and cleanliness.
6. Write the provenance manifest durably.
7. Publish the generation atomically.
8. Run non-identity lifecycle hooks only after verification.

An interrupted staging directory is never treated as a valid checkout.

### 11.2 Verification requirements

Before worker attach or resume, verify:

- the checkout is a Git worktree;
- the canonical remote matches the expected safe fingerprint;
- the configured target branch exists at the expected remote;
- current HEAD and branch match the recorded lifecycle phase;
- the checkout generation matches its manifest;
- the worktree's dirty/untracked state matches recorded policy;
- required history is available for the next operation;
- the instruction entrypoint belongs to the same verified commit.

An existing `.git` directory alone is not proof.

### 11.3 Provenance manifest

Every generation records:

- schema and generation version;
- issue and run IDs;
- canonical repository ID and resolved alias;
- safe remote fingerprint;
- target branch;
- fetched target commit;
- current branch and HEAD;
- shallow/full history state;
- cleanliness and quarantine state;
- config, inventory, and central policy hashes;
- repository instruction path and hash;
- worker conversation binding;
- lease owners;
- creation, refresh, and verification receipts;
- cleanup intent and tombstone.

Raw credentials and credential-bearing URLs are prohibited.

## 12. Workspace leases and cleanup

### 12.1 Lease types

The initial lease model includes:

- `LeafWorker(issue, run)` while a child worker is active;
- `Review(issue, pull_request)` while required review or merge work remains;
- `AncestorIntegration(parent, descendant)` while an ancestor needs a descendant generation;
- `Repair(parent, repository, attempt)` while a parent repair is active;
- `DiagnosticHold(operator, expiry)` for bounded investigation.

### 12.2 Lease acquisition

- Publishing a terminal child's checkout generation creates its leaf lease.
- If the task has active ancestors, ancestor leases are created before the checkout can become cleanup-eligible.
- Child terminal transition and leaf-lease release are transactional with confirmation that required ancestor and review leases exist.
- A deeper ancestor receives an owner-identified edge lease or durable subtree hold.
- Storage supports several lease owners even if the first product UI exposes only a tree.

### 12.3 Deletion eligibility

A generation may be deleted only when:

- it has no active leases;
- it has no unresolved provider operation;
- it has no pending repair or cleanup retry;
- its evidence retention policy permits deletion;
- its generation identity still matches the deletion intent.

Already missing paths count as successful cleanup only when the tombstone proves the same generation was intended.

### 12.4 Cleanup behavior

- Cleanup runs bottom-up through the workspace manager.
- `before_remove` runs once per generation intent and its receipt is persisted.
- Hook failure, permission failure, and filesystem failure create retryable cleanup states.
- Parent finalization releases its leases only after final verification and durable evidence are written.
- If a higher ancestor still owns a lease, the descendant remains.
- Failed or canceled workspaces follow central retention policy and never bypass lease checks.
- Parent-owned integration worktrees are removed through Git-aware workspace operations before the parent orchestration root is deleted.
- The parent orchestration root is cleanup-eligible only after its integration worktrees, process evidence, and child-checkout map have durable terminal receipts.

## 13. Parent integration model

### 13.1 Parent eligibility

A parent becomes integration-eligible only when every required child edge has:

- a terminal orchestrator run outcome;
- required pull requests merged or explicitly waived by policy;
- provider-confirmed merge evidence;
- resulting target commit recorded;
- no unresolved retry, worker, review, check, or merge failure;
- a retained checkout generation or an explicit controlled restoration state;
- all required ancestor leases acquired.

Linear terminal state is necessary only when policy requires it; it is never sufficient by itself.

### 13.2 Parent orchestration workspace

The parent receives a non-Git orchestration workspace:

```text
parents/<parent-key>/<generation>/
  parent-manifest.json
  child-checkouts.json
  integration-plan.md
  evidence/
  repositories/
    <checkout-handle>/
```

The parent root itself is not a repository. Every directory below `repositories/` is a verified parent-owned integration checkout, and no repository role is encoded in its handle or directory layout.

The child-checkout map groups descendants by canonical repository ID and records:

- descendant issue and run IDs;
- retained generation handles;
- lease owners;
- safe remote fingerprint;
- target branch and merged target commit;
- child branch and merge evidence;
- cleanliness or quarantine state;
- instruction path and hash;
- provider pull-request, review, check, and merge references.

Public APIs may expose a sanitized projection, not arbitrary local paths.

### 13.3 Reusing child workspaces

The parent must reuse retained child repository storage and provenance. It must not perform a fresh network clone merely because the task is a parent.

The default implementation:

1. Retains every child generation unchanged for evidence.
2. Selects one verified repository storage source per canonical repository.
3. Creates a parent-owned integration worktree below the parent execution root from that storage.
4. Fetches the configured target branch and deepens history if needed.
5. Resets the integration worktree to the recorded merged target commit.
6. Uses that integration worktree for implementation, verification, and repairs.

This satisfies reuse while avoiding mutation of a completed child's feature-branch evidence. If shared-object worktrees are unavailable, the run enters an explicit restoration state rather than silently recloning.

### 13.4 Multiple children in one repository

When several children target the same repository:

- every child generation remains leased for evidence;
- the parent creates one integration handle for the canonical repository at a target commit containing every provider-reported merge result required by the children;
- every required provider merge-result commit must be reachable from that target commit, including when squash or rebase merge replaced the child feature commits;
- conflicting or missing merges block integration with a typed reason.

### 13.5 Parent state machine

The durable parent states are:

1. `WaitingForChildren`
2. `WaitingForChildMerges`
3. `AcquiringChildLeases`
4. `PreparingIntegrationWorkspace`
5. `RefreshingRepositories`
6. `Integrating`
7. `Fixing(repository_id, repair_attempt)`
8. `AwaitingFixReview(repository_id, repair_attempt, pull_request_id)`
9. `AwaitingFixMerge(repository_id, repair_attempt, pull_request_id)`
10. `RefreshingAfterFixes`
11. `FinalVerification`
12. `Finalizing`
13. `CleaningSubtree`
14. `Completed`
15. `Blocked(reason)`
16. `Failed(reason)`
17. `Canceled(reason)`

`Blocked` is resumable after operator or external-system intervention. `Completed`, `Failed`, and `Canceled` are terminal, but failed and canceled outcomes may retain diagnostic leases according to policy.

Every transition records:

- previous and next state;
- state version;
- reason;
- idempotency key;
- input facts and their versions;
- side-effect intent;
- result receipt;
- retry classification.

### 13.6 Parent execution topology

The initial design uses one parent orchestration conversation:

- its default `cwd` is the parent orchestration workspace;
- it receives a verified map from checkout handles and canonical repository IDs to relative integration-checkout paths;
- it uses the harness's native shell and file tools for implementation and verification;
- orchestrator-owned checkout, lease, cleanup, and provider operations accept checkout handles and reject arbitrary paths;
- direct agent tool access is constrained only by the effective harness execution profile, not by prompt text or handle validation;
- the orchestrator authorizes the scope, the adapter translates it, the harness enforces what it supports, and the adapter records the effective containment as `trusted_host` or `workspace_confined`;
- the first release may use the existing trusted-host OpenHands and Codex profiles, but must not describe them as workspace isolation;
- no command broker is required for the first release;
- one durable parent controller owns all repair and provider side effects.

Per-repository child workers may be added later only if one parent state machine remains authoritative.

### 13.7 Integration command and process lifecycle

- The first release supports bounded foreground integration commands and checks, not unmanaged background services.
- A command may coordinate any number of processes or repositories, but it must own readiness, child-process teardown, and temporary resource cleanup before it exits.
- Each named verification attempt records its execution root or checkout handle, harness conversation, start and terminal state, timeout, exit result, bounded log artifact, and cleanup result.
- Cancellation requests the harness to stop the active turn and records whether stop was acknowledged. The runtime does not assume an unobserved process stopped.
- A restart marks any nonterminal verification attempt indeterminate, reconciles the harness when supported, performs configured cleanup, and reruns the bounded check from a verified baseline.
- Ports and other shared local resources must be unique per parent attempt when allocated, recorded without secrets, and released during teardown.
- A future generic process supervisor may extend this contract only when a demonstrated workflow needs durable long-running processes; it must not introduce topology-specific repository roles.

### 13.8 Higher-level parents

When a parent is itself a child of a higher task:

- its integration result and recorded commits become descendant evidence for the higher task;
- lower checkout leases are not released if the higher ancestor still owns them;
- cleanup remains bottom-up;
- no completed intermediate parent collapses several repositories into a false single-repository binding.

## 14. Refresh, repair, review, and merge lifecycle

### 14.1 Refresh

For every repository:

1. Verify canonical identity.
2. Resolve the central target branch.
3. Detect dirty, untracked, or unpushed state.
4. Quarantine and block on unexpected local state by default.
5. Fetch the target branch through the configured credential provider.
6. Deepen history as required.
7. Verify that every required provider merge-result commit is reachable from the selected target commit; never require replaced feature commits after squash or rebase merge.
8. Reset the integration worktree to the recorded target commit.
9. Persist the new branch, HEAD, and verification receipt.

### 14.2 Repair attempt

When integration detects a defect:

- create a fresh parent-owned branch from the verified target commit;
- name and tag it deterministically from parent and repair attempt identity;
- acquire a repair lease;
- load current instructions for the affected repository;
- make and validate the smallest required change;
- commit and push through typed credentials;
- create or find the provider pull request using stable idempotency metadata;
- run required checks and review;
- respond to requested changes within the same repair attempt history;
- merge only when central review policy is satisfied;
- record the resulting target commit;
- refresh the affected repository before final verification.

A parent may have several repair attempts in one repository or across several repositories. Each is a durable child entity, not a mutable field overwritten by the next attempt.

### 14.3 Provider truth and idempotency

- Provider state is authoritative for pull-request, check, review, and merge facts.
- Tracker state is not substituted for provider merge state.
- Create, review-request, merge, and close operations use idempotency keys or search-before-create metadata.
- Restart reconciles provider state before repeating an action.
- External closure, force-push, review rejection, merge conflict, or provider outage creates a typed resumable state.
- Provider credentials and merge policy come only from central profiles.

### 14.4 Completion

A parent may complete only when:

- every required repair pull request is merged;
- all repositories are refreshed to recorded post-merge target commits;
- central integration verification passes;
- repository-required checks pass;
- evidence is written durably;
- final tracker update has been attempted without making scheduler correctness depend on it;
- cleanup or retention intent is recorded.

## 15. Runtime envelope and durable recovery

### 15.1 Terminal child envelope

The envelope includes:

- normalized task and hierarchy snapshot;
- typed binding result and canonical repository ID;
- project set, Linear project, config, inventory, and policy generations;
- checkout generation and safe provenance;
- target branch and exact commits;
- repository instruction source and hash;
- worker harness, model, requested execution scope, effective containment, and conversation binding;
- provider and review profile IDs;
- active leases and cleanup intent.

### 15.2 Parent envelope

The parent envelope includes:

- parent state and transition history;
- versioned child-checkout map;
- pinned hierarchy generation and required child edges;
- child run and merge evidence;
- integration worktree handles and commits;
- central integration policy hash;
- project-set integration-instruction path and hash when configured;
- requested execution scope and effective harness containment;
- verification attempts, allocated resources, and teardown receipts;
- repair attempt entities;
- provider receipts and idempotency keys;
- final verification evidence;
- lease release and cleanup receipts.

### 15.3 Restart reconciliation

On startup, reconcile persisted intent with:

- current config availability, while preserving the run's pinned generation;
- tracker hierarchy and status;
- filesystem and workspace generations;
- Git identity, branch, commits, and cleanliness;
- worker conversations;
- nonterminal verification attempts and their process/resource cleanup evidence;
- provider pull requests, checks, reviews, and merges;
- leases and cleanup tombstones.

Recovery must either continue idempotently or enter a precise blocked/quarantined state. It must not:

- attach by issue ID alone;
- accept a mismatched remote;
- move an active run to a new repository binding;
- resume a conversation with different instructions silently;
- delete a terminal descendant before rebuilding ancestor leases;
- duplicate provider side effects.

## 16. Failure behavior

| Failure | Required behavior |
|---|---|
| Missing, unknown, or multiple task binding | Stable typed blocked state; no checkout |
| Binding changes during a run | Controlled supersession; no silent move |
| Repository removed from new config | Existing run stays pinned but blocks on operations requiring unavailable policy; new runs cannot bind |
| Wrong remote or provider identity | Quarantine generation; no worker attach |
| Interrupted clone | Remove or quarantine staging generation; retry with a new generation |
| Dirty or unpushed child state | Quarantine and block by default; never hard-reset silently |
| Missing retained checkout | Controlled restore decision with provenance; no unrecorded reclone |
| Instruction file changes | Active run remains pinned; new run or repair records the new hash |
| Worker disconnect or late event | Reconcile conversation and ignore superseded events |
| Hierarchy changes after parent scope freeze | Block the parent and require explicit re-planning into a new generation |
| Integration command times out or becomes indeterminate | Stop or reconcile the harness, clean attempt-owned resources, and rerun from a verified baseline |
| Tracker outage | Preserve scheduler state and retry polling |
| Provider outage | Preserve provider intent and retry without duplication |
| Review rejection or failed checks | Remain in repair/review state with actionable reason |
| Merge conflict or force-pushed target | Block, refresh evidence, and require a new repair decision |
| Cleanup hook failure | Record receipt and retry; do not release unrelated leases |
| Filesystem deletion failure | Keep cleanup intent and retry idempotently |
| Final verification failure | Return to integration/repair or block; do not release leases |

## 17. Migration and compatibility

### 17.1 Migration principles

- Ordinary update does not activate multi-repository mode.
- Migration is an explicit operator action.
- A read-only preflight always precedes apply.
- Activation is separate from file conversion.
- Repeated preflight and apply are idempotent.
- Failure leaves the previous runnable configuration intact.

### 17.2 Preflight

Preflight reports:

- current config selection behavior;
- repository remote and canonical identity;
- literal credentials or credential-bearing URLs;
- orchestrator-owned fields still in repo-local workflow front matter;
- recognized hardcoded clone hooks;
- unknown repository-creating hooks;
- current Linear projects and active tasks;
- missing, unknown, multiple, or disallowed repository labels;
- target-branch mismatches;
- duplicate aliases and inventory conflicts;
- state/workspace-root collisions;
- rollback feasibility.

Preflight performs no writes and no tracker mutations.

### 17.3 Apply

Apply:

- writes the central config and any generated migration report through staging and atomic replacement;
- preserves unrelated repository-local instruction content;
- removes or disables recognized hardcoded clone hooks after typed checkout creation is configured;
- blocks on unknown clone-like hooks unless the operator resolves them explicitly;
- preserves file permissions and documents any formatting change;
- creates a recoverable backup before rewriting legacy files;
- does not add repository labels to parent tasks;
- does not guess terminal child bindings when more than one repository is possible;
- records an activation marker only after validation succeeds.

### 17.4 Compatibility window

- Legacy single-repository mode remains dispatchable without labels.
- Legacy and strict modes are explicit variants, not inferred from an empty map.
- Strict mode refuses repository-local orchestration fields.
- Rollback returns the instance to the prior explicit mode and config generation.
- Rollback is blocked while active multi-repository runs have unresolved leases or provider operations.
- Deprecation of legacy mode requires measured usage, migration evidence, and a separate decision.

## 18. Provider, API, and operator surfaces

### 18.1 Provider adapter

The provider boundary must support:

- repository identity lookup;
- safe remote fingerprint verification;
- pull-request lookup and creation;
- check and review status;
- requested-change state;
- merge eligibility and merge;
- resulting target commit lookup;
- idempotency metadata;
- sanitized errors.

The first implementation may support GitHub only, but domain state must not encode GitHub-specific names where a general concept exists.

### 18.2 Control-plane schema

Run and issue snapshots add:

- routing mode and active project set;
- Linear project identity;
- typed binding status;
- canonical repository ID and safe display alias for terminal children;
- config and inventory generations;
- checkout generation and verification status;
- target branch and sanitized commit IDs;
- instruction source and hash;
- parent state and blocked reason;
- descendant repository summary;
- lease owners and cleanup status;
- repair attempts;
- pull-request, check, review, and merge status.

Rust and TypeScript schemas must round-trip the same states. Blocked or deferred work must never render as completed.

### 18.3 CLI, TUI, web, and desktop

All clients must be able to answer:

- Why is this task not running?
- Which repository and commit is this terminal child using?
- Which repositories and child generations is this parent integrating?
- What leases prevent cleanup?
- What repair pull request or review is pending?
- Did final verification run against merged target commits?
- What cleanup remains?

Public remote clients receive sanitized values. Exact local paths are limited to trusted local diagnostics.

### 18.4 Logs and diagnostic bundles

Structured events include stable semantic IDs, state transitions, and safe repository provenance. They exclude:

- raw credentials;
- credential-bearing URLs;
- secret environment values;
- full proprietary instruction content;
- unrestricted local filesystem paths in remote surfaces.

Diagnostic bundles include hashes and safe fingerprints so an operator can correlate facts without exposing secrets.

### 18.5 Memory

#### 18.5.1 Topology and ownership

- One memory catalog and MCP service belongs to one orchestrator instance and stores private runtime records under that instance's central state root.
- Normal workers use one injected MCP endpoint for that service rather than starting or directly reading an independent repository-scoped database.
- Repository-local memory policy, public documentation, and portable OKF bundles remain repository-owned sources. They are registered by canonical repository ID and verified commit; they do not become separate authorization authorities.
- Repo-neutral project, milestone, parent, and cross-repository records live in the central catalog and may carry zero, one, or several repository scope references.
- Existing `.opensymphony/memory` stores are imported or registered once with provenance. Migration must not leave old and new stores receiving concurrent authoritative writes.

#### 18.5.2 Scope and authorization

Each memory record stores `scope_refs` for the applicable instance, project set, Linear project, milestone, work item, canonical repository, code path, and area rather than requiring one repository foreign key.

Authorization claims and query filters are separate:

- a per-run token or equivalent harness credential declares the maximum accessible project set, projects, work items, repositories, visibility, and administrative capabilities;
- query filters may only narrow those claims;
- `all_accessible` means all records inside the grant and never widens the grant;
- repository facets are provenance, not proof of permission;
- raw clone URLs, credentials, unrestricted paths, and full private instruction bodies are not stored in queryable records.

A terminal child receives:

- its current project set, Linear project, work item, and execution repository;
- read access to persisted records and target-branch code snapshots for repositories associated with its Linear project;
- an explicit repository filter when it queries another authorized repository;
- live workspace-overlay access only for its own verified checkout.

A parent receives:

- repo-neutral records allowed by its pinned project and hierarchy scope;
- persisted repository records and target-branch code snapshots only for repositories in its verified descendant set;
- live overlays only for parent-owned integration checkouts in the active integration envelope.

Memory access does not grant filesystem or workspace access. A worker may retrieve authorized memory about another repository without receiving that repository's checkout.

#### 18.5.3 Code intelligence and overlays

- Persisted code-intelligence snapshots are keyed by canonical repository ID and exact commit, not by local path.
- The memory service resolves canonical IDs to registered target-branch snapshots centrally.
- A live overlay records its checkout generation, commit, dirtiness, and owning run.
- One terminal child never receives another child's dirty overlay.
- A parent never receives an overlay outside its pinned descendant integration envelope.
- Cross-repository code retrieval cites canonical repository ID, commit, path, symbol or artifact ID, and whether evidence came from a persisted snapshot or live overlay.

#### 18.5.4 Capture, documentation, and failure behavior

- Leaf capture records the work item, execution repository, commits, instruction hash, and source refs.
- Parent capture may record multiple repositories and the exact verified commits used for final integration.
- Documentation sync writes only to an explicitly owning repository and follows that repository's current instructions and review lifecycle. Repo-neutral or multi-repository records are not written into an arbitrary descendant repository.
- If the per-instance service is unavailable, the worker receives a visible degraded or blocked memory state according to central policy. It must not fall back silently to an unrelated repository-local store.
- Direct repository-local file or database access remains an offline administrative recovery path, not a normal worker path.
- The initial deployment is still single-tenant, but per-run grants and negative-scope tests are required so hosted enforcement can reuse the same contract.

## 19. Security requirements

- Reject credentials embedded in repository URLs.
- Resolve secrets through typed credential references.
- Prefer SSH agent, Git credential helper, or scoped app credentials over tokens in process arguments.
- Redact secrets from logs, errors, manifests, snapshots, memory requests, support bundles, hook output, and process-display strings.
- Use fixed argument vectors for Git and provider commands.
- Keep checkout, orchestration, staging, worktree, evidence, and cleanup paths inside configured roots.
- Reject symlink, traversal, Unicode/case collision, and generation-mismatch escapes.
- Validate checkout handles before every orchestrator-owned repository operation; never accept an arbitrary path where a handle is required.
- Treat prompts and checkout maps as instructions and provenance, not access-control enforcement.
- Treat repository-local instructions as codebase-owned input. They may guide implementation but cannot alter central trust-boundary settings.
- Keep requested harness execution profiles centrally selected and record the adapter's effective containment.
- Record credential profile IDs, never resolved secret values.
- Use least-privilege provider credentials for clone, push, review, and merge operations.
- Issue memory credentials with scope claims independent of query filters.
- Bound log, instruction, and diagnostic artifact sizes.
- Preserve unknown harness events as data without executing them as configuration.
- Document that trusted local mode still permits host filesystem and process access.

## 20. Rollback and production-safety strategy

- All work starts from current `develop` on new semantically named implementation branches.
- The delegated fork remains read-only reference material.
- Multi-repository routing is disabled by default until explicitly activated.
- Each implementation phase lands with legacy behavior green.
- Central config parsing and typed routing may land before activation, provided legacy mode remains unchanged.
- Parent integration cannot activate before verified checkout generations and leases exist.
- Repair automation cannot activate before provider idempotency and recovery tests pass.
- An operator can revert the active routing mode to `legacy_single` only when no multi-repository run has unresolved state.
- Rollback preserves durable evidence and never deletes workspaces merely because the feature was disabled.
- Schema changes are versioned and support reading the immediately previous durable-state version during rollout.
- A bounded disposable live run is required before enabling the feature on production Linear projects.

## 21. Acceptance criteria

### 21.1 Configuration and routing

- A default user-level configuration can run independently of the current directory.
- Two explicit instance configs do not share state or workspace roots.
- A Linear project can associate with two repositories without selecting a default.
- A repository can associate with two Linear projects without duplicated inventory.
- A terminal child with one valid binding resolves to one canonical repository.
- Missing, unknown, multiple, disallowed, parent, and out-of-scope bindings have distinct blocked states.
- A parent carries no execution-repository binding.
- Legacy single-repository mode dispatches unlabelled tasks.
- Repository-local orchestration fields fail strict-mode validation.
- A configured project-set integration artifact is loaded from the central config scope and hash-pinned without becoming repository-local orchestration state.
- No secret appears in resolved serializable config.

### 21.2 Terminal child execution

- Two repositories with contradictory instruction markers each receive only their own instructions.
- Each worker `cwd` is the checkout verified for its binding.
- A hardcoded legacy clone hook cannot create a checkout of the wrong repository.
- Clone failure cannot poison retry.
- Wrong remote, corrupt Git state, wrong branch, dirty state, and provenance mismatch block before attach.
- A restarted run reattaches only to the same repository, generation, policy, instruction hash, and conversation.
- Changing a running task's binding produces controlled supersession.

### 21.3 Leases and parent integration

- A child checkout remains present after child completion while any ancestor lease exists.
- Parents wait for provider-confirmed required merges, not only tracker completion.
- A parent spanning several repositories starts exactly once after eligibility.
- The parent has a metadata orchestration workspace and no primary repository.
- The parent receives a complete verified child-checkout map.
- Multiple children in one repository produce one integration handle at a commit containing all required merges.
- Integration worktrees reuse child repository storage and do not perform fresh network clones.
- Every integration worktree is contained below the parent execution root and refreshes to its recorded merged target commit.
- Squash- and rebase-merged children are verified through provider merge-result commit reachability, not feature-branch commit ancestry.
- Parent integration can run bounded checks in the parent root and any explicit repository worktree without repository-role assumptions.
- Orchestrator-owned operations reject paths or stale-generation handles outside the pinned checkout map.
- Trusted-host harness runs report `trusted_host` and never claim workspace confinement.
- A hierarchy mutation after scope freeze blocks for explicit re-planning without releasing required leases.
- A timed-out or indeterminate check is cleaned and rerun from a verified baseline without assuming its processes survived.
- Restart at every parent state converges without losing leases or duplicating work.

### 21.4 Repair and provider lifecycle

- A seeded integration defect creates one repair branch and one pull request in only the affected repository.
- The repair uses that repository's current instructions.
- Requested changes update the durable repair history.
- Crash after push, pull-request creation, review request, approval, or merge does not duplicate side effects.
- Final verification runs after refreshing all affected repositories from target.
- Provider outage, external closure, failed checks, review rejection, and merge conflict are visible resumable states.

### 21.5 Cleanup and recovery

- No active lease permits deletion.
- Parent completion releases leases bottom-up.
- Cleanup runs through manager hooks and records tombstones.
- Cleanup removes parent-owned integration worktrees with Git-aware operations before deleting the parent orchestration root.
- Crash during cleanup resumes only the remaining eligible deletions.
- Failed or canceled runs retain or delete work according to central policy.
- Higher-ancestor leases survive intermediate parent completion.

### 21.6 Observability and security

- Domain, gateway, CLI, TUI, web, and desktop agree on routing, parent, lease, provider, and cleanup states.
- Blocked states never render as completed.
- Operators can identify repository, target commit, instruction hash, lease owner, and pending provider step.
- Secret-canary tests find no secret in arguments where the credential mechanism permits, logs, errors, manifests, snapshots, memory, or support bundles.
- Memory filters for project set, project, and repository are enforced.
- A terminal child can query persisted memory for another repository associated with its Linear project but cannot access that repository's live overlay.
- A parent can query exactly its verified descendant repositories and integration overlays; an unrelated repository remains denied even with `all_accessible`.
- Memory and code-intelligence requests use canonical repository IDs rather than local paths.
- Restart preserves memory grants and registered source identity without reverting to a repository-scoped server.

### 21.7 Release validation

The release-gating hermetic scenario must:

1. Create at least three local bare repositories with different instructions and no encoded repository roles.
2. Create a fake Linear hierarchy with terminal children across those repositories and one parent.
3. Route each child to the correct repository.
4. Run and merge every child change through a fake review provider.
5. Retain every child generation.
6. Start the parent and refresh every integration worktree below one parent execution root.
7. Detect a deliberate cross-repository defect.
8. Create one repair pull request, complete a requested-change loop, and merge.
9. Prove cross-repository memory access follows persisted-snapshot, live-overlay, and negative-scope rules.
10. Refresh and pass final verification.
11. Remove integration worktrees, release leases, and clean the entire parent subtree.
12. Inject restart at every numbered boundary and reach the same result without duplicate side effects.

After the hermetic suite passes, a disposable live test must demonstrate the same lifecycle with bounded credentials, budgets, timeouts, unique resources, and complete teardown evidence.

## 22. Phased implementation plan

### Phase 0: Contract and semantic port map

- Freeze the current `develop` baseline.
- Map every useful fork concept and test to current owners.
- Lock config names, canonical repository identity serialization, task-binding parser, and durable state versions.
- Lock the generic harness execution-scope, hierarchy-generation, and memory scope-reference contracts.
- Categorize fork material as reuse, rewrite, discard, or superseded.
- Write failing acceptance tests for legacy dispatch, typed binding, wrong-repository prevention, instruction isolation, memory scope isolation, and parent lease retention.

**Gate:** No implementation slice depends on permanent parent deferral, empty-inventory routing, direct cleanup, or a run-wide bootstrap workflow.

### Phase 1: Central configuration, explicit modes, and safe migration

- Add the central config model and default selection.
- Add project sets, Linear projects, repository inventory, aliases, credentials, review profiles, and explicit routing modes.
- Add optional hash-pinned project-set integration instruction artifacts without introducing repository-role fields.
- Port task-package and Linear binding propagation.
- Implement typed binding outcomes and stable blocked states.
- Build migration preflight, atomic apply, activation, and rollback.

**Gate:** Legacy dispatch remains green; strict routing is distinct and observable; migration cannot activate an invalid checkout path or hardcoded clone hook.

### Phase 2: Verified terminal-child checkout and instructions

- Add canonical repository identity and safe remote fingerprints.
- Implement typed atomic checkout generations.
- Persist complete provenance and runtime envelopes.
- Load and hash target-repository instructions after verification.
- Build per-job prompts for current OpenHands and Codex harness routes.
- Add the per-instance memory catalog/service, register or migrate repository-local sources, enforce filters and per-run grants, and expose only the bound leaf overlay.
- If strict multi-repository memory is not ready at activation, disable it with a visible diagnostic rather than falling back to the current single-root behavior.
- Make cleanup manager-owned.

**Gate:** Multi-repository instruction isolation, wrong-remote, memory negative-scope, cross-repository persisted-memory, live-overlay isolation, crash, secret-canary, and conversation-resume tests pass.

### Phase 3: Leases and parent integration without repairs

- Add owner-identified leases and hierarchy reconciliation.
- Add parent execution roots, contained integration worktrees, child-checkout maps, and harness execution-scope receipts.
- Add the durable parent state machine through integration and final verification.
- Add shared-object integration worktrees, provider merge-result reachability, target refresh, and topology-neutral multi-repository verification.
- Add parent memory grants and integration overlays.
- Add bounded foreground verification receipts, indeterminate-attempt recovery, and restart-safe resource cleanup.
- Add hierarchy freeze, restart, Git-aware worktree removal, and bottom-up cleanup behavior.

**Gate:** A parent spanning at least three repositories completes without repairs across restart injection, reports effective harness containment honestly, deletes no leased checkout, and preserves registered memory provenance.

### Phase 4: Repair branches, review, and merge

- Add durable repair-attempt entities.
- Extend current provider/review abstractions with idempotent create, reconcile, and merge operations.
- Add requested-change loops, post-merge refresh, and final verification.

**Gate:** The seeded one-repository repair scenario completes across crash injection without duplicate branches or pull requests.

### Phase 5: Operator schemas, clients, and memory surfaces

- Extend domain and gateway schemas.
- Update CLI, TUI, web, and desktop projections.
- Add sanitized diagnostics and support bundles.
- Expose memory scope, source freshness, overlay provenance, and effective containment without widening access.

**Gate:** Contract parity, negative-scope, and secret-redaction tests pass across every client.

### Phase 6: Validation hardening and isolated rollout

- Add systematic filesystem, Git, tracker, provider, worker, and disk/network fault injection.
- Run the hermetic lifecycle suite.
- Run a bounded disposable live test.
- Capture evidence at one immutable candidate commit and config hash.
- Activate on one non-production project set, verify rollback, then expand deliberately.

**Gate:** All acceptance criteria pass on one immutable current-`develop` candidate. Production activation remains explicit.

## 23. Decision log

### 23.1 Settled decisions

| Decision | Outcome |
|---|---|
| Implementation base | Current OpenSymphony `develop`; semantic port only |
| Fork disposition | Reference concepts and tests; no direct merge |
| Config ownership | One selected orchestrator-owned config per instance |
| Default config location | `~/.opensymphony/config.yaml` |
| Independent environments | Separate explicitly selected config files and non-overlapping roots |
| Project-to-repository relationship | Allowed association only; no default repository |
| Terminal child binding | Exactly one binding in task metadata |
| Parent binding | None; derive repository set from descendants |
| Separate repo-set concept | Omit until a demonstrated need exceeds project associations plus inventory |
| Repository-local config | Instructions only in strict mode |
| Parent workspace | Non-Git orchestration workspace plus explicit repository handles |
| Integration-checkout placement | Contained below one parent execution root; no repository roles encoded |
| Child reuse | Retain child generations and create integration worktrees from their repository storage |
| Cleanup | Durable owner-identified leases and manager-only deletion |
| Merge truth | Provider merge-result commit reachability plus durable receipts, not tracker status or feature-branch ancestry |
| Parent command execution | Harness-native tools; handle validation applies to orchestrator-owned operations |
| Execution containment | Orchestrator authorizes scope, harness adapter enforces what it supports, runtime records the effective boundary |
| First-release command broker | Omitted; add only for demonstrated hosted enforcement or uniform auditing needs |
| First-release integration processes | Bounded foreground checks that own readiness and teardown; no unmanaged background services |
| Hierarchy mutation | Reconcile while waiting; block for explicit re-planning after parent scope freeze |
| Memory topology | One per-instance catalog and MCP service with repository-owned registered sources |
| Memory cross-repository access | Persisted authorized repository memory; live overlays only for the bound leaf or active parent integration set |
| Activation | Explicit and reversible; legacy behavior preserved |

### 23.2 Defaults that may be changed before planning

Implementation planning may proceed with these defaults unless the product owner changes them:

| Choice | Default | Consequence of changing it |
|---|---|---|
| Linear binding representation | Existing `repo:<alias>` managed label | Structured metadata requires planner, converter, migration, and resolver redesign |
| Canonical identity encoding | Provider plus provider-native immutable repository ID | Human coordinates alone weaken rename/transfer safety |
| Config profiles | Separate files selected with `--config` | Nested profiles add precedence and migration work |
| Instruction precedence | Configured path, root `AGENTS.md`, legacy `WORKFLOW.md` body | Different precedence changes migration and prompt tests |
| Parent execution topology | One parent conversation and one authoritative controller | Per-repo controllers require new coordination and side-effect ownership |
| Integration checkout | Shared-object Git worktree; retain child feature checkout | In-place mutation weakens evidence; fresh clone violates reuse |
| System integration instructions | One optional project-set artifact selected by central config | Task-only or repository-local placement loses stable cross-repository context |
| Leaf persisted-memory scope | Repositories associated with its Linear project | Project-set-wide access increases unrelated context and future hosted exposure |
| Initial provider support | GitHub first behind provider-neutral domain states | Supporting several providers expands Phase 4 and live-test scope |
| Hierarchy shape | Existing tree UI; lease storage supports multiple ancestor owners | Tree-only storage makes later DAG/shared-descendant support a migration |
| Failed-workspace policy | Retain under an explicit diagnostic lease; operator releases it | Immediate deletion reduces evidence; indefinite implicit retention leaks disk |

### 23.3 Values to select during implementation planning

The architecture is complete without these numeric/operator choices, but the implementation plan must assign them:

- retry counts and backoff limits per external operation;
- provider and final-verification timeouts;
- instruction and diagnostic artifact size limits;
- diagnostic-hold expiry;
- bounded post-completion metadata retention;
- live-test budget and teardown timeout;
- first non-production project set used for rollout.

## 24. Reuse and discard guide

Reuse or adapt:

- typed repository identity as a domain concept;
- one managed task-binding convention;
- project-set and repository validation ideas;
- planner and Linear propagation tests;
- workspace path containment;
- fixed-argument Git invocation;
- repository memory facets after scope enforcement;
- bounded polling and local fixture ideas from the prior harness.

Rewrite against current owners:

- config loading and migration;
- scheduler routing integration;
- checkout creation and verification;
- per-job instruction loading;
- runtime and recovery manifests;
- gateway and client schemas;
- memory integration;
- provider and review lifecycle.

Discard:

- permanent `ParentDeferred`;
- empty inventory as a legacy-mode signal;
- direct filesystem cleanup;
- “any `.git` is valid” checkout acceptance;
- hardcoded or arbitrary clone hooks as the identity mechanism;
- one shared bootstrap workflow for all workers;
- default-on migration during ordinary update;
- raw repository URLs in persisted/public state;
- the prior harness assertion that parent workspace absence is success.

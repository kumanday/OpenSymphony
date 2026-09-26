# Upgrading to OpenSymphony 3.0.0

OpenSymphony 3.0.0 adds multi-repository project routing and local ACP v1
worker profiles. Both are explicit choices. An existing single-repository
workflow can continue with `legacy_single` routing and its current harness.

## Before upgrading

1. Stop `opensymphony run` and keep the existing `WORKFLOW.md`, `config.yaml`,
   workspace root, and memory store until the new run has been checked.
2. Install 3.0.0 with Rust 1.97.1 or newer. Run `opensymphony doctor` in the
   target repository to check its local prerequisites.
3. If you want a central configuration, run the read-only migration preflight,
   review its findings, and then apply the migration:

   ```bash
   opensymphony migrate preflight --repo /path/to/target-repo
   opensymphony migrate apply --repo /path/to/target-repo \
     --config /absolute/path/to/config.yaml
   ```

   The generated central config starts with the compatible single-repository
   route. Run `opensymphony run --config /absolute/path/to/config.yaml` to
   select it explicitly. [Configuration migration](configuration.md#configuration-migration)
   describes the backup, activation marker, and rollback checks.

## Enable multi-repository routing

Add an active project set, repository inventory, credentials, and review
profiles to the central config. Set `routing.mode: project_set` only after
every terminal issue has exactly one valid `repo:<alias>` label and every
parent is repository-neutral. Keep repository-specific instructions in each
checkout and integration instructions in the project set. Follow the
[multi-repository guide](multi-repository.md) for the run flow and the
[configuration model](configuration.md#central-configuration) for the full
schema.

Before enabling a non-production project set, pass the
[hermetic and disposable live gate](multi-repository-rollout.md) at the same
clean commit and selected config hash. That gate leaves production activation
to a separate operator decision.

## Select an ACP agent

ACP changes execution, not repository binding. Install and sign in to a local
ACP v1 CLI, configure a named stdio profile, and select it with
`routing.harness: acp` and `routing.harness_profile`. The
[ACP guide](acp.md) shows a profile and explains operator permission handling.
The negotiated live session determines optional features such as persistence,
model choices, and vendor extensions.

## Roll back a migrated configuration

Stop the strict-config instance first. `opensymphony migrate rollback --config
/absolute/path/to/config.yaml` restores the backed-up legacy files only when
the strict-run marker is absent and the migrated memory catalog has not changed
since migration. Preserve the state, workspaces, manifests, and receipts for
any run that has not reached a terminal state. See
[migration recovery](configuration.md#configuration-migration) for the
specific refusal conditions.

# Scripts

Repository-owned helper entrypoints live here.

Current scripts:

- `smoke_local.sh`: runs the static `opensymphony doctor` preflight against `examples/configs/local-dev.yaml`.
- `live_e2e.sh`: runs the opt-in live local suite. It executes the live `doctor` preflight, launches the pinned local OpenHands server, runs the ignored `live_local_suite` integration tests, and writes logs plus scenario summaries under `target/live-local/<timestamp>/` unless `OPENSYMPHONY_LIVE_SUITE_OUTPUT_ROOT` overrides that root.
- `hermetic_multi_repo_lifecycle.sh`: composes the inherited strict-config,
  migration, workspace, scheduler, provider, memory, gateway, TUI, web, and
  desktop regressions into the deterministic multi-repository release gate. It
  writes immutable commit/config-hash evidence below
  `target/multi-repo-lifecycle/<run-id>/`.
- `live_multi_repo_rollout.sh`: opt-in bounded rollout against uniquely named
  private GitHub repositories and a disposable Linear project. It exercises
  failed-check rework; automated review of this repository's PR follows
  `WORKFLOW.md`. It records teardown evidence below
  `target/live-multi-repo/<run-id>/`.

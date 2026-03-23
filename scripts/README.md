# Scripts

Repository-owned helper entrypoints live here.

Current scripts:

- `smoke_local.sh`: runs the static `opensymphony doctor` preflight against `examples/configs/local-dev.yaml`.
- `live_e2e.sh`: runs the opt-in live local suite. It executes the live `doctor` preflight, launches the pinned local OpenHands server, runs the ignored `live_local_suite` integration tests, and writes logs plus scenario summaries under `target/live-local/<timestamp>/` unless `OPENSYMPHONY_LIVE_SUITE_OUTPUT_ROOT` overrides that root.

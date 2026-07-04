---
id: OSYM-865
title: Installer Docs And End-To-End Validation
milestone: "M12.8: Desktop App Installer And Auto-Update"
priority: 3
estimate: 3
blockedBy: ["OSYM-861", "OSYM-862", "OSYM-863", "OSYM-864"]
blocks: []
areas:
  - documentation
  - desktop
  - installer
parent: null
---

## Summary

Finish the user-facing installer wave by updating docs and proving the complete
first-run, custom-path, update, and fallback flows.

## Scope

### In scope

- Update README, operations, desktop, development, and installer docs so
  `opensymphony app` is described as a real first-run installer/launcher.
- Document `--install-path`, update prompts, prebuilt download behavior, and
  source-build fallback behavior.
- Add an end-to-end smoke test using fake release metadata and fake assets.
- Record any remaining platform limitations clearly.

### Out of scope

- Publishing a production-signed native installer.
- Hosted desktop update channels.
- Reworking desktop app UI.

## Deliverables

- Updated docs for normal users and contributors.
- End-to-end installer smoke coverage.
- Release checklist updates for desktop bundle metadata.

## Acceptance Criteria

- [ ] A normal user can run `opensymphony app` from an empty desktop cache and
      get a useful install attempt instead of a missing-manifest dead end.
- [ ] Documentation shows the default install path and `--install-path`.
- [ ] Documentation explains auto-update prompt behavior with default yes.
- [ ] Documentation distinguishes prebuilt download from source-build fallback.
- [ ] End-to-end fake-release tests cover first install, update, custom install
      root, and corrupt cache recovery.

## Test Plan

- Run the end-to-end fake-release installer smoke test.
- Run focused `desktop_launcher` tests.
- Run `cargo fmt --check`.
- Run `git diff --check`.

## Context

- Depends on OSYM-861, OSYM-862, OSYM-863, and OSYM-864.
- Inspect `README.md`.
- Inspect `docs/operations.md`.
- Inspect `docs/desktop.md`.
- Inspect `docs/DEVELOPMENT.md`.
- Inspect `docs/installer-and-distribution.md`.

## Definition of Ready

- [ ] Hidden assumptions from prior discussion are written down.
- [ ] Required files, docs, and dependencies are explicitly referenced.
- [ ] A coding agent could begin execution without additional planning context.

## Notes

This task should make the docs honest: if a platform cannot self-install yet,
say exactly why and what the command will do instead.

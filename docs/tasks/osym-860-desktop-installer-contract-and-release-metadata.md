---
id: OSYM-860
title: Desktop Installer Contract And Release Metadata
milestone: "M12.8: Desktop App Installer And Auto-Update"
priority: 2
estimate: 3
blockedBy: []
blocks: ["OSYM-861", "OSYM-862", "OSYM-863", "OSYM-864"]
areas:
  - cli
  - desktop
  - installer
parent: null
---

## Summary

Define the durable installer contract for `opensymphony app`: release metadata,
installed bundle layout, custom install path semantics, and update-check policy.

## Scope

### In scope

- Define the release metadata shape for downloadable desktop assets.
- Define the installed layout under `~/.opensymphony/desktop` and
  `--install-path <dir>`.
- Decide how the installed manifest evolves from
  `opensymphony-desktop-manifest.json`.
- Define prompt behavior for auto-update, including default yes and
  non-interactive behavior.
- Document fallback order: cached bundle, prebuilt download, source-build
  fallback, clear failure.

### Out of scope

- Implementing download or build logic.
- Publishing real release assets.
- Building a signed native installer.

## Deliverables

- Updated `docs/specs/desktop-app-installer-auto-update-spec.md` if the
  implementation contract changes during discovery.
- Installer metadata structs or fixture files in the CLI test surface.
- Focused tests for metadata parsing and install-root normalization.

## Acceptance Criteria

- [ ] The installer has one documented release metadata contract with version,
      platform, architecture, URL, checksum, and launch target fields.
- [ ] `--install-path <dir>` semantics are documented as an install root, with
      versioned bundles beneath it.
- [ ] Auto-update prompt defaults are explicit for TTY and non-TTY execution.
- [ ] Existing local `--bundle-dir` behavior remains compatible or has a
      documented migration path.
- [ ] Path containment and symlink safety rules are preserved.

## Test Plan

- Run `cargo fmt --check`.
- Run focused `desktop_launcher` unit tests for metadata parsing and install
  root normalization.
- Run `git diff --check`.

## Context

- Read `docs/specs/desktop-app-installer-auto-update-spec.md`.
- Inspect `crates/opensymphony-cli/src/desktop_launcher.rs`.
- Inspect `docs/installer-and-distribution.md`.
- Inspect `README.md` and `docs/operations.md` for current user-facing claims.

## Definition of Ready

- [ ] Hidden assumptions from prior discussion are written down.
- [ ] Required files, docs, and dependencies are explicitly referenced.
- [ ] A coding agent could begin execution without additional planning context.

## Notes

Keep the contract boring. A small release index plus installed manifest is
enough; do not build a package-manager abstraction here.

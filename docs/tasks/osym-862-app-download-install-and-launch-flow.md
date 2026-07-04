---
id: OSYM-862
title: App Download Install And Launch Flow
milestone: "M12.8: Desktop App Installer And Auto-Update"
priority: 2
estimate: 5
blockedBy: ["OSYM-860", "OSYM-861"]
blocks: ["OSYM-864", "OSYM-865"]
areas:
  - cli
  - desktop
  - installer
parent: null
---

## Summary

Teach `opensymphony app` to download, verify, install, and launch a compatible
desktop bundle on first run instead of failing with a missing-manifest error.

## Scope

### In scope

- Add `--install-path <dir>` to choose the desktop install root.
- Preserve the default install root at `~/.opensymphony/desktop`.
- Discover the compatible desktop release asset from the metadata defined by
  OSYM-860 and produced by OSYM-861.
- Download into a temporary staging directory.
- Verify checksum, version, platform, architecture, and launch target before
  promotion.
- Atomically promote the verified bundle into `<install-root>/<version>/`.
- Launch the installed desktop app.
- Keep `--bundle-dir` as a local smoke/install override.

### Out of scope

- Source-build fallback when no prebuilt bundle exists.
- Auto-update prompting for already-installed bundles.
- Native package-manager installation.

## Deliverables

- CLI argument and help text for `--install-path`.
- Download, verify, stage, promote, and launch logic in
  `crates/opensymphony-cli/src/desktop_launcher.rs` or a focused sibling module.
- Fake release server tests for first-run install success and checksum failure.

## Acceptance Criteria

- [ ] `opensymphony app` on an empty cache downloads and installs a compatible
      bundle when release metadata provides one.
- [ ] `opensymphony app --install-path <dir>` installs under
      `<dir>/<version>/` and launches from there.
- [ ] Failed download or checksum verification does not leave a broken promoted
      bundle.
- [ ] Existing verified bundles still launch without re-downloading when no
      update is needed.
- [ ] The missing-bundle error no longer points normal users only at
      `--bundle-dir` when a remote install path is configured.

## Test Plan

- Run focused `desktop_launcher` tests with a fake HTTP release metadata and
  asset server.
- Run `cargo test-system-duckdb --test help` or the relevant help test.
- Run `cargo fmt --check`.
- Run `git diff --check`.

## Context

- Depends on OSYM-860 and OSYM-861.
- Inspect current `run_app`, `ensure_verified_bundle`, and `verify_bundle` in
  `crates/opensymphony-cli/src/desktop_launcher.rs`.
- Reuse existing path containment and symlink rejection helpers.

## Definition of Ready

- [ ] Hidden assumptions from prior discussion are written down.
- [ ] Required files, docs, and dependencies are explicitly referenced.
- [ ] A coding agent could begin execution without additional planning context.

## Notes

Do not add a new async runtime or installer framework. One blocking download
path with good tests is enough for this CLI command.

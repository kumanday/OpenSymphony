---
id: OSYM-863
title: Source Build Fallback And Prerequisites
milestone: "M12.8: Desktop App Installer And Auto-Update"
priority: 3
estimate: 8
blockedBy: ["OSYM-860"]
blocks: ["OSYM-865"]
areas:
  - cli
  - desktop
  - installer
parent: null
---

## Summary

Add a best-effort source-build fallback for platforms without a prebuilt desktop
bundle, including prerequisite detection and guided or automatic installation.

## Scope

### In scope

- Download the matching OpenSymphony source archive for the selected desktop
  version.
- Detect required build tools: Rust/Cargo, Node/npm, and platform desktop/Tauri
  dependencies.
- Attempt prerequisite installation through known package managers when safe
  and available.
- Print exact manual commands when automatic installation cannot continue.
- Build the desktop app from source into a staging directory.
- Generate the installed desktop manifest, verify the result, promote it into
  the install root, and launch it.

### Out of scope

- Guaranteeing fully unattended installation on operating systems that require
  administrator approval.
- Maintaining a complete cross-distro package-manager database forever.
- Replacing prebuilt bundle downloads as the preferred path.

## Deliverables

- Prerequisite probe and installer plan logic.
- Source archive download and build staging path.
- Manifest generation for source-built bundles.
- Tests for missing tools, package-manager command planning, build failure, and
  successful fake build promotion.

## Acceptance Criteria

- [ ] When no compatible prebuilt asset exists, `opensymphony app` explains that
      it will build locally and shows progress.
- [ ] The command checks for Rust/Cargo, Node/npm, and platform desktop
      dependencies before building.
- [ ] Known safe prerequisite installers are attempted with clear output.
- [ ] Unsupported prerequisite installation fails with exact commands for the
      user to run, not a generic missing-manifest error.
- [ ] A source-built bundle uses the same installed verification path as a
      downloaded bundle.

## Test Plan

- Run focused prerequisite probe tests with fake `PATH` and fake package-manager
  commands.
- Run source-build fallback tests with fake command runners.
- Run `cargo fmt --check`.
- Run `git diff --check`.

## Context

- Depends on OSYM-860.
- Inspect `apps/desktop/src-tauri/build.rs`.
- Inspect `apps/desktop/package.json`.
- Inspect `docs/DEVELOPMENT.md` desktop build commands.
- Use official Tauri prerequisite documentation as source material when filling
  platform package lists during implementation.

## Definition of Ready

- [ ] Hidden assumptions from prior discussion are written down.
- [ ] Required files, docs, and dependencies are explicitly referenced.
- [ ] A coding agent could begin execution without additional planning context.

## Notes

Prefer prebuilt download whenever possible. Source build fallback is for
coverage and recovery, not the fast path.

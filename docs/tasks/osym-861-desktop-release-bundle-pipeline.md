---
id: OSYM-861
title: Desktop Release Bundle Pipeline
milestone: "M12.8: Desktop App Installer And Auto-Update"
priority: 2
estimate: 5
blockedBy: ["OSYM-860"]
blocks: ["OSYM-862", "OSYM-864", "OSYM-865"]
areas:
  - desktop
  - installer
  - release
parent: null
---

## Summary

Produce downloadable desktop bundle artifacts and release metadata so
`opensymphony app` has something real to fetch on first run.

## Scope

### In scope

- Add a packaging script or CI workflow that builds the desktop app for the
  current release platform.
- Generate `opensymphony-desktop-manifest.json` or the chosen v2 manifest with
  version, platform, architecture, launch target, and SHA-256.
- Package the desktop bundle into a release asset with a stable naming scheme.
- Publish or document the release metadata asset consumed by the CLI.
- Keep the default Cargo package from compiling desktop dependencies.

### Out of scope

- Full notarization, code signing, MSIX, `.pkg`, `.deb`, `.rpm`, or AppImage
  production hardening.
- Hosted update channels.
- Changing desktop runtime behavior.

## Deliverables

- Packaging script or GitHub Actions workflow for desktop bundle assets.
- Generated release metadata fixture used by CLI tests.
- Documentation for the artifact naming and upload process.

## Acceptance Criteria

- [ ] A maintainer can produce a desktop bundle asset for the current version.
- [ ] The asset includes a manifest and verified launch target.
- [ ] The metadata gives the CLI enough information to select a compatible
      platform/architecture asset without cloning the repo.
- [ ] Release packaging does not add Tauri or npm dependencies to the root
      crates.io package build.
- [ ] Packaging failure leaves no partially published metadata claiming an
      unavailable asset.

## Test Plan

- Run `npm run build --workspace=@opensymphony/desktop`.
- Run the desktop packaging script in a dry-run or local output mode.
- Verify the generated archive contains the manifest and launch target.
- Run `git diff --check`.

## Context

- Depends on OSYM-860.
- Inspect `.github/workflows/`.
- Inspect `apps/desktop/package.json`.
- Inspect `apps/desktop/src-tauri/Cargo.toml` and `tauri.conf.json`.
- Inspect `docs/installer-and-distribution.md`.

## Definition of Ready

- [ ] Hidden assumptions from prior discussion are written down.
- [ ] Required files, docs, and dependencies are explicitly referenced.
- [ ] A coding agent could begin execution without additional planning context.

## Notes

Start with the smallest artifact format the CLI can download and launch. Signed
native installers can come later.

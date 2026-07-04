---
id: OSYM-864
title: Desktop Auto-Update Flow
milestone: "M12.8: Desktop App Installer And Auto-Update"
priority: 2
estimate: 5
blockedBy: ["OSYM-860", "OSYM-861", "OSYM-862"]
blocks: ["OSYM-865"]
areas:
  - cli
  - desktop
  - installer
parent: null
---

## Summary

Before launching an existing desktop bundle, check whether a newer compatible
desktop version is available and prompt to update first, defaulting to yes.

## Scope

### In scope

- Check release metadata before launching a cached bundle.
- Compare semantic versions for the installed bundle and newest compatible
  desktop release.
- Prompt `Update before launch? [Y/n]` when a newer version is available.
- Treat Enter as yes.
- In non-interactive execution, update by default unless an explicit no-update
  flag is added by implementation.
- Keep launching the existing verified bundle if update check fails or update
  fails after a previous verified bundle exists.
- Clean up failed update staging directories.

### Out of scope

- Background updates after launch.
- Delta updates or binary patching.
- Updating the `opensymphony` CLI itself.

## Deliverables

- Update-check and prompt logic in the app launcher.
- Tests for prompt default yes, explicit no, non-interactive default, failed
  update fallback, and successful update promotion.
- Help text and docs for update behavior and any opt-out flag.

## Acceptance Criteria

- [ ] Cached bundles launch directly when already current.
- [ ] A newer compatible version triggers an update prompt before launch.
- [ ] Pressing Enter updates before launch.
- [ ] Choosing no launches the currently installed verified bundle.
- [ ] Failed update does not corrupt the existing working bundle.
- [ ] Non-interactive runs do not hang waiting for input.

## Test Plan

- Run focused `desktop_launcher` tests with fake release metadata versions.
- Run prompt tests with fake stdin/stdout.
- Run `cargo fmt --check`.
- Run `git diff --check`.

## Context

- Depends on OSYM-860, OSYM-861, and OSYM-862.
- Inspect existing CLI prompt helpers in `crates/opensymphony-cli/src/init_repo.rs`
  before adding a new prompt style.
- Keep update behavior separate from `opensymphony update`, which manages the
  CLI and target-repo templates.

## Definition of Ready

- [ ] Hidden assumptions from prior discussion are written down.
- [ ] Required files, docs, and dependencies are explicitly referenced.
- [ ] A coding agent could begin execution without additional planning context.

## Notes

No daemon, no background updater, no long-lived state machine. Check, ask,
install if yes, launch.

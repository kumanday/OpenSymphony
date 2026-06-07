## Dependency Blockers & PR Review Priority

| Priority | Issue | PR | Blocked By | Blocks | Impact |
|:--------:|:------|:--:|:-----------|:-------|:-------|
| 🔴 **P0** | [COE-404](https://linear.app/trilogy-ai-coe/issue/COE-404/desktop-connection-profiles-and-daemon-management) | [#105](https://github.com/kumanday/OpenSymphony/pull/105) | ~~COE-391~~, ~~COE-397~~, ~~COE-398~~ (all Done) | COE-409, COE-410 | Desktop connection profiles, gateway discovery, daemon supervision - 9/9 tests pass, CI green, ready for human merge |
| 🔴 **P0** | [COE-410](https://linear.app/trilogy-ai-coe/issue/COE-410/desktop-local-stream-optimization) | [#107](https://github.com/kumanday/OpenSymphony/pull/107) | ~~COE-391~~, ~~COE-397~~, ~~COE-398~~ (all Done), COE-404 (Merging) | *  | Desktop local stream optimization - PR description cleaned, Evidence section added, AI review triggered, all checks green |
| 🟡 **P1** | [COE-413](https://linear.app/trilogy-ai-coe/issue/COE-413/implementation-plan-generator-stage) | [#96](https://github.com/kumanday/OpenSymphony/pull/96) | ~~COE-406~~ (Done) | COE-415 | Implementation plan generator - all AI review feedback addressed, CI green |
| 🟡 **P1** | [COE-409](https://linear.app/trilogy-ai-coe/issue/COE-409/desktop-settings-keychain-and-native-actions) | [#108](https://github.com/kumanday/OpenSymphony/pull/108) | ~~COE-398~~ (Done), COE-404 (Merging) | COE-423 | Desktop settings, keychain, redaction, native actions - 7/7 tests pass |
| 🟢 **P2** | [COE-402](https://linear.app/trilogy-ai-coe/issue/COE-402/app-shell-dashboard-task-graph-and-run-views) | [#104](https://github.com/kumanday/OpenSymphony/pull/104) | ~~COE-392~~, ~~COE-394~~, ~~COE-397~~ (all Done) | COE-411, COE-414, COE-417, COE-419 | App shell, dashboard, task graph, run views - 97 tests, CI green |
| ⚪ **P3** | [COE-415](https://linear.app/trilogy-ai-coe/issue/COE-415/milestone-issue-and-sub-issue-compiler) | *  | COE-413 | *  | Backlog - blocked by COE-413 merge |
| ⚪ **P3** | [COE-423](https://linear.app/trilogy-ai-coe/issue/COE-423) | *  | COE-409 | *  | Backlog - blocked by COE-409 merge |

**Legend:** 🔴 Critical path | 🟡 Parent issue | 🟢 Ready but lower priority | ⚪ Waiting on dependencies

**Immediate Action:** Merge COE-404 PR #105 (unblocks COE-409 and COE-410). Review and approve COE-413 PR #96, COE-402 PR #104, COE-410 PR #107, and COE-409 PR #108.

---
name: custom-codereview-guide
description: |
  Repository-specific code review guidance for this project.
  Update this file so OpenHands PR review focuses on the right risks.
---

# Custom Code Review Guide

OpenHands PR review will load this file when it is present. Replace this starter content with repository-specific expectations.

## Default Priorities

- Prioritize correctness, regressions, security risks, and missing tests ahead of style-only feedback.
- Treat behavior changes as incomplete unless the PR includes concrete verification or evidence.
- Call out risky data migrations, auth changes, concurrency hazards, and production operability regressions explicitly.

## Customize For This Repository

- List the most security-sensitive paths or subsystems.
- List required validation commands reviewers should expect to see.
- Describe any architecture invariants that must not be broken.
- Add framework- or language-specific review heuristics that matter here.

## Evidence Expectations

- Behavior changes should include test or reproduction output.
- UI changes should include screenshots or recordings.
- Performance-sensitive changes should include benchmark data or timing notes.

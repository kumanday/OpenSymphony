# Provider-agnostic AI PR review system for GitHub using OpenHands

This document is an implementation spec for a coding agent. Follow it to fully add an automated pull request review system to a repository.

## Executive decision

Use **OpenHands PR Review** as the primary integration path.

## Why this is the right design

1. **OpenHands already has a real PR-review integration path** via the `OpenHands/extensions/plugins/pr-review` workflow and action, and it is designed to use repo-local skills and `AGENTS.md` rather than forcing a fork of the reviewer logic.
2. **OpenHands is model-agnostic through LiteLLM**, which means it can work with OpenAI-compatible providers and with native LiteLLM provider routes.
3. **The best security posture is not the stock fork-enabled workflow.** The official OpenHands example uses `pull_request_target` so it can access secrets for fork PRs. For a hardened deployment, do not make that the default. Use a same-repo `pull_request` workflow only, and keep fork PRs out of scope unless maintainers explicitly opt into the risk.
4. **The current all-hands-bot / OpenHands Cloud GitHub App path is not yet the clean reusable primary option.** The current workflow still revolves around PAT-based token management, and a dedicated cloud-mode PR review path is still described as planned work.

## Core design principles

1. **Advisory, not merge-authoritative**
   - AI review should help humans.
   - It should not count as the required human approval gate.
   - It should never auto-approve or auto-merge.

2. **High-signal review**
   - Focus on correctness, security, compatibility, migrations, data integrity, concurrency, caching, retries, error handling, tests, and maintainability problems with real operational impact.
   - Avoid style-only churn and low-value polish comments.

3. **Repo-context aware**
   - Put permanent repo rules in `AGENTS.md`.
   - Put review-specific heuristics in `.agents/skills/custom-codereview-guide.md`.
   - Keep both additive and consistent with any existing `AGENTS.md`, `CLAUDE.md`, or `GEMINI.md` files.

4. **Evidence-based PRs**
   - Require an `Evidence` section in PRs for substantive changes.
   - UI changes should include screenshots or video.
   - Backend or CLI changes should include exact commands and runtime output.
   - This should be enforced by both the PR template and the OpenHands `require-evidence` setting.

5. **Small, reviewable PRs**
   - OpenHands uses the GitHub diff view as the review source of truth and truncates very large diffs.
   - The system should prefer smaller PRs and honest partial coverage over false certainty.

6. **Secret minimization**
   - Only pass the LLM provider key and the GitHub token required to post comments.
   - Do not expose deployment credentials, cloud keys, package publishing tokens, database secrets, or any unrelated secrets to this workflow.

## What must be implemented

Create or update the following:

1. `.github/workflows/ai-pr-review.yml`
2. `.agents/skills/custom-codereview-guide.md`
3. `AGENTS.md` (append an additive AI-review section, or create one if it does not exist)
4. `.github/pull_request_template.md` or the repository’s existing PR template(s)
5. `docs/ai-pr-review-human-setup.md`

Do **not** create a fork-secret workflow by default.

Do **not** enable `pull_request_target` by default.

Do **not** create or modify branch protection or labels through code. Put those steps in the human setup document.

Do **not** invent `CODEOWNERS` entries if real owners are not obvious from the repository. Put a TODO and an example in the human setup document instead.

## Coding-agent implementation rules

1. Preserve existing files and merge changes additively.
2. If `AGENTS.md` already exists, append a clearly labeled section instead of overwriting it.
3. If `.github/pull_request_template.md` already exists, merge the `Evidence` section into the existing template instead of replacing the whole file.
4. If the repo uses `.github/PULL_REQUEST_TEMPLATE/` multiple templates, update the default or the most relevant template and document the rest in the human setup file.
5. If the repo already has `.agents/skills/` or `.openhands/skills/`, use `.agents/skills/` and avoid conflicting duplicate skill names.
6. Use a **unique** skill name like `custom-codereview-guide`, not `code-review`, so the repo skill **supplements** the default OpenHands review skill instead of overriding it.
7. Pin the OpenHands action to a **full commit SHA** before finalizing the implementation.
8. Use the **same SHA** for the action ref and for the `extensions-version` input.
9. Do not ask humans for follow-up information during implementation. Infer what is safe to infer from the repository. Put everything else into `docs/ai-pr-review-human-setup.md`.

## File 1: `.github/workflows/ai-pr-review.yml`

Create this workflow.

Important:
- Replace `__PINNED_OPENHANDS_EXTENSIONS_SHA__` with the same full commit SHA from the canonical `OpenHands/extensions` repository in **both** places before finalizing.
- Keep this workflow on **GitHub-hosted runners**.
- Keep this as a **same-repo** workflow only.
- The Fireworks example uses the repo secret `FIREWORKS_API_KEY` exactly as requested.

```yaml
name: ai-pr-review

on:
  pull_request:
    types: [opened, synchronize, reopened, ready_for_review, labeled]

permissions: {}

concurrency:
  group: ai-pr-review-${{ github.event.pull_request.number }}
  cancel-in-progress: true

jobs:
  review:
    name: openhands-review
    if: >
      github.event.pull_request.draft == false &&
      github.event.pull_request.head.repo.full_name == github.repository &&
      github.event.pull_request.user.login != 'dependabot[bot]' &&
      (
        github.event.action != 'labeled' ||
        github.event.label.name == 'review-this'
      )
    runs-on: ubuntu-24.04
    timeout-minutes: 25
    permissions:
      contents: read
      pull-requests: write
      issues: write
    steps:
      - name: Validate and resolve LLM configuration
        id: llm
        shell: bash
        env:
          AI_REVIEW_PROVIDER_KIND: ${{ vars.AI_REVIEW_PROVIDER_KIND }}
          AI_REVIEW_MODEL_ID: ${{ vars.AI_REVIEW_MODEL_ID }}
          AI_REVIEW_BASE_URL: ${{ vars.AI_REVIEW_BASE_URL }}
          AI_REVIEW_STYLE: ${{ vars.AI_REVIEW_STYLE }}
          AI_REVIEW_REQUIRE_EVIDENCE: ${{ vars.AI_REVIEW_REQUIRE_EVIDENCE }}
        run: |
          set -euo pipefail

          provider_kind="${AI_REVIEW_PROVIDER_KIND:-}"
          model_id="${AI_REVIEW_MODEL_ID:-}"
          base_url="${AI_REVIEW_BASE_URL:-}"
          review_style="${AI_REVIEW_STYLE:-standard}"
          require_evidence="${AI_REVIEW_REQUIRE_EVIDENCE:-true}"

          if [[ -z "$provider_kind" ]]; then
            echo "::error::Missing repository variable AI_REVIEW_PROVIDER_KIND"
            exit 1
          fi

          if [[ -z "$model_id" ]]; then
            echo "::error::Missing repository variable AI_REVIEW_MODEL_ID"
            exit 1
          fi

          case "$provider_kind" in
            openai-compatible)
              if [[ -z "$base_url" ]]; then
                echo "::error::AI_REVIEW_BASE_URL is required when AI_REVIEW_PROVIDER_KIND=openai-compatible"
                exit 1
              fi
              resolved_model="openai/${model_id}"
              resolved_base_url="$base_url"
              ;;
            litellm-native)
              resolved_model="$model_id"
              resolved_base_url="$base_url"
              ;;
            *)
              echo "::error::Unsupported AI_REVIEW_PROVIDER_KIND: $provider_kind"
              echo "::error::Supported values: openai-compatible, litellm-native"
              exit 1
              ;;
          esac

          echo "model=$resolved_model" >> "$GITHUB_OUTPUT"
          echo "base_url=$resolved_base_url" >> "$GITHUB_OUTPUT"
          echo "review_style=$review_style" >> "$GITHUB_OUTPUT"
          echo "require_evidence=$require_evidence" >> "$GITHUB_OUTPUT"

      - name: Run OpenHands PR review
        uses: OpenHands/extensions/plugins/pr-review@__PINNED_OPENHANDS_EXTENSIONS_SHA__
        with:
          llm-model: ${{ steps.llm.outputs.model }}
          llm-base-url: ${{ steps.llm.outputs.base_url }}
          review-style: ${{ steps.llm.outputs.review_style }}
          require-evidence: ${{ steps.llm.outputs.require_evidence }}
          extensions-version: __PINNED_OPENHANDS_EXTENSIONS_SHA__
          llm-api-key: ${{ secrets.FIREWORKS_API_KEY }}
          github-token: ${{ secrets.GITHUB_TOKEN }}
          # lmnr-api-key: ${{ secrets.LMNR_PROJECT_API_KEY }}
```

### Notes for the coding agent

- Do **not** switch this to `pull_request_target` in the primary implementation.
- Do **not** add checkout steps unless you have a specific reason. The OpenHands composite action already handles its own execution path.
- Do **not** add unrelated secrets to this job.
- If the organization blocks third-party actions, note that in the human setup document. Do not silently work around it with an insecure alternative.
- Keep the trigger label name as `review-this`.

## File 2: `.agents/skills/custom-codereview-guide.md`

Create this file.

```markdown
---
name: custom-codereview-guide
description: Project-specific overlay for automated pull request review
triggers:
  - /codereview
---

# Automated PR review overlay

Apply these rules in addition to the default OpenHands code-review skill.

## Review objective

Favor comments that materially improve overall code health.

Report only issues that are high-confidence, actionable, and worth interrupting the author for.

## Priority order

1. Correctness and data integrity
2. Security, authn/authz, secret handling, injection risks, unsafe deserialization, SSRF, path traversal, and permission bugs
3. API contract compatibility, schema compatibility, and migration safety
4. Concurrency, async boundaries, retries, idempotency, caching, and race conditions
5. Error handling, rollback safety, observability, and operational reliability
6. Test adequacy for changed behavior
7. Maintainability problems that are likely to cause future bugs or make operations unsafe

## Review scope rules

- Prioritize changed code and the surrounding execution path.
- Use broader repository context only when needed to validate behavior or architecture fit.
- Prefer the smallest credible fix over broad rewrites.
- Ignore pure formatting and style unless they hide a correctness, security, or maintainability defect.
- De-emphasize generated files, lockfiles, snapshots, vendored code, and build artifacts unless the issue is in the generated artifact itself or the source input is missing.

## Comment rules

- One distinct issue per comment.
- Be direct, concise, and technical.
- Explain the failure mode or risk, not just the symptom.
- Explain when the issue would trigger.
- Suggest the smallest concrete fix that would address the issue.
- Use severity tags only when they improve prioritization: `[high]`, `[medium]`, `[low]`.
- Do not praise, narrate, or add filler.
- Do not approve or request changes on behalf of the agent.
- If no meaningful issues are found, leave a single brief summary comment: `No high-confidence issues found.`

## Uncertainty rules

- If the diff appears truncated or context is insufficient, say the review is partial and avoid speculative claims.
- If something may be intentional, say what assumption would make it safe.
- Do not invent evidence, runtime behavior, or test results.

## Project rules

- Read and obey `AGENTS.md`.
- Respect existing project conventions and architecture boundaries.
- Treat backward compatibility, migration reversibility, and operational safety as first-class concerns.
```

## File 3: `AGENTS.md`

If `AGENTS.md` already exists, append the following section near the end and reconcile any obvious duplication.

If `AGENTS.md` does not exist, create it and include this section as the starting content, plus any repo facts you can safely infer.

Important:
- Replace the angle-bracket placeholders with facts inferred from the repository.
- If you cannot infer a value safely, keep the structure but move the unresolved item into the human setup document as a TODO.

```markdown
## AI PR review overlay

This section applies to automated pull-request review and to coding agents reading this repository.

### Hard rules

- AI review is advisory only. Human reviewers remain the merge gate.
- Do not auto-approve, auto-merge, or treat the agent as a required human approver.
- Prioritize correctness, security, compatibility, migration safety, and operational safety over style.
- If review coverage is partial or the diff is too large for confident review, say so explicitly.

### Repository map

- Primary languages: <fill from repo>
- Main applications or packages: <fill from repo>
- Test commands: <fill from repo>
- Lint or typecheck commands: <fill from repo>
- Generated, vendored, or low-signal paths: <fill from repo>
- Sensitive paths requiring conservative review: <fill from repo>
- Contract boundaries to protect: <fill from repo>

### Project invariants

- <fill with repo-specific invariants discovered from code and docs>
- <fill with migration, API, auth, data, or deployment invariants>
- <fill with any feature-flag, rollout, backward-compatibility, or performance constraints>

### Evidence rule for PRs

Functional changes should include an `Evidence` section in the PR description.

- UI changes: screenshot or video of the real behavior
- Backend, API, CLI, job, or script changes: exact command(s) and resulting output
- AI-assisted implementation when available: conversation link or equivalent trace

### Review heuristics

- Prefer small, local fixes over broad rewrites.
- Avoid commenting on pure style unless it hides a more meaningful issue.
- Treat auth, billing, migrations, async work, caching, retries, idempotency, serialization, and external API integration as high-scrutiny areas.
```

## File 4: PR template update

If the repository already has a PR template, merge the following sections into the existing template.

If the repository has no PR template, create `.github/pull_request_template.md` with the following content.

```markdown
## What changed

Describe the change in a few sentences.

## Why

Explain the user, product, or operational reason for this change.

## Risk and rollback

- Risk level: low / medium / high
- Rollback plan:
- Any migrations, feature flags, or follow-up steps:

## Evidence

Provide concrete proof that the change works.

- UI change: screenshot or video
- Backend, API, CLI, worker, or script change: exact command(s) and runtime output
- If AI-assisted, include a conversation or run link when available

## Reviewer notes

Call out any specific files, invariants, migrations, compatibility concerns, or areas that deserve extra scrutiny.

## Checklist

- [ ] Tests added or updated where appropriate
- [ ] Docs updated where appropriate
- [ ] Backward compatibility considered
- [ ] Migrations are reversible or rollback is documented
- [ ] Evidence section is complete
```

## File 5: `docs/ai-pr-review-human-setup.md`

Create this documentation file for humans.

Use the following content, updating any repo-specific references or notes you discover during implementation.

```markdown
# Human setup for AI PR review

This repository includes an automated PR review workflow powered by OpenHands.

## What the code already does

The repository contains:

- A same-repo GitHub Actions workflow for automated PR review
- A repo-local OpenHands review skill overlay
- An `AGENTS.md` overlay for permanent repo rules
- A PR template with an `Evidence` section

## What a human must do in GitHub

### 1. Add the required secret

Go to:

Settings → Secrets and variables → Actions

Create this repository secret:

- `FIREWORKS_API_KEY` = your Fireworks API key

This is the exact Fireworks key used by the PR review workflow.

## 2. Add the required repository variables

Go to:

Settings → Secrets and variables → Actions → Variables

Create these repository variables:

- `AI_REVIEW_PROVIDER_KIND` = `openai-compatible`
- `AI_REVIEW_MODEL_ID` = `accounts/fireworks/models/glm-5`
- `AI_REVIEW_BASE_URL` = `https://api.fireworks.ai/inference/v1`
- `AI_REVIEW_STYLE` = `standard`
- `AI_REVIEW_REQUIRE_EVIDENCE` = `true`

### 3. Create the manual rerun label

Go to:

Issues → Labels → New label

Create:

- Name: `review-this`
- Description: `Trigger AI PR review`

Adding this label to a same-repo PR will retrigger the review workflow.

### 4. If your org restricts Actions, allow the OpenHands action

If your organization or repository allows only approved actions, allowlist the canonical `OpenHands/extensions` action, or use your organization’s standard third-party action review process.

### 5. Keep this workflow on GitHub-hosted runners

Do not move this workflow to self-hosted runners unless you have separately reviewed the security model for untrusted repository content.

### 6. Keep fork PRs out of scope by default

This implementation intentionally does **not** expose the LLM secret to fork PR workflows.

Do not enable any of the following unless you explicitly accept the risk and redesign the workflow accordingly:

- sending secrets to fork PR workflows
- sending write tokens to fork PR workflows
- changing this workflow to `pull_request_target`

### 7. Recommended branch protection settings

In your protected branch rules:

- Require a pull request before merging
- Require at least one human approval, or more if your team prefers
- Require review from Code Owners
- Dismiss stale approvals when new commits are pushed
- Require conversation resolution before merging

Important:

- Treat the AI review as advisory
- Do not count the AI reviewer as a human approval gate
- Do not make the AI review workflow a required status check if you regularly merge fork PRs or Dependabot PRs, because those are intentionally out of scope in the default hardened design

### 8. CODEOWNERS

If the repository already has `CODEOWNERS`, keep it up to date.

If it does not, create one with real owners. Example only:

```text
*                     @org/platform
/api/                 @org/backend
/web/                 @org/frontend
/db/migrations/       @org/backend @org/dba
```

Do not copy the example blindly. Use real maintainers or teams.

### 9. Validation checklist

Use the following test plan after setup:

1. Open a same-repo non-draft PR with a deliberate bug
   - Expected: AI review runs and posts inline or review comments
2. Push another commit to the same PR
   - Expected: previous in-progress run is canceled and a new run starts
3. Mark a draft PR as ready for review
   - Expected: workflow runs
4. Add the `review-this` label to a same-repo PR
   - Expected: workflow reruns
5. Open a PR without a meaningful `Evidence` section
   - Expected: the review flags missing or weak proof when the change is substantive
6. Open a Dependabot PR
   - Expected: this workflow is skipped by design
7. Open a fork PR
   - Expected: this workflow is skipped by design in the hardened default setup

### 10. Switching to another OpenAI-compatible provider later

To switch providers later:

1. Change these repo variables:
   - `AI_REVIEW_MODEL_ID`
   - `AI_REVIEW_BASE_URL`
2. Update the workflow secret reference from `FIREWORKS_API_KEY` to the new provider’s secret name
3. Leave `AI_REVIEW_PROVIDER_KIND=openai-compatible`

Example pattern for other providers:

- `AI_REVIEW_MODEL_ID` = the provider’s raw model id
- `AI_REVIEW_BASE_URL` = the provider’s OpenAI-compatible base URL ending at `/v1` when that is what the provider expects

### 11. Optional LiteLLM-native provider mode

If you want to use a native LiteLLM provider route instead of the generic OpenAI-compatible route:

- Set `AI_REVIEW_PROVIDER_KIND=litellm-native`
- Set `AI_REVIEW_MODEL_ID` to the full LiteLLM-native model name
- Set `AI_REVIEW_BASE_URL` only if the native route needs it

Example for native Fireworks through LiteLLM:

- `AI_REVIEW_PROVIDER_KIND` = `litellm-native`
- `AI_REVIEW_MODEL_ID` = `fireworks_ai/accounts/fireworks/models/glm-5`
- `AI_REVIEW_BASE_URL` = empty

## Local developer note

If you want to test the provider configuration locally with OpenHands tooling, these environment variables are the useful defaults:

```bash
export FIREWORKS_API_KEY="<your-fireworks-key>"
export LLM_API_KEY="$FIREWORKS_API_KEY"
export LLM_MODEL="openai/accounts/fireworks/models/glm-5"
export LLM_BASE_URL="https://api.fireworks.ai/inference/v1"
```
```

## Why the default implementation does not use `pull_request_target`

Keep this explanation in mind while implementing:

- The official OpenHands example workflow supports fork PR review by using `pull_request_target` and maintainer-triggered labels or reviewer requests.
- That is useful, but it is **not** the hardened default.
- The safe default for this implementation is a same-repo `pull_request` workflow only.

If humans later want fork support, do **not** silently enable it. Document it as an explicit opt-in redesign.

## Optional future enhancement: fork PR support

Do **not** implement this by default.

If maintainers later decide they want AI reviews on fork PRs, the least-bad path is a **separate** workflow based on the official OpenHands label/reviewer gating pattern, with strong warnings and explicit maintainer-only triggering.

If that is ever added later, document all of the following prominently:

- it uses `pull_request_target`
- maintainers must inspect the PR before triggering
- the workflow must not receive unrelated secrets
- the repository accepts the prompt-injection and privileged-workflow risk tradeoff

## Optional future enhancement: Laminar evaluation and A/B testing

OpenHands supports:

- multiple comma-separated models for randomized A/B selection
- Laminar observability and post-hoc evaluation

Do not implement this in the first pass unless the repository owner explicitly wants it.

```

## Additional implementation guidance

### About Dependabot

Keep the Dependabot skip in the workflow.

Reason:
- Dependabot-triggered workflows do not receive normal Actions secrets.
- Leaving the workflow enabled for Dependabot would produce confusing failures.

If humans want AI review on Dependabot later, that should be a separate deliberate design.

### About status checks

Do not design the workflow under the assumption that it will be a required status check.

Reason:
- the hardened default intentionally excludes fork PRs and Dependabot PRs
- making the check mandatory would create avoidable merge friction

### About comments and approvals

The AI reviewer should comment.

It should not:
- count as the required human approval
- auto-approve
- auto-request merge
- auto-dismiss human concerns

### About repo-specific enrichment

The coding agent should inspect the repository and enrich the following where safely possible:

- `AGENTS.md` repository map
- test and lint commands
- generated or vendored paths
- sensitive areas like auth, payments, migrations, queues, background jobs, schema, API contracts, rollout flags

If a fact is not safely inferable, put it in `docs/ai-pr-review-human-setup.md` as a TODO.

## Final checklist for the coding agent

Before finishing, verify all of the following:

- [ ] `.github/workflows/ai-pr-review.yml` exists
- [ ] the OpenHands action ref is pinned to a full commit SHA
- [ ] `extensions-version` uses the same SHA as the action ref
- [ ] `.agents/skills/custom-codereview-guide.md` exists with a **unique** skill name
- [ ] `AGENTS.md` exists and includes the AI review overlay
- [ ] the PR template contains an `Evidence` section
- [ ] `docs/ai-pr-review-human-setup.md` exists
- [ ] no fork-secret workflow was added by default
- [ ] no `pull_request_target` workflow was added by default
- [ ] no auto-approval logic was added
- [ ] the Fireworks configuration is documented exactly with:
  - model id `accounts/fireworks/models/glm-5`
  - base URL `https://api.fireworks.ai/inference/v1`
  - secret name `FIREWORKS_API_KEY`


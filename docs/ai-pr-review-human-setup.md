# AI PR Review System - Human Setup Guide

This document describes the manual setup steps required to activate the AI PR review system.

## Prerequisites

- Repository admin access to configure GitHub settings
- A Fireworks AI account (or another OpenAI-compatible LLM provider)
- GitHub Actions enabled for the repository

## Setup Steps

### 1. Configure Repository Secrets

Go to: **Settings → Secrets and variables → Actions**

Add the following **Secret**:

| Name | Value |
|------|-------|
| `FIREWORKS_API_KEY` | Your Fireworks AI API key |

### 2. Configure Repository Variables

Go to: **Settings → Secrets and variables → Actions → Variables tab**

Add the following **Variables**:

| Name | Value | Description |
|------|-------|-------------|
| `AI_REVIEW_PROVIDER_KIND` | `openai-compatible` | Provider type |
| `AI_REVIEW_MODEL_ID` | `accounts/fireworks/models/glm-5` | Model identifier |
| `AI_REVIEW_BASE_URL` | `https://api.fireworks.ai/inference/v1` | API base URL |
| `AI_REVIEW_STYLE` | `standard` | Review style (standard, strict, lenient) |
| `AI_REVIEW_REQUIRE_EVIDENCE` | `true` | Require Evidence section in PRs |

### 3. Create the Manual Rerun Label

Go to: **Issues → Labels → New label**

Create:

- **Name:** `review-this`
- **Description:** `Trigger AI PR review`
- **Color:** Any (suggested: `#0052CC` blue)

Adding this label to a same-repo PR will retrigger the review workflow.

### 4. Allow the OpenHands Action (if restricted)

If your organization restricts Actions:

Go to: **Settings → Actions → General**

Under "Actions permissions":
- Select "Allow select actions"
- Add `OpenHands/extensions` to the allowlist
- Or use your organization's standard third-party action review process

### 5. Pin the OpenHands Extensions SHA

**TODO:** Before activating, pin the action to a specific commit SHA:

1. Go to https://github.com/OpenHands/extensions/commits/main
2. Find a recent stable commit
3. Copy the full SHA (40 characters)
4. Update `.github/workflows/ai-pr-review.yml`:
   - Replace `__PINNED_OPENHANDS_EXTENSIONS_SHA__` with the actual SHA in **both** places:
     - The `uses:` line
     - The `extensions-version:` input

Example:
```yaml
uses: OpenHands/extensions/plugins/pr-review@abc123def456...
with:
  extensions-version: abc123def456...
```

### 6. Branch Protection Settings (Recommended)

Go to: **Settings → Branches → Add rule**

For your protected branch (e.g., `main`):

- [x] Require a pull request before merging
- [x] Require at least one human approval
- [x] Require review from Code Owners (if CODEOWNERS exists)
- [x] Dismiss stale approvals when new commits are pushed
- [x] Require conversation resolution before merging

**Important:** Do NOT make the AI review workflow a required status check. The hardened default intentionally excludes fork PRs and Dependabot PRs.

### 7. CODEOWNERS (if not exists)

If the repository doesn't have a `CODEOWNERS` file, create one at `.github/CODEOWNERS`:

```text
# Example only - replace with real maintainers
*                     @your-org/maintainers
/crates/opensymphony-orchestrator/  @your-org/orchestration-team
/crates/opensymphony-linear/         @your-org/integrations-team
/crates/opensymphony-openhands/      @your-org/runtime-team
```

**Note:** Use real GitHub usernames or team names, not the examples above.

## Validation Checklist

After setup, verify the system works:

1. **Open a same-repo non-draft PR with a deliberate bug**
   - Expected: AI review runs and posts inline or review comments

2. **Push another commit to the same PR**
   - Expected: Previous in-progress run is canceled and a new run starts

3. **Mark a draft PR as ready for review**
   - Expected: Workflow runs

4. **Add the `review-this` label to a same-repo PR**
   - Expected: Workflow reruns

5. **Open a PR without a meaningful `Evidence` section**
   - Expected: The review flags missing or weak proof when the change is substantive

6. **Open a Dependabot PR**
   - Expected: Workflow is skipped by design

7. **Open a fork PR**
   - Expected: Workflow is skipped by design in the hardened default setup

## Troubleshooting

### Workflow doesn't run

- Check that the PR is not a draft
- Check that the PR is from the same repository (not a fork)
- Check that the author is not Dependabot
- Check that Actions are enabled in repository settings

### "Missing repository variable" errors

- Verify all variables in step 2 are set
- Check variable names match exactly (case-sensitive)

### "Missing secret" errors

- Verify `FIREWORKS_API_KEY` secret is set
- Check the secret is available to Actions (not environment-scoped only)

### No review comments posted

- Check the workflow logs for errors
- Verify the LLM provider is responding
- Check that the PR has actual code changes (not just documentation)

## Security Considerations

### Fork PRs are intentionally excluded

The default implementation does **not** expose the LLM secret to fork PR workflows. This is a security feature, not a bug.

**Do not** enable any of the following unless you explicitly accept the risk:
- Sending secrets to fork PR workflows
- Sending write tokens to fork PR workflows
- Changing this workflow to `pull_request_target`

### Secret minimization

The workflow only receives:
- `FIREWORKS_API_KEY` (LLM provider key)
- `GITHUB_TOKEN` (to post comments)

No deployment credentials, cloud keys, package publishing tokens, or database secrets are exposed.

### GitHub-hosted runners

Keep this workflow on GitHub-hosted runners (`ubuntu-24.04`). Do not move to self-hosted runners unless you have separately reviewed the security model for untrusted repository content.

## Optional: Switching Providers Later

To switch to another OpenAI-compatible provider:

1. Update repository variables:
   - `AI_REVIEW_MODEL_ID` = the provider's model ID
   - `AI_REVIEW_BASE_URL` = the provider's OpenAI-compatible base URL

2. Update the workflow secret reference from `FIREWORKS_API_KEY` to the new provider's secret name

3. Leave `AI_REVIEW_PROVIDER_KIND=openai-compatible`

## Optional: LiteLLM-Native Mode

To use a native LiteLLM provider route instead of OpenAI-compatible:

1. Set `AI_REVIEW_PROVIDER_KIND=litellm-native`
2. Set `AI_REVIEW_MODEL_ID` to the full LiteLLM-native model name
3. Set `AI_REVIEW_BASE_URL` only if the native route needs it

Example for native Fireworks:
- `AI_REVIEW_PROVIDER_KIND` = `litellm-native`
- `AI_REVIEW_MODEL_ID` = `fireworks_ai/accounts/fireworks/models/glm-5`
- `AI_REVIEW_BASE_URL` = (empty)

## Future Enhancements (Not Implemented)

### Fork PR Support

If maintainers later want AI reviews on fork PRs, the least-bad path is a **separate** workflow based on the official OpenHands label/reviewer gating pattern, with:
- `pull_request_target` trigger
- Maintainer-only triggering
- Strong warnings about prompt-injection and privileged-workflow risks

### Laminar Evaluation and A/B Testing

OpenHands supports:
- Multiple comma-separated models for randomized A/B selection
- Laminar observability and post-hoc evaluation

Do not implement unless explicitly requested.

## Support

For issues with the OpenHands action:
- https://github.com/OpenHands/extensions/issues

For issues with this repository's configuration:
- File an issue in this repository

For Fireworks API issues:
- https://docs.fireworks.ai/

---
name: land
description:
  Land a PR by monitoring conflicts, resolving them, waiting for checks, and
  squash-merging when green; use when asked to land, merge, or shepherd a PR to
  completion.
---

# Land

## Goals

- Ensure the PR is conflict-free with main.
- Keep CI green and fix failures when they occur.
- Address all review feedback.
- Confirm PR is ready for merge (green CI, no unresolved comments).
- **DO NOT MERGE** - the user does the final merge. Hand off when ready.

## Preconditions

- `gh` CLI is authenticated.
- You are on the PR branch with a clean working tree.

## Steps

1. Locate the PR for the current branch.
2. Confirm the full gauntlet is green locally before any push.
3. If the working tree has uncommitted changes, commit with the `commit` skill
   and push with the `push` skill before proceeding.
4. Check mergeability and conflicts against main.
5. If conflicts exist, use the `pull` skill to fetch/merge `origin/main` and
   resolve conflicts, then use the `push` skill to publish the updated branch.
6. Address any review comments before merging (see Review Handling below).
7. Watch checks until complete.
8. If checks fail, pull logs, fix the issue, commit with the `commit` skill,
   push with the `push` skill, and re-run checks.
9. When all checks are green and review feedback is addressed, set the Linear
   issue to "Human Review" state. **DO NOT MERGE YOURSELF.**
10. **Context guard:** Before implementing review feedback, confirm it does not
   conflict with the user's stated intent or task context. If it conflicts,
   respond inline with a justification and ask the user before changing code.
11. **Pushback template:** When disagreeing, reply inline with: acknowledge +
   rationale + offer alternative.
12. **Ambiguity gate:** When ambiguity blocks progress, use the clarification
   flow (assign PR to current GH user, mention them, wait for response). Do not
   implement until ambiguity is resolved.
    - If you are confident you know better than the reviewer, you may proceed
      without asking the user, but reply inline with your rationale.
13. **Per-comment mode:** For each review comment, choose one of: accept,
   clarify, or push back. Reply inline stating the mode before changing code.
14. **Reply before change:** Always respond with intended action before pushing
   code changes.

## Commands

```
# Ensure branch and PR context
branch=$(git branch --show-current)
pr_number=$(gh pr view --json number -q .number)
pr_title=$(gh pr view --json title -q .title)
pr_body=$(gh pr view --json body -q .body)

# Check mergeability and conflicts
mergeable=$(gh pr view --json mergeable -q .mergeable)

if [ "$mergeable" = "CONFLICTING" ]; then
  # Run the `pull` skill to handle fetch + merge + conflict resolution.
  # Then run the `push` skill to publish the updated branch.
fi

# Check for unresolved review comments
unresolved=$(gh pr view --json reviewDecision,reviews -q '.reviewDecision')
if [ "$unresolved" = "REVIEW_REQUIRED" ]; then
  echo "Review required - address feedback before merging"
fi

# Watch checks
if ! gh pr checks --watch; then
  gh pr checks
  # Identify failing run and inspect logs
  # gh run list --branch "$branch"
  # gh run view <run-id> --log
  exit 1
fi

# PR is ready - move Linear issue to Human Review
echo "PR #$pr_number is ready:"
echo "  - CI: green"
echo "  - Conflicts: resolved"  
echo "  - Review: addressed"
echo ""
echo "Move the Linear issue to 'Human Review' state via Linear MCP tools."
echo "**DO NOT MERGE** - the user reviews and merges."
```

## Failure Handling

- If checks fail, pull details with `gh pr checks` and `gh run view --log`, then
  fix locally, commit with the `commit` skill, push with the `push` skill, and
  re-run the watch.
- Use judgment to identify flaky failures. If a failure is a flake (e.g., a
  timeout on only one platform), you may proceed without fixing it.
- If CI pushes an auto-fix commit (authored by GitHub Actions), it does not
  trigger a fresh CI run. Detect the updated PR head, pull locally, merge
  `origin/main` if needed, add a real author commit, and force-push to retrigger
  CI, then restart the checks loop.
- If all jobs fail with corrupted pnpm lockfile errors on the merge commit, the
  remediation is to fetch latest `origin/main`, merge, force-push, and rerun CI.
- If mergeability is `UNKNOWN`, wait and re-check.
- Do not merge while review comments are outstanding.
- Do not enable auto-merge; this repo has no required checks so auto-merge can
  skip tests.
- If the remote PR branch advanced due to your own prior force-push or merge,
  avoid redundant merges; re-run the formatter locally if needed and
  `git push --force-with-lease`.

## Review Handling

This repo uses the OpenHands PR Review plugin for automated reviews. Reviews
post as inline comments on specific lines of code.

### AI Review Comments

AI reviews are posted by GitHub Actions with `openhands-review` as the job name.
They are advisory only and do not count as required approvals.

To address AI review feedback:
1. Read the inline comment on the specific line
2. Decide whether to: accept (implement fix), clarify (explain why current code
   is correct), or push back (disagree with reasoning)
3. Reply inline to the review comment explaining your action
4. If accepting, implement the fix, commit, and push
5. Optionally re-trigger AI review by adding the `review-this` label

### Human Review Comments

Human review comments are blocking and must be addressed (responded to and
resolved) before requesting a new review or merging.

- If multiple reviewers comment in the same thread, respond to each comment
  (batching is fine) before closing the thread.
- Fetch review comments via `gh api`:
  ```
  # List PR review comments (inline feedback)
  gh api repos/{owner}/{repo}/pulls/<pr_number>/comments

  # PR issue comments (top-level discussion)
  gh api repos/{owner}/{repo}/issues/<pr_number>/comments
  ```
- Reply to a specific review comment:
  ```
  gh api -X POST /repos/{owner}/{repo}/pulls/<pr_number>/comments \
    -f body='Your response here' -F in_reply_to=<comment_id>
  ```
- `in_reply_to` must be the numeric review comment id (e.g., `2710521800`), not
  the GraphQL node id (e.g., `PRRC_...`), and the endpoint must include the PR
  number (`/pulls/<pr_number>/comments`).

### Review Response Format

For each review comment:
1. Acknowledge the feedback
2. State your intended action (implementing fix / deferring / declining)
3. If implementing, briefly describe the fix
4. If deferring or declining, provide rationale

Example response:
```
Acknowledged. Implementing fix by adding null check before the cast.
Commit: abc123
```

## Scope + PR Metadata

- The PR title and description should reflect the full scope of the change, not
  just the most recent fix.
- If review feedback expands scope, decide whether to include it now or defer
  it. You can accept, defer, or decline feedback. If deferring or declining,
  explain why (e.g., out-of-scope, conflicts with intent, unnecessary).
- Correctness issues raised in review comments should be addressed. If you plan
  to defer or decline a correctness concern, validate first and explain why the
  concern does not apply.
- Classify each review comment as one of: correctness, design, style,
  clarification, scope.
- For correctness feedback, provide concrete validation (test, log, or
  reasoning) before closing it.
- When accepting feedback, include a one-line rationale in the root-level
  update.
- When declining feedback, offer a brief alternative or follow-up trigger.
- Prefer a single consolidated "review addressed" root-level comment after a
  batch of fixes instead of many small updates.
- For doc feedback, confirm the doc change matches behavior (no doc-only edits
  to appease review).
# Codex Code Review - Setup and Switching Guide

OpenSymphony supports two automated PR review providers:

| | OpenHands PR Review plugin | Codex code review |
|---|---|---|
| Runs on | GitHub Actions in your repo | OpenAI infrastructure via the Codex GitHub app |
| Cost | Pay-per-token via `AI_REVIEW_API_KEY` | Included with a ChatGPT subscription |
| Quota | Your LLM provider account | A **separate Code Review usage pool** — GitHub-triggered reviews never compete with Codex implementation runs for quota |
| Initial review | `pull_request.opened` workflow event | Codex **Automatic reviews** setting |
| Re-review after a push | Agent adds the `review-this` label | Agent comments exactly `@codex review` |
| Review guidance | `.agents/skills/custom-codereview-guide.md` | `## Review guidelines` section in `AGENTS.md` (which points at the same guide) |

The active provider is recorded in the target repo's `WORKFLOW.md` under
`## Automated AI PR review` (`Active review provider:`), set by
`opensymphony init --review-provider <openhands|codex|none>` or the matching
interactive prompt.

With either provider the review loop is intentionally iterative: the initial
review runs when the PR opens, and the agent re-triggers a fresh review after
**every** follow-up push, addressing findings in-thread until none remain.
Expect many rounds per PR; that is the designed behavior, not waste. This is
one reason Codex code review pairs well with the Codex harness: review rounds
bill the separate code-review pool, while implementation turns bill the
general pool. Note that only **GitHub-triggered** reviews get the separate
pool; `codex /review` in the CLI or review-via-cloud-task bills general usage.

## Setting up Codex code review

Prerequisites: a ChatGPT subscription (any paid tier) and admin access to the
GitHub repository.

1. Sign in at <https://chatgpt.com/codex> with the ChatGPT account that should
   fund reviews.

   > ⚠️ **Multi-account caveat**: if your GitHub identity is linked to more
   > than one ChatGPT account (for example personal Pro + a Business
   > workspace — workspaces count as separate accounts), the most recently
   > connected account is used for reviews. Connect from the intended account
   > **last**. This is also the most common cause of "quota available but
   > reviews report limit reached".

2. In Codex settings, install the Codex GitHub app and grant it access to the
   repository (or organization).
3. Create a **Codex cloud environment** for the repository at
   <https://chatgpt.com/codex/cloud/settings/environments> (**Create
   environment**, then select the repo). This is
   the "Codex cloud set up for the repository" prerequisite in OpenAI's docs —
   until an environment exists, Codex responds to PRs with
   *"To use Codex here, create an environment for this repo"* instead of
   reviewing. For review-only use the defaults are fine (universal container
   image, no setup script, internet off during the agent phase).
4. Enable **Code review** for the repository.
5. Turn on **Automatic reviews** so every newly opened PR gets an initial
   review without any agent action. In the repository preferences, set
   **Review trigger** to **On PR open**, not **On every push**. OpenSymphony
   already requests follow-up reviews by commenting exactly `@codex review`
   after each fix push; using **On every push** would duplicate that review.
6. Keep review guidance current. Codex applies the `## Review guidelines`
   section of the closest `AGENTS.md`; `opensymphony init` seeds that section
   and the fuller `.agents/skills/custom-codereview-guide.md` it points to.
7. Verify: open a trivial test PR. Codex should react with 👀 within about a
   minute and post a standard GitHub review. Then push a follow-up commit and
   comment `@codex review` to confirm re-review works.

### Agent-facing rules (enforced by `WORKFLOW.md`)

- The re-review trigger must be **exactly** `@codex review`. Mentioning
  `@codex` with any other text starts a Codex **cloud task** that bills
  against general usage limits (not the review pool) and operates outside the
  OpenSymphony workspace.
- Never ask Codex to fix, implement, or push changes — even though the Codex
  product supports "@codex fix the P1 issue". That path bypasses the
  orchestrated loop, pushes commits from outside the managed workspace, and
  bills general usage. All fixes are implemented by the OpenSymphony agent in
  its workspace; Codex only reviews.

## Switching an existing repo: OpenHands → Codex

1. Complete the Codex setup above for the repository.
2. In the target repo's `WORKFLOW.md`, change the marker under
   `## Automated AI PR review`:

   ```
   Active review provider: `codex`
   ```

   If the repo was initialized before this section existed, re-run
   `opensymphony init` (accepting the WORKFLOW.md update) or copy the section
   from the current template.
3. Disable the OpenHands review workflow — keep the file so switching back is
   trivial:

   ```bash
   gh workflow disable ai-pr-review
   ```

   (Alternative: delete `.github/workflows/ai-pr-review.yml`, or unset the
   `AI_REVIEW_MODEL_ID` repository variable — the workflow self-skips when its
   variables are missing.)
4. Commit and push the `WORKFLOW.md` change.
5. Optional cleanup: the `review-this` label and `AI_REVIEW_API_KEY` secret
   are unused by Codex and can stay for an easy switch back.

## Switching back: Codex → OpenHands

1. Set `Active review provider:` back to `openhands` in `WORKFLOW.md`.
2. Re-enable the Actions workflow:

   ```bash
   gh workflow enable ai-pr-review
   ```

3. In Codex settings, disable Code review (or at least Automatic reviews) for
   the repository — otherwise every PR gets two competing AI reviews.
4. Verify the `AI_REVIEW_*` variables, the `AI_REVIEW_API_KEY` secret, and the
   `review-this` label still exist (see
   [ai-pr-review-human-setup.md](ai-pr-review-human-setup.md)).

## Troubleshooting

- **Codex comments "To use Codex here, create an environment for this repo"**:
  the GitHub app is installed but the repository has no Codex cloud
  environment. Create one (setup step 3), then re-trigger with an exact
  `@codex review` comment.
- **No review, no error**: usually stale connector state — disconnect and
  reconnect the GitHub connector in Codex settings.
- **Reviews report limit reached while the dashboard shows quota**: the wrong
  linked ChatGPT account is funding reviews; see the multi-account caveat
  above.
- **A `@codex` comment started a cloud task instead of a review**: the comment
  contained more than the exact phrase `@codex review`. This is the documented
  Codex behavior; the workflow guardrails exist to prevent it.
- **Duplicate Codex reviews after a push**: the repository's Codex **Review
  trigger** is probably set to **On every push**. Set it to **On PR open** so
  the initial review stays automatic and follow-up reviews come only from the
  exact `@codex review` agent comment.
- **Both providers reviewed a PR**: the OpenHands workflow was left enabled
  while Codex Automatic reviews were on. Disable one (steps above).

### Prerequisites

#### Rust toolchain (stable)

**macOS / Linux**
1. Visit [rustup.rs](https://rustup.rs/).
2. Copy the install command shown on that page.
3. Run it in Terminal and follow the prompts.
4. Open a new terminal window after installation.
5. Verify the installation with `rustc --version`.

**Windows**
1. Visit [rustup.rs](https://rustup.rs/).
2. Download the Windows installer shown there.
3. Run it and follow the prompts.
4. Open a new PowerShell or Command Prompt window after installation.
5. Verify the installation with `rustc --version`.

Rust installed via `rustup` uses the stable toolchain by default, which is what OpenSymphony expects.

---

#### Python 3.13.12 with `uv` for the OpenHands server

**Recommended path on macOS, Windows, and Linux**
1. Visit the [uv installation docs](https://docs.astral.sh/uv/getting-started/installation/).
2. Follow the instructions there to install `uv` for your platform.
3. Install Python 3.13.12 with `uv python install 3.13.12`.
4. Verify `uv` with `uv --version`.
5. Verify Python with `python3.13 --version`, or the equivalent command on your platform.

**Alternative**
If you already have Python 3.13.12 installed, you can keep it and just install `uv`. If you need a manual Python installer, use the official [Python downloads page](https://www.python.org/downloads/).

<!-- BEGIN OPENSYMPHONY MANAGED MEMORY SYNC -->

## Current model

- COE-256 contributed: PR #1: COE-257: tighten hosted deployment guidance
- COE-272 contributed: PR #44: COE-272: Centralize scripted fake OpenHands runtime coverage (merge `df2f69c`)
- COE-273 contributed: PR #45: Add live local end-to-end suite (merge `237c52c`)
- COE-274 contributed: PR #46: Package CLI doctor preflight and local setup (merge `898935f`)
- COE-275 contributed: PR #1: COE-257: tighten hosted deployment guidance
- COE-280 contributed: PR #54: Support workflow-owned OpenHands runtime overrides (merge `5663898`)

## Important invariants

- Preserve the behavior described in the recent captured changes unless current code and tests show it has changed.
- Use capsule source refs to inspect the original PR or Linear issue when context is ambiguous.

## Operational flow

- No generated diagram requested for this sync.

## Known gotchas

- No area-specific gotchas were inferred from the selected memory.

## Recent changes

- COE-256: Validation and Local Operations
- COE-272: Fake OpenHands server and protocol contract suite
- COE-273: Live local end-to-end suite
- COE-274: CLI packaging, doctor, and local operations docs
- COE-275: Remote agent-server mode and auth hardening
- COE-280: Support workflow-owned OpenHands auth, provider, and launcher overrides at runtime
- COE-281: Support path-bearing OpenHands base URLs and MCP config at runtime
- COE-282: Support workflow-owned OpenHands conversation reuse policy at runtime
- COE-294: Detect LLM config changes and rehydrate conversations with updated env vars
- COE-382: Add supply-chain and security audits to CI
- COE-383: Decompose oversized session and TUI modules into focused submodules
- COE-384: Expand error-path tests for Linear client and workspace hooks
- COE-385: Resolve runtime tracking TODO in OpenHands session runner
- COE-386: Wire cargo-llvm-cov coverage reporting and regression floor into CI
- COE-387: Audit tracing spans and diagnostics for secret leakage
- COE-400: OpenHands Event Normalization And Runtime Mirror
- COE-405: Linear Milestone, Issue, And Sub-Issue Mutations
- COE-411: Task Graph Editor And Runtime Overlay UI
- COE-412: Runtime Timeline And Terminal/Log Association
- COE-414: Diff, Validation, Approval, And Run Action Views

## Source refs

- COE-256
- COE-272
- COE-273
- COE-274
- COE-275
- COE-280
- COE-281
- COE-282
- COE-294
- COE-382
- COE-383
- COE-384
- COE-385
- COE-386
- COE-387
- COE-400
- COE-405
- COE-411
- COE-412
- COE-414

<!-- END OPENSYMPHONY MANAGED MEMORY SYNC -->

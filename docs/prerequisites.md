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

- COE-389 contributed: PR #85: docs: gateway inventory, domain vocabulary, and DTO boundary checklist (COE-389) (merge `3ed56af`)
- COE-390 contributed: PR #85: docs: gateway inventory, domain vocabulary, and DTO boundary checklist (COE-389) (merge `3ed56af`)
- COE-391 contributed: PR #85: docs: gateway inventory, domain vocabulary, and DTO boundary checklist (COE-389) (merge `3ed56af`)
- COE-392 contributed: PR #85: docs: gateway inventory, domain vocabulary, and DTO boundary checklist (COE-389) (merge `3ed56af`)
- COE-393 contributed: PR #91: feat: Event Journal and Stream Broker (COE-393) (merge `1183bc6`)
- COE-396 contributed: PR #110: feat(gateway): action envelope and receipt framework for COE-396 (merge `5097a96`)

## Important invariants

- Preserve the behavior described in the recent captured changes unless current code and tests show it has changed.
- Use capsule source refs to inspect the original PR or Linear issue when context is ambiguous.

## Operational flow

- No generated diagram requested for this sync.

## Known gotchas

- No area-specific gotchas were inferred from the selected memory.

## Recent changes

- COE-389: Current Gateway Inventory And Vocabulary
- COE-390: Gateway Schemas And Stream Feasibility
- COE-391: Gateway Module, Capabilities, And Dashboard Snapshot
- COE-392: Task Graph, Run Detail, File, And Diff Read APIs
- COE-393: Event Journal And Stream Broker
- COE-396: Action Receipts And Initial Run Actions
- COE-399: Linear Read Coverage And Task Graph Cache
- COE-400: OpenHands Event Normalization And Runtime Mirror
- COE-405: Linear Milestone, Issue, And Sub-Issue Mutations
- COE-411: Task Graph Editor And Runtime Overlay UI
- COE-412: Runtime Timeline And Terminal/Log Association
- COE-414: Diff, Validation, Approval, And Run Action Views
- COE-434: Long-running harness liveness and scheduler/runtime ownership contract
- COE-435: Long-running run observability fixtures and client-facing diagnostics

## Source refs

- COE-389
- COE-390
- COE-391
- COE-392
- COE-393
- COE-396
- COE-399
- COE-400
- COE-405
- COE-411
- COE-412
- COE-414
- COE-434
- COE-435

<!-- END OPENSYMPHONY MANAGED MEMORY SYNC -->

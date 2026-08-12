### Prerequisites

#### Rust 1.97.1 or newer

**macOS / Linux**
1. Visit [rustup.rs](https://rustup.rs/).
2. Copy the install command shown on that page.
3. Run it in Terminal and follow the prompts.
4. Run `rustup update stable`.
5. Open a new terminal window after installation.
6. Verify the installation with `rustc +stable --version`.

**Windows**
1. Visit [rustup.rs](https://rustup.rs/).
2. Download the Windows installer shown there.
3. Run it and follow the prompts.
4. Run `rustup update stable`.
5. Open a new PowerShell or Command Prompt window after installation.
6. Verify the installation with `rustc +stable --version`.

OpenSymphony 2.11.0 and newer require Rust 1.97.1. The repository pins that
toolchain in `rust-toolchain.toml`, and published packages declare the same
minimum through `rust-version`.

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

- COE-461 contributed: PR #164: Expose memory graph DTOs and endpoints (merge `762cec5`)
- COE-464 contributed: PR #165: Derive OKF memory graph metrics and communities (merge `7d58035`)
- COE-465 contributed: PR #166: Add shared frontend graph package (merge `21281d0`)
- COE-467 contributed: PR #171: Add Knowledge Graph renderer (merge `1337903`)
- COE-468 contributed: PR #169: Add Knowledge Graph inspector surface (merge `960541a`)
- COE-469 contributed: PR #170: Wire live memory graph privacy gates (merge `11ac876`)

## Important invariants

- Preserve the behavior described in the recent captured changes unless current code and tests show it has changed.
- Use capsule source refs to inspect the original PR or Linear issue when context is ambiguous.

## Operational flow

- No generated diagram requested for this sync.

## Known gotchas

- No area-specific gotchas were inferred from the selected memory.

## Recent changes

- COE-461: Memory Graph DTOs And Gateway Endpoints
- COE-464: Graph Extraction, Metrics, And Community Pipeline
- COE-465: Shared Graph Frontend Package And Reducers
- COE-467: Three.js Graph Renderer And Worker Layouts
- COE-468: Concept Inspector, Search, Filters, And Accessibility Fallback
- COE-469: Live Memory Graph Integration And Privacy Gates
- COE-471: Graph Scale, Visual Regression, And Web/Desktop Hardening
- COE-520: Route desktop Knowledge Graph through native gateway commands
- COE-525: Desktop Installer Contract And Release Metadata
- COE-526: Desktop Release Bundle Pipeline
- COE-527: Source Build Fallback And Prerequisites
- COE-528: App Download Install And Launch Flow
- COE-529: Desktop Auto-Update Flow
- COE-530: Installer Docs And End-To-End Validation

## Source refs

- COE-461
- COE-464
- COE-465
- COE-467
- COE-468
- COE-469
- COE-471
- COE-520
- COE-525
- COE-526
- COE-527
- COE-528
- COE-529
- COE-530

<!-- END OPENSYMPHONY MANAGED MEMORY SYNC -->

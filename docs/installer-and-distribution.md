# Installer and Distribution Strategy

This document defines a future installer and update shape for OpenSymphony. It
is a planning spec, not the current installation path.

## Audience and outcome

Reader: an OpenSymphony implementation agent or maintainer planning packaging,
desktop, web, hosted, or memory-server distribution work.

After reading this document, the reader should be able to decide whether a
change belongs in the current Cargo install path, the developer build
acceleration path, or a future managed installer/update path.

## Current decision

Keep `cargo install opensymphony` turnkey for normal users. The CLI package uses
bundled DuckDB by default so first-time users do not need to install a native
DuckDB library or configure dynamic loader paths.

Keep downloaded or system-linked DuckDB as an optimization path:

- OpenSymphony contributors use the repository aliases documented in
  [Development Guide](DEVELOPMENT.md).
- Power users may opt into system-linked DuckDB when they are willing to manage
  the native library and runtime loader path themselves.
- Release-sensitive validation still includes the default bundled path.

Do not make downloaded prebuilt DuckDB the default `cargo install` path until a
managed installer or equivalent runtime-library strategy owns native library
placement, loader configuration, health checks, and rollback.

## Why a managed installer exists

The downloaded prebuilt DuckDB mode reduces compile time, but it introduces a
runtime packaging concern. Cargo can compile against a downloaded native
library, yet a binary installed by Cargo is only the executable. It does not
also install `libduckdb` into a stable application-owned runtime location, and
it does not by itself guarantee that the OS dynamic loader can find the
library.

That makes raw `cargo install` a poor place to own native runtime distribution.
A managed installer can make the dependency graph explicit and can install,
verify, update, and remove native runtime assets alongside OpenSymphony
components.

## Future product shape

The future installer should be a signed, platform-native installer or installer
bootstrapper for macOS, Windows, and Linux. It may start as a script for early
testing, but the target experience should be a signed app or package.

The installer should let the user select components:

| Component | Purpose | Native dependencies |
|---|---|---|
| Desktop client | Tauri shell and shared frontend for local or hosted profiles | Desktop webview/runtime requirements |
| Web app server | Gateway-served browser client or local static web server | None beyond the OpenSymphony host if bundled there |
| Orchestrator host | `opensymphony run`, gateway, workspace manager, OpenHands supervisor | Rust executable, OpenHands tooling, optional managed Python/uv |
| Memory server | Local memory MCP server and DuckDB-backed index | DuckDB native runtime |
| OpenHands tooling | Pinned local agent-server bundle | Python/uv and pinned OpenHands server assets |

The desktop and web clients must remain clients. They can start or connect to a
local host profile, and they can connect to hosted mode, but they must not
become the authority for scheduling state.

## Component boundaries

The installer should preserve the same boundaries used by the runtime
architecture:

- `opensymphony run` remains the local execution-plane entrypoint.
- GUI launcher behavior belongs to a separate desktop/web command or app flow.
- The desktop client may supervise a local host, attach to an existing local
  host, or connect to hosted mode through profiles.
- The web client connects to a configured gateway URL.
- The memory server is part of the host/runtime component, not the UI-only
  components.
- DuckDB is a memory-server dependency and should be absent from UI-only
  installs unless the selected profile also installs a local host.

## DuckDB distribution requirements

A managed installer that chooses non-bundled DuckDB must own all of the
following:

- Select the exact DuckDB runtime version compatible with the OpenSymphony
  build.
- Install the native library into an application-owned location.
- Configure the executable so the OS dynamic loader can find that library.
- Verify the memory database can be opened before reporting success.
- Detect a missing or incompatible DuckDB runtime during `doctor`.
- Update DuckDB and OpenSymphony together or refuse an unsafe partial update.
- Roll back both executable and native runtime assets after a failed update.
- Avoid exposing the memory database or admin token through client-only
  components.

The installer may use different platform mechanisms:

- macOS: signed `.app`, `.pkg`, or command-line package with notarization,
  stable library placement, and rpath or loader-path configuration.
- Windows: signed installer or MSIX with `duckdb.dll` placed beside the
  executable or in an application-owned runtime directory.
- Linux: signed archive, AppImage, `.deb`, `.rpm`, or tarball with an explicit
  library directory and wrapper or rpath strategy.

## Update strategy

The current `opensymphony update` command refreshes the CLI through Cargo and
then updates template-managed agent assets in target repositories. That remains
appropriate for the bundled default.

A future managed update path should be staged:

1. Ship a safe version that still installs through the bundled default but adds
   managed-installer awareness to `opensymphony update`.
2. Teach `opensymphony update` to detect whether the current install is
   Cargo-managed, installer-managed, Homebrew-managed, or otherwise externally
   managed.
3. For installer-managed installs, update the selected components and their
   native dependencies together.
4. For Cargo-managed installs, keep using the bundled default unless the user
   explicitly selected a power-user system-linked build.
5. Only after the managed path is proven should non-bundled DuckDB become a
   default for normal user installs.

This staging matters because existing users run the updater from the version
they already have. They cannot rely on new updater behavior until they first
receive a version that installs and launches safely.

## Relation to the roadmap

This work is complementary to, not a replacement for, current roadmap stories:

- M9.5 developer build acceleration keeps source checkouts fast without
  changing normal user installation behavior.
- M10 web client and external gateway work defines the browser and remote
  gateway profiles that an installer can configure.
- M11 hosted alpha reduces the need for local memory-server installation when a
  user selects hosted mode only.
- M13 hardening and release quality is the natural place to convert this spec
  into signed installers, update tests, platform smoke tests, and support docs.

The installer should not be used to blur local and hosted mode. Hosted mode is a
server-side topology with centralized auth, isolation, secrets, runtime pools,
and memory storage. A local installer may configure hosted client profiles, but
it should not pretend that local process execution has hosted isolation.

## Current landing guidance

For normal users:

- Keep the default bundled DuckDB path.
- Install with `cargo install opensymphony`.
- Update with `opensymphony update`.
- Do not require a system DuckDB package.

For power users who want faster native builds and accept manual setup:

- Install DuckDB through the platform package manager or from DuckDB release
  artifacts.
- Build OpenSymphony with `--no-default-features --features duckdb-prebuilt`.
- Set `DUCKDB_LIB_DIR` and `DUCKDB_INCLUDE_DIR` at build time.
- Keep the matching library directory on the OS runtime loader path when
  running the installed binary.
- Update by repeating the same Cargo install command before running
  `opensymphony update` for target-repo template refresh.
- Verify `opensymphony memory status` or a focused memory command before using
  the build for real work.

For OpenSymphony contributors:

- Use the repository aliases for iterative checks.
- Let the repository aliases set `DUCKDB_DOWNLOAD_LIB=1`.
- Run default bundled validation before release-sensitive or packaging changes.
- Do not use a shared target directory across independent agent workspaces
  unless the task explicitly accepts that isolation tradeoff.

## Open questions

- Which platform packaging format should be first: macOS package, Homebrew
  formula, signed shell bootstrapper, Windows installer, Linux tarball, or
  AppImage?
- Should the desktop installer embed the local host by default or install it as
  an optional component selected during first run?
- Should `opensymphony update` become a dispatcher that delegates to the package
  manager that originally installed OpenSymphony?
- Should the memory server eventually ship as a separately versioned local
  service when hosted mode is not selected?
- Which health check should be the release gate for managed DuckDB runtime
  compatibility?

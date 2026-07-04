# Desktop App Installer And Auto-Update Spec

This spec defines the durable contract for `opensymphony app` desktop bundle
installation and update discovery. It does not define signed installer
packaging, download implementation, or source builds.

## Release Index

The release index is a small JSON document. It is the only contract the CLI
needs before choosing a downloadable desktop asset.

```json
{
  "schema_version": 1,
  "assets": [
    {
      "version": "2.7.0",
      "platform": "macos",
      "arch": "aarch64",
      "url": "https://github.com/kumanday/OpenSymphony/releases/download/v2.7.0/opensymphony-desktop-v2.7.0-macos-aarch64.tar.gz",
      "checksum": {
        "algorithm": "sha256",
        "value": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
      },
      "launch_target": {
        "executable": "OpenSymphony",
        "args": []
      }
    }
  ]
}
```

Fields:

- `schema_version`: integer release-index schema version. The current schema is
  `1`.
- `version`: OpenSymphony version carried by the bundle.
- `platform`: Rust target OS string used by the launcher, such as `macos`,
  `linux`, or `windows`.
- `arch`: Rust target architecture string used by the launcher, such as
  `aarch64` or `x86_64`.
- `url`: HTTPS URL for the downloadable bundle archive.
- `checksum`: archive integrity metadata. Schema version `1` requires
  `algorithm: "sha256"` and a lowercase or uppercase hex `value`.
- `launch_target`: launch metadata copied into the installed manifest after the
  bundle is materialized. `executable` is a bundle-relative path and `args`
  defaults to an empty list when omitted.

Unknown top-level and asset fields are ignored by the current contract so a
future publisher can add metadata without breaking old clients. Required fields
above must remain required for schema version `1`.

## Installed Layout

The default install root is:

```text
~/.opensymphony/desktop/
```

`opensymphony app --install-path <dir>` and
`OPENSYMPHONY_DESKTOP_INSTALL_PATH=<dir>` set the install root. The value is not
a bundle directory. The launcher installs or verifies versioned bundles under
that root:

```text
<install-root>/<version>/
  opensymphony-desktop-manifest.json
  <launch-target executable and assets>
```

The hidden `OPENSYMPHONY_DESKTOP_CACHE_ROOT` override remains accepted for
existing smoke tests, but new user-facing docs and scripts should use
`--install-path` or `OPENSYMPHONY_DESKTOP_INSTALL_PATH`.

## Installed Manifest

The installed manifest remains `opensymphony-desktop-manifest.json`. Existing
local bundles stay compatible with the current fields:

```json
{
  "version": "2.7.0",
  "platform": "macos",
  "arch": "aarch64",
  "executable": "OpenSymphony",
  "sha256": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
}
```

The installed `executable` is the release-index `launch_target.executable`.
`sha256` is the executable checksum used at launch time; the release index
checksum covers the downloadable archive before installation. A future schema
may add `schema_version`, `source_url`, or launch `args`, but schema version `1`
must continue to read the manifest above so `--bundle-dir` remains compatible.

## Update Prompt Policy

When the installed version is older than a matching release-index asset:

- TTY execution prompts before replacing the installed version and defaults to
  yes when the operator presses Enter.
- TTY execution accepts an explicit no and launches the cached installed bundle
  when it still verifies.
- Non-TTY execution does not prompt. It may update only when a future
  non-interactive flag or config explicitly opts in; otherwise it launches the
  cached verified bundle or follows the fallback order below.

The launcher must never treat a failed prompt read as consent.

## Fallback Order

`opensymphony app` resolves a launch target in this order:

1. Use the cached installed bundle for the requested OpenSymphony version when
   the installed manifest verifies.
2. Use a matching prebuilt download from the release index when update policy
   permits it. The default index URL is the matching GitHub release asset named
   `opensymphony-desktop-release-index.json`; test and mirror flows can set
   `OPENSYMPHONY_DESKTOP_RELEASE_INDEX_URL`.
3. Use source-build fallback when that later feature is available and its
   prerequisites pass.
4. Fail with a clear repair message.

Early local `--bundle-dir <dir>` and `OPENSYMPHONY_DESKTOP_BUNDLE_DIR=<dir>`
remain a smoke-test materialization path. They copy a local expanded bundle into
`<install-root>/<version>/` and then run the same installed manifest checks.

## Path Safety

Install roots and manifest launch paths preserve the existing launcher safety
rules:

- install roots must not contain parent-directory components and must not be
  filesystem roots;
- versioned bundle directories must stay beneath the install root;
- install roots and versioned bundle directories must not be symlinks;
- local `--bundle-dir` materialization refuses symlinked entries;
- manifest executable paths must be relative and must canonicalize inside the
  versioned bundle directory.

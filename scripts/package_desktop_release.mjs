#!/usr/bin/env node
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import {
  chmod,
  copyFile,
  mkdir,
  readFile,
  rename,
  rm,
  stat,
  writeFile,
} from "node:fs/promises";
import { createReadStream } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const scriptDir = dirname(fileURLToPath(import.meta.url));
const repoRoot = resolve(scriptDir, "..");
const manifestFile = "opensymphony-desktop-manifest.json";
const indexFile = "opensymphony-desktop-release-index.json";

const options = parseArgs(process.argv.slice(2));

if (options.help) {
  printHelp();
  process.exit(0);
}

try {
  await main(options);
} catch (error) {
  console.error(error instanceof Error ? error.message : String(error));
  process.exit(1);
}

async function main(options) {
  const version = options.version ?? (await readWorkspaceVersion());
  await assertDesktopVersions(version);
  const platform = rustPlatform(process.platform);
  const arch = rustArch(process.arch);
  const executableName = process.platform === "win32" ? "OpenSymphony.exe" : "OpenSymphony";
  const outputDir = resolveFromRoot(options.outputDir);
  const targetDir = resolveFromRoot(options.targetDir);
  const binary = options.binary
    ? resolveFromRoot(options.binary)
    : join(targetDir, "release", executableName);
  const assetName = `opensymphony-desktop-v${version}-${platform}-${arch}.tar.gz`;
  const assetBaseUrl =
    options.assetBaseUrl ?? `https://github.com/kumanday/OpenSymphony/releases/download/v${version}`;
  const assetUrl = `${assetBaseUrl.replace(/\/+$/, "")}/${assetName}`;

  if (options.dryRun) {
    console.log(`Dry run: version ${version}`);
    console.log(`Dry run: platform ${platform}`);
    console.log(`Dry run: architecture ${arch}`);
    console.log(`Dry run: ${options.skipBuild ? "skip" : "run"} desktop cargo build`);
    console.log(`Dry run: package ${binary}`);
    console.log(`Dry run: write ${join(outputDir, assetName)}`);
    console.log(`Dry run: write ${join(outputDir, indexFile)}`);
    return;
  }

  if (!options.skipBuild) {
    run("cargo", [
      "build",
      "--release",
      "--manifest-path",
      "apps/desktop/src-tauri/Cargo.toml",
      "--target-dir",
      targetDir,
    ]);
  }

  await assertExecutable(binary);
  await mkdir(outputDir, { recursive: true });

  const staging = join(outputDir, `.tmp-desktop-release-${process.pid}-${Date.now()}`);
  const bundleDir = join(staging, "bundle");
  const stagedArchive = join(staging, assetName);
  const stagedIndex = join(staging, indexFile);
  const finalArchive = join(outputDir, assetName);
  const finalIndex = join(outputDir, indexFile);

  try {
    await mkdir(bundleDir, { recursive: true });
    const stagedExecutable = join(bundleDir, executableName);
    await copyFile(binary, stagedExecutable);
    if (process.platform !== "win32") {
      const mode = (await stat(binary)).mode;
      await chmod(stagedExecutable, mode | 0o755);
    }

    const executableSha = await sha256(stagedExecutable);
    await writeJson(join(bundleDir, manifestFile), {
      version,
      platform,
      arch,
      executable: executableName,
      sha256: executableSha,
    });

    run("tar", [
      "-czf",
      stagedArchive,
      "-C",
      bundleDir,
      manifestFile,
      executableName,
    ]);

    const archiveSha = await sha256(stagedArchive);
    const releaseIndex = await mergeReleaseIndex(finalIndex, {
      version,
      platform,
      arch,
      url: assetUrl,
      checksum: {
        algorithm: "sha256",
        value: archiveSha,
      },
      launch_target: {
        executable: executableName,
        args: [],
      },
    });
    await writeJson(stagedIndex, releaseIndex);

    await rename(stagedArchive, finalArchive);
    await rename(stagedIndex, finalIndex);

    console.log(`Wrote ${finalArchive}`);
    console.log(`Wrote ${finalIndex}`);
  } finally {
    await rm(staging, { recursive: true, force: true });
  }
}

function parseArgs(args) {
  const parsed = {
    outputDir: "dist/desktop-release",
    targetDir: "apps/desktop/src-tauri/target",
    assetBaseUrl: undefined,
    binary: undefined,
    version: undefined,
    skipBuild: false,
    dryRun: false,
    help: false,
  };

  for (let index = 0; index < args.length; index += 1) {
    const arg = args[index];
    switch (arg) {
      case "--output-dir":
        parsed.outputDir = takeValue(args, ++index, arg);
        break;
      case "--target-dir":
        parsed.targetDir = takeValue(args, ++index, arg);
        break;
      case "--asset-base-url":
        parsed.assetBaseUrl = takeValue(args, ++index, arg);
        break;
      case "--binary":
        parsed.binary = takeValue(args, ++index, arg);
        break;
      case "--version":
        parsed.version = takeValue(args, ++index, arg);
        break;
      case "--skip-build":
        parsed.skipBuild = true;
        break;
      case "--dry-run":
        parsed.dryRun = true;
        break;
      case "--help":
      case "-h":
        parsed.help = true;
        break;
      default:
        throw new Error(`unknown argument: ${arg}`);
    }
  }

  return parsed;
}

function takeValue(args, index, flag) {
  const value = args[index];
  if (!value || value.startsWith("--")) {
    throw new Error(`${flag} requires a value`);
  }
  return value;
}

async function readWorkspaceVersion() {
  const cargoToml = await readFile(join(repoRoot, "Cargo.toml"), "utf8");
  const workspacePackage = cargoToml.match(/\[workspace\.package\][\s\S]*?^version\s*=\s*"([^"]+)"/m);
  if (!workspacePackage) {
    throw new Error("could not read [workspace.package] version from Cargo.toml");
  }
  return workspacePackage[1];
}

async function assertDesktopVersions(expectedVersion) {
  const packageJson = JSON.parse(await readFile(join(repoRoot, "apps/desktop/package.json"), "utf8"));
  const tauriConfig = JSON.parse(
    await readFile(join(repoRoot, "apps/desktop/src-tauri/tauri.conf.json"), "utf8"),
  );
  const cargoToml = await readFile(join(repoRoot, "apps/desktop/src-tauri/Cargo.toml"), "utf8");
  const cargoPackage = cargoToml.match(/\[package\][\s\S]*?^version\s*=\s*"([^"]+)"/m);
  const versions = [
    ["apps/desktop/package.json", packageJson.version],
    ["apps/desktop/src-tauri/tauri.conf.json", tauriConfig.version],
    ["apps/desktop/src-tauri/Cargo.toml", cargoPackage?.[1]],
  ];
  const mismatches = versions.filter(([, version]) => version !== expectedVersion);
  if (mismatches.length > 0) {
    throw new Error(
      `desktop version mismatch for release ${expectedVersion}: ${mismatches
        .map(([path, version]) => `${path} has ${version ?? "no version"}`)
        .join(", ")}`,
    );
  }
}

async function mergeReleaseIndex(indexPath, asset) {
  const existing = await readExistingIndex(indexPath);
  return {
    schema_version: 1,
    assets: [
      ...existing.assets.filter(
        (candidate) =>
          !(
            candidate.version === asset.version &&
            candidate.platform === asset.platform &&
            candidate.arch === asset.arch
          ),
      ),
      asset,
    ],
  };
}

async function readExistingIndex(indexPath) {
  let raw;
  try {
    raw = await readFile(indexPath, "utf8");
  } catch (error) {
    if (error?.code === "ENOENT") {
      return { schema_version: 1, assets: [] };
    }
    throw error;
  }
  const parsed = JSON.parse(raw);
  if (parsed.schema_version !== 1 || !Array.isArray(parsed.assets)) {
    throw new Error(`${indexPath} is not a schema_version 1 desktop release index`);
  }
  return parsed;
}

function rustPlatform(platform) {
  if (platform === "darwin") {
    return "macos";
  }
  if (platform === "win32") {
    return "windows";
  }
  return platform;
}

function rustArch(arch) {
  if (arch === "x64") {
    return "x86_64";
  }
  if (arch === "arm64") {
    return "aarch64";
  }
  return arch;
}

function resolveFromRoot(path) {
  return resolve(repoRoot, path);
}

async function assertExecutable(path) {
  const metadata = await stat(path).catch((source) => {
    throw new Error(`desktop executable is missing at ${path}: ${source.message}`);
  });
  if (!metadata.isFile()) {
    throw new Error(`desktop executable is not a file: ${path}`);
  }
}

async function sha256(path) {
  return new Promise((resolveHash, reject) => {
    const hash = createHash("sha256");
    const stream = createReadStream(path);
    stream.on("data", (chunk) => hash.update(chunk));
    stream.on("error", reject);
    stream.on("end", () => resolveHash(hash.digest("hex")));
  });
}

async function writeJson(path, value) {
  await writeFile(path, `${JSON.stringify(value, null, 2)}\n`);
}

function run(command, args) {
  const result = spawnSync(command, args, {
    cwd: repoRoot,
    stdio: "inherit",
    env: process.env,
  });
  if (result.error) {
    throw new Error(`failed to run ${command}: ${result.error.message}`);
  }
  if (result.status !== 0) {
    throw new Error(`${command} ${args.join(" ")} exited with ${result.status}`);
  }
}

function printHelp() {
  console.log(`Usage: node scripts/package_desktop_release.mjs [options]

Build and package the current-platform OpenSymphony desktop bundle.

Options:
  --output-dir <dir>       Output directory (default: dist/desktop-release)
  --target-dir <dir>       Cargo target dir (default: apps/desktop/src-tauri/target)
  --asset-base-url <url>   Release URL prefix for the archive
  --binary <path>          Existing desktop binary to package
  --version <version>      Override the Cargo workspace package version
  --skip-build             Package --binary or the default target binary as-is
  --dry-run                Print planned actions without building or writing
  -h, --help               Show this help
`);
}

#!/usr/bin/env python3
"""Compile/test the production Windows ACP helpers without unrelated CLI dependencies.

The temporary Cargo harness is not a workspace member or published package. Its
runtime dependency definitions come directly from the root manifest.
"""
import argparse
import json
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import tomllib

parser = argparse.ArgumentParser()
parser.add_argument("--check-target", help="Cross-compile check instead of native Windows tests")
args = parser.parse_args()
if args.check_target and "windows" not in args.check_target:
    parser.error("--check-target must name a Windows Rust target")
if not args.check_target and sys.platform != "win32":
    parser.error("native process tests require Windows; use --check-target for compilation only")
root = Path(__file__).resolve().parents[2]
config = tomllib.loads((root / "Cargo.toml").read_text())
deps = {
    "process-wrap": config["target"]["cfg(windows)"]["dependencies"]["process-wrap"],
    "tokio": config["workspace"]["dependencies"]["tokio"],
    "tempfile": config["workspace"]["dependencies"]["tempfile"],
}

def toml(value):
    if isinstance(value, dict):
        return "{ " + ", ".join(f"{key} = {toml(val)}" for key, val in value.items()) + " }"
    return json.dumps(value)

with tempfile.TemporaryDirectory(prefix="acp-windows-") as temp:
    harness = Path(temp)
    (harness / "Cargo.toml").write_text(
        '[package]\nname = "acp-windows-validation"\nversion = "0.0.0"\nedition = "2024"\n'
        '[lib]\npath = "lib.rs"\n[lints.rust]\nunsafe_code = "forbid"\n[dependencies]\n'
        + "\n".join(f"{key} = {toml(value)}" for key, value in deps.items()) + "\n"
    )
    (harness / "lib.rs").write_text(
        '#![cfg(windows)]\n#[path = '
        + json.dumps(str(root / "crates/opensymphony-acp/src/windows_process.rs"))
        + ']\nmod windows_process;\n#[path = '
        + json.dumps(str(root / "crates/opensymphony-workspace/src/environment.rs"))
        + ']\nmod environment;\n#[path = '
        + json.dumps(str(root / "crates/opensymphony-acp/src/windows_path.rs"))
        + ']\nmod windows_path;\n#[path = '
        + json.dumps(str(root / "crates/opensymphony-acp/src/atomic_file.rs"))
        + ']\nmod atomic_file;\n'
    )
    shutil.copyfile(root / "Cargo.lock", harness / "Cargo.lock")
    command = ["cargo", "check", "--tests", "--target", args.check_target] if args.check_target else ["cargo", "test"]
    subprocess.run(command + ["--manifest-path", str(harness / "Cargo.toml")], check=True)

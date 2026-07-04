use std::{env, path::PathBuf, process::Command};

fn main() {
    println!("cargo:rerun-if-changed=../index.html");
    println!("cargo:rerun-if-changed=../package.json");
    println!("cargo:rerun-if-changed=../src");
    println!("cargo:rerun-if-changed=../vite.config.ts");
    println!("cargo:rerun-if-changed=../../../package.json");
    println!("cargo:rerun-if-changed=../../../package-lock.json");
    println!("cargo:rerun-if-changed=../../../packages/api-client/src");
    println!("cargo:rerun-if-changed=../../../packages/gateway-schema/src");
    println!("cargo:rerun-if-changed=../../../packages/state/src");
    println!("cargo:rerun-if-changed=../../../packages/ui-core/src");

    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let desktop_dir = manifest_dir
        .parent()
        .expect("desktop Tauri crate should live under apps/desktop/src-tauri");
    let status = Command::new("npm")
        .args(["run", "build"])
        .current_dir(desktop_dir)
        .status()
        .expect("failed to run npm; install frontend dependencies with `npm ci --include=dev`");
    assert!(status.success(), "desktop frontend build failed");

    tauri_build::build()
}

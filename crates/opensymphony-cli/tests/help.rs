use std::process::Command;

#[test]
fn top_level_help_describes_commands_and_safety_posture() {
    let output = Command::new(env!("CARGO_BIN_EXE_opensymphony"))
        .arg("--help")
        .output()
        .expect("help command should run");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        output.status.success(),
        "top-level help should succeed: stdout={stdout}, stderr={stderr}",
    );
    for snippet in [
        "Operate the OpenSymphony local MVP on a trusted machine",
        "process-level isolation only",
        "Initialize the current target repository with OpenSymphony files",
        "Update the installed CLI and refresh template-managed skills",
        "Install app-managed runtimes and integrations",
        "Run the real orchestrator against the current project workflow",
        "Resume an issue conversation for interactive debugging",
        "Serve the local control-plane demo stream",
        "Attach the FrankenTUI operator client to a control plane",
        "Run local preflight checks for trusted-machine deployment",
        "GraphQL-backed Linear workflows",
    ] {
        assert!(
            stdout.contains(snippet),
            "top-level help should include `{snippet}`: stdout={stdout}",
        );
    }
}

#[test]
fn update_help_explains_self_update_and_skill_refresh() {
    let output = Command::new(env!("CARGO_BIN_EXE_opensymphony"))
        .args(["update", "--help"])
        .output()
        .expect("update help should run");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        output.status.success(),
        "update help should succeed: stdout={stdout}, stderr={stderr}",
    );
    for snippet in [
        "Update the installed CLI and refresh template-managed skills",
        "Usage: opensymphony update",
    ] {
        assert!(
            stdout.contains(snippet),
            "update help should include `{snippet}`: stdout={stdout}",
        );
    }
}

#[test]
fn install_help_explains_available_runtime_targets() {
    let output = Command::new(env!("CARGO_BIN_EXE_opensymphony"))
        .args(["install", "--help"])
        .output()
        .expect("install help should run");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        output.status.success(),
        "install help should succeed: stdout={stdout}, stderr={stderr}",
    );
    for snippet in [
        "Install app-managed runtimes and integrations",
        "Install the pinned app-managed OpenHands agent-server runtime",
        "openhands",
    ] {
        assert!(
            stdout.contains(snippet),
            "install help should include `{snippet}`: stdout={stdout}",
        );
    }
}

#[test]
fn doctor_help_explains_config_and_live_probe_options() {
    let output = Command::new(env!("CARGO_BIN_EXE_opensymphony"))
        .args(["doctor", "--help"])
        .output()
        .expect("doctor help should run");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        output.status.success(),
        "doctor help should succeed: stdout={stdout}, stderr={stderr}",
    );
    for snippet in [
        "Run local preflight checks for trusted-machine deployment",
        "Doctor config YAML path",
        "Run the live OpenHands probe instead of static preflight only",
    ] {
        assert!(
            stdout.contains(snippet),
            "doctor help should include `{snippet}`: stdout={stdout}",
        );
    }
}

#[test]
fn debug_help_explains_issue_lookup_and_config() {
    let output = Command::new(env!("CARGO_BIN_EXE_opensymphony"))
        .args(["debug", "--help"])
        .output()
        .expect("debug help should run");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        output.status.success(),
        "debug help should succeed: stdout={stdout}, stderr={stderr}",
    );
    for snippet in [
        "Resume an issue conversation for interactive debugging",
        "Linear issue identifier or persisted issue ID to resume",
        "Runtime config YAML path; defaults to ./config.yaml when present",
    ] {
        assert!(
            stdout.contains(snippet),
            "debug help should include `{snippet}`: stdout={stdout}",
        );
    }
}

#[test]
fn run_help_explains_config_autodetection() {
    let output = Command::new(env!("CARGO_BIN_EXE_opensymphony"))
        .args(["run", "--help"])
        .output()
        .expect("run help should run");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        output.status.success(),
        "run help should succeed: stdout={stdout}, stderr={stderr}",
    );
    for snippet in [
        "Run the real orchestrator against the current project workflow",
        "Runtime config YAML path; defaults to ./config.yaml when present",
    ] {
        assert!(
            stdout.contains(snippet),
            "run help should include `{snippet}`: stdout={stdout}",
        );
    }
}

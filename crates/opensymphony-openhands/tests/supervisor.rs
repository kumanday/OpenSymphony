use std::{
    fs,
    net::TcpListener,
    path::Path,
    process::{Child, Command, Stdio},
    thread,
    time::Duration,
};

use opensymphony_openhands::{
    ExternalServerConfig, LaunchOwnership, LocalServerSupervisor, LocalServerTooling, ServerState,
    SupervisedServerConfig, SupervisorConfig, SupervisorError,
};
use tempfile::TempDir;

#[test]
fn supervised_start_and_stop_report_owned_process_metadata() {
    let fixture = FakeToolingFixture::new("ready");
    let port = free_port();
    let mut config = SupervisedServerConfig::new(
        LocalServerTooling::load(fixture.tool_dir()).expect("tooling should load"),
    );
    config.port_override = Some(port);
    config.startup_timeout = Duration::from_secs(3);
    config
        .extra_env
        .insert("FAKE_SERVER_MODE".to_string(), "ready".to_string());

    let mut supervisor = LocalServerSupervisor::new(SupervisorConfig::Supervised(config));
    let started = supervisor.start().expect("server should start");

    assert_eq!(started.ownership, LaunchOwnership::Launched);
    assert_eq!(started.state, ServerState::Ready);
    assert_eq!(started.base_url, format!("http://127.0.0.1:{port}"));
    assert_eq!(started.version.as_deref(), Some("1.2.3"));
    assert!(started.pid.is_some());

    let running = supervisor.status().expect("status should work");
    assert_eq!(running.state, ServerState::Ready);

    let stopped = supervisor.stop().expect("stop should work");
    assert_eq!(stopped.state, ServerState::Stopped);
}

#[test]
fn startup_timeout_kills_unready_child() {
    let fixture = FakeToolingFixture::new("slow");
    let port = free_port();
    let mut config = SupervisedServerConfig::new(
        LocalServerTooling::load(fixture.tool_dir()).expect("tooling should load"),
    );
    config.port_override = Some(port);
    config.startup_timeout = Duration::from_millis(200);
    config
        .extra_env
        .insert("FAKE_SERVER_MODE".to_string(), "slow".to_string());
    config
        .extra_env
        .insert("FAKE_READY_DELAY_SECS".to_string(), "2.0".to_string());

    let mut supervisor = LocalServerSupervisor::new(SupervisorConfig::Supervised(config));
    let error = supervisor.start().expect_err("startup should time out");

    assert!(matches!(error, SupervisorError::StartupTimeout { .. }));
    assert_eq!(
        supervisor.status().expect("status should work").state,
        ServerState::Stopped
    );
}

#[test]
fn unexpected_exit_is_reported_before_readiness() {
    let fixture = FakeToolingFixture::new("exit");
    let port = free_port();
    let mut config = SupervisedServerConfig::new(
        LocalServerTooling::load(fixture.tool_dir()).expect("tooling should load"),
    );
    config.port_override = Some(port);
    config.startup_timeout = Duration::from_secs(1);
    config
        .extra_env
        .insert("FAKE_SERVER_MODE".to_string(), "exit".to_string());
    config
        .extra_env
        .insert("FAKE_EXIT_CODE".to_string(), "41".to_string());

    let mut supervisor = LocalServerSupervisor::new(SupervisorConfig::Supervised(config));
    let error = supervisor.start().expect_err("startup should fail");

    assert!(matches!(
        error,
        SupervisorError::UnexpectedExit { code: Some(41), .. }
    ));
}

#[test]
fn stopping_external_mode_never_kills_the_server() {
    let fixture = FakeToolingFixture::new("ready");
    let port = free_port();
    let mut child = fixture.spawn_external_server(port, "ready");

    wait_for_ready(port);

    let mut supervisor = LocalServerSupervisor::new(SupervisorConfig::External(
        ExternalServerConfig::new(format!("http://127.0.0.1:{port}")),
    ));

    let started = supervisor
        .start()
        .expect("external server should be reachable");
    assert_eq!(started.ownership, LaunchOwnership::External);
    assert_eq!(started.state, ServerState::Ready);

    let stopped = supervisor.stop().expect("stop should not fail");
    assert_eq!(stopped.ownership, LaunchOwnership::External);
    assert_eq!(stopped.state, ServerState::Ready);
    assert!(child.try_wait().expect("poll child").is_none());

    child.kill().expect("kill child");
    child.wait().expect("wait child");
}

struct FakeToolingFixture {
    temp_dir: TempDir,
    python: String,
}

impl FakeToolingFixture {
    fn new(default_mode: &str) -> Self {
        let temp_dir = TempDir::new().expect("temp dir");
        let python = resolve_python();
        write_tooling_fixture(temp_dir.path(), &python, default_mode);
        Self { temp_dir, python }
    }

    fn tool_dir(&self) -> &Path {
        self.temp_dir.path()
    }

    fn spawn_external_server(&self, port: u16, mode: &str) -> Child {
        Command::new(&self.python)
            .arg(self.tool_dir().join("fake_server.py"))
            .arg(port.to_string())
            .env("FAKE_SERVER_MODE", mode)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn external fake server")
    }
}

fn write_tooling_fixture(tool_dir: &Path, python: &str, default_mode: &str) {
    let run_local = format!(
        "#!/usr/bin/env bash\nset -euo pipefail\nport=\"${{OPENHANDS_SERVER_PORT:?OPENHANDS_SERVER_PORT is required}}\"\nmode=\"${{FAKE_SERVER_MODE:-{default_mode}}}\"\nexec {python} \"$(cd -- \"$(dirname -- \"${{BASH_SOURCE[0]}}\")\" && pwd)/fake_server.py\" \"$port\" \"$mode\"\n"
    );
    let pyproject = r#"[project]
name = "fixture"
version = "0.0.0"

[project.optional-dependencies]
agent-server = ["openhands-agent-server==1.2.3"]

[tool.opensymphony.openhands_server]
module = "openhands.agent_server"
runtime_env = "RUNTIME"
runtime = "process"
host = "127.0.0.1"
default_port = 8000
port_env = "OPENHANDS_SERVER_PORT"
launcher = "RUNTIME=process bash run-local.sh"
"#;
    let fake_server = r#"import json
import os
import sys
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

port = int(sys.argv[1])
mode = sys.argv[2] if len(sys.argv) > 2 else os.environ.get("FAKE_SERVER_MODE", "ready")

if mode == "exit":
    sys.exit(int(os.environ.get("FAKE_EXIT_CODE", "41")))

if mode == "slow":
    time.sleep(float(os.environ.get("FAKE_READY_DELAY_SECS", "2.0")))

class Handler(BaseHTTPRequestHandler):
    def do_GET(self):
        if self.path == "/openapi.json":
            body = json.dumps({"ok": True}).encode("utf-8")
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)
            return

        self.send_response(404)
        self.end_headers()

    def log_message(self, format, *args):
        return

server = ThreadingHTTPServer(("127.0.0.1", port), Handler)
server.serve_forever()
"#;

    fs::write(tool_dir.join("run-local.sh"), run_local).expect("run-local.sh");
    fs::write(tool_dir.join("pyproject.toml"), pyproject).expect("pyproject.toml");
    fs::write(tool_dir.join("uv.lock"), "version = 1").expect("uv.lock");
    fs::write(tool_dir.join("version.txt"), "1.2.3").expect("version.txt");
    fs::write(tool_dir.join("fake_server.py"), fake_server).expect("fake_server.py");
}

fn resolve_python() -> String {
    for candidate in ["python3", "python"] {
        if Command::new(candidate)
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
        {
            return candidate.to_string();
        }
    }

    panic!("python3 or python must be available for supervisor integration tests");
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .expect("bind free port")
        .local_addr()
        .expect("local addr")
        .port()
}

fn wait_for_ready(port: u16) {
    for _ in 0..20 {
        if TcpListener::bind(("127.0.0.1", port)).is_err() {
            thread::sleep(Duration::from_millis(50));
            return;
        }

        thread::sleep(Duration::from_millis(50));
    }

    panic!("fake server did not start on port {port}");
}

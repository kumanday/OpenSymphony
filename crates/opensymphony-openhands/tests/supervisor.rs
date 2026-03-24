use std::{
    fs,
    io::{Read, Write},
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

    let mut supervisor = LocalServerSupervisor::new(SupervisorConfig::Supervised(Box::new(config)));
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

    let mut supervisor = LocalServerSupervisor::new(SupervisorConfig::Supervised(Box::new(config)));
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

    let mut supervisor = LocalServerSupervisor::new(SupervisorConfig::Supervised(Box::new(config)));
    let error = supervisor.start().expect_err("startup should fail");

    assert!(matches!(
        error,
        SupervisorError::UnexpectedExit { code: Some(41), .. }
    ));
}

#[test]
fn status_preserves_exited_state_after_supervised_child_crashes() {
    let fixture = FakeToolingFixture::new("crash");
    let port = free_port();
    let mut config = SupervisedServerConfig::new(
        LocalServerTooling::load(fixture.tool_dir()).expect("tooling should load"),
    );
    config.port_override = Some(port);
    config.startup_timeout = Duration::from_secs(2);
    config
        .extra_env
        .insert("FAKE_SERVER_MODE".to_string(), "crash".to_string());
    config
        .extra_env
        .insert("FAKE_CRASH_DELAY_SECS".to_string(), "0.1".to_string());
    config
        .extra_env
        .insert("FAKE_EXIT_CODE".to_string(), "42".to_string());

    let mut supervisor = LocalServerSupervisor::new(SupervisorConfig::Supervised(Box::new(config)));
    let started = supervisor
        .start()
        .expect("server should reach readiness first");
    assert_eq!(started.state, ServerState::Ready);

    for _ in 0..40 {
        let status = supervisor.status().expect("status should work");
        if status.state == (ServerState::Exited { code: Some(42) }) {
            return;
        }
        thread::sleep(Duration::from_millis(25));
    }

    panic!("supervisor never reported the exited child state");
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

#[test]
fn external_mode_supports_path_prefixed_base_urls() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener should bind");
    let address = listener
        .local_addr()
        .expect("listener address should resolve");
    let server = thread::spawn(move || {
        for _ in 0..2 {
            let (mut stream, _) = listener.accept().expect("request should connect");
            let mut request = Vec::new();
            let mut chunk = [0_u8; 256];
            while !request.windows(4).any(|window| window == b"\r\n\r\n") {
                let bytes_read = stream.read(&mut chunk).expect("request should read");
                if bytes_read == 0 {
                    break;
                }
                request.extend_from_slice(&chunk[..bytes_read]);
            }

            let request = String::from_utf8(request).expect("request should be valid UTF-8");
            assert!(
                request.starts_with("GET /runtime/openapi.json HTTP/1.1\r\n"),
                "unexpected request: {request:?}"
            );

            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{{}}"
            )
            .expect("response should write");
            stream.flush().expect("response should flush");
        }
    });

    let mut supervisor = LocalServerSupervisor::new(SupervisorConfig::External(
        ExternalServerConfig::new(format!("http://{address}/runtime")),
    ));

    let started = supervisor
        .start()
        .expect("path-prefixed external server should be reachable");
    assert_eq!(started.state, ServerState::Ready);
    assert_eq!(started.ownership, LaunchOwnership::External);

    let status = supervisor.status().expect("status should work");
    assert_eq!(status.state, ServerState::Ready);

    server.join().expect("server thread should finish");
}

#[test]
fn supervised_mode_rejects_a_foreign_ready_server_on_its_target_port() {
    let fixture = FakeToolingFixture::new("ready");
    let port = free_port();
    let mut foreign = fixture.spawn_external_server(port, "ready");

    wait_for_ready(port);

    let mut config = SupervisedServerConfig::new(
        LocalServerTooling::load(fixture.tool_dir()).expect("tooling should load"),
    );
    config.port_override = Some(port);
    config.startup_timeout = Duration::from_secs(1);

    let mut supervisor = LocalServerSupervisor::new(SupervisorConfig::Supervised(Box::new(config)));
    let error = supervisor
        .start()
        .expect_err("supervised mode should reject foreign ready servers");

    assert!(matches!(error, SupervisorError::ExistingReadyServer { .. }));
    assert!(foreign.try_wait().expect("poll foreign child").is_none());

    foreign.kill().expect("kill child");
    foreign.wait().expect("wait child");
}

#[test]
fn supervised_start_supports_relative_tool_dir_paths() {
    let current_dir = std::env::current_dir().expect("cwd should resolve");
    let temp_dir = tempfile::tempdir_in(&current_dir).expect("temp dir");
    let python = resolve_python();
    write_tooling_fixture(temp_dir.path(), &python, "ready");
    let relative_tool_dir = temp_dir
        .path()
        .strip_prefix(&current_dir)
        .expect("fixture should live under the test cwd");
    let port = free_port();
    let mut config = SupervisedServerConfig::new(
        LocalServerTooling::load(relative_tool_dir).expect("tooling should load"),
    );
    config.port_override = Some(port);
    config.startup_timeout = Duration::from_secs(3);
    config
        .extra_env
        .insert("FAKE_SERVER_MODE".to_string(), "ready".to_string());

    let mut supervisor = LocalServerSupervisor::new(SupervisorConfig::Supervised(Box::new(config)));
    let started = supervisor
        .start()
        .expect("relative tooling path should still start the pinned launcher");

    assert_eq!(started.ownership, LaunchOwnership::Launched);
    assert_eq!(started.state, ServerState::Ready);

    let stopped = supervisor.stop().expect("stop should work");
    assert_eq!(stopped.state, ServerState::Stopped);
}

#[test]
fn supervised_start_honors_workflow_command_overrides() {
    let fixture = FakeToolingFixture::new("ready");
    let port = free_port();
    let custom_launcher = fixture.tool_dir().join("custom-run.sh");
    let marker_path = fixture.tool_dir().join("custom-launch-marker.txt");
    fs::write(
        &custom_launcher,
        format!(
            "#!/usr/bin/env bash\nset -euo pipefail\nprintf '%s\\n' \"$0 $*\" > \"{}\"\nexec {} \"$(cd -- \"$(dirname -- \"$0\")\" && pwd)/fake_server.py\" \"$OPENHANDS_SERVER_PORT\" ready\n",
            marker_path.display(),
            fixture.python,
        ),
    )
    .expect("custom launcher should be written");

    let mut config = SupervisedServerConfig::new(
        LocalServerTooling::load(fixture.tool_dir()).expect("tooling should load"),
    );
    config.command = Some(vec![
        "bash".to_string(),
        custom_launcher
            .file_name()
            .expect("file name should exist")
            .to_string_lossy()
            .into_owned(),
    ]);
    config.port_override = Some(port);
    config.startup_timeout = Duration::from_secs(3);

    let mut supervisor = LocalServerSupervisor::new(SupervisorConfig::Supervised(Box::new(config)));
    let started = supervisor.start().expect("workflow override should start");

    assert_eq!(started.state, ServerState::Ready);
    assert!(
        started
            .launcher
            .as_deref()
            .is_some_and(|launcher| launcher.contains("workflow override"))
    );
    let marker = fs::read_to_string(&marker_path).expect("custom launcher marker should exist");
    assert!(marker.contains("custom-run.sh"));

    let stopped = supervisor.stop().expect("stop should work");
    assert_eq!(stopped.state, ServerState::Stopped);
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
agent-server = [
  "openhands-agent-server==1.2.3",
  "openhands-sdk==1.2.3",
  "openhands-tools==1.2.3",
  "openhands-workspace==1.2.3",
]

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
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

port = int(sys.argv[1])
mode = sys.argv[2] if len(sys.argv) > 2 else os.environ.get("FAKE_SERVER_MODE", "ready")

if mode == "exit":
    sys.exit(int(os.environ.get("FAKE_EXIT_CODE", "41")))

if mode == "slow":
    time.sleep(float(os.environ.get("FAKE_READY_DELAY_SECS", "2.0")))

if mode == "crash":
    def crash_later():
        time.sleep(float(os.environ.get("FAKE_CRASH_DELAY_SECS", "0.1")))
        os._exit(int(os.environ.get("FAKE_EXIT_CODE", "42")))

    threading.Thread(target=crash_later, daemon=True).start()

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
    fs::write(
        tool_dir.join("uv.lock"),
        r#"version = 1

[[package]]
name = "fixture"
version = "0.0.0"
source = { virtual = "." }

[package.optional-dependencies]
agent-server = [
  { name = "openhands-agent-server" },
  { name = "openhands-sdk" },
  { name = "openhands-tools" },
  { name = "openhands-workspace" },
]

[[package]]
name = "openhands-agent-server"
version = "1.2.3"
source = { registry = "https://pypi.org/simple" }

[[package]]
name = "openhands-sdk"
version = "1.2.3"
source = { registry = "https://pypi.org/simple" }

[[package]]
name = "openhands-tools"
version = "1.2.3"
source = { registry = "https://pypi.org/simple" }

[[package]]
name = "openhands-workspace"
version = "1.2.3"
source = { registry = "https://pypi.org/simple" }
"#,
    )
    .expect("uv.lock");
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

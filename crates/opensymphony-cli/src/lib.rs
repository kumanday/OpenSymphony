use std::{
    env,
    fmt::Write,
    path::{Path, PathBuf},
    time::Duration,
};

use opensymphony_openhands::{
    LocalServerSupervisor, LocalServerTooling, SupervisedServerConfig, SupervisorConfig,
};

pub const COMMANDS: &[&str] = &["daemon", "tui", "doctor", "linear-mcp"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliOutput {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}

impl CliOutput {
    fn success(stdout: String) -> Self {
        Self {
            exit_code: 0,
            stdout,
            stderr: String::new(),
        }
    }

    fn failure(exit_code: i32, stderr: String) -> Self {
        Self {
            exit_code,
            stdout: String::new(),
            stderr,
        }
    }
}

pub fn run<I, S>(args: I) -> CliOutput
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let mut args = args.into_iter().map(Into::into);
    let _program = args.next().unwrap_or_else(|| "opensymphony".to_string());

    let Some(command) = args.next() else {
        return CliOutput::failure(2, usage());
    };

    let trailing: Vec<String> = args.collect();

    match command.as_str() {
        "doctor" => run_doctor(trailing),
        _ if is_known_command(&command) => CliOutput::failure(1, placeholder_message(&command)),
        _ => CliOutput::failure(2, usage()),
    }
}

pub fn is_known_command(command: &str) -> bool {
    COMMANDS.contains(&command)
}

pub fn placeholder_message(command: &str) -> String {
    format!("opensymphony bootstrap placeholder: `{command}` is not implemented yet.")
}

pub fn usage() -> String {
    format!(
        "opensymphony\navailable commands: {}\ndoctor options: --repo-root <path> --startup-timeout-ms <ms>",
        COMMANDS.join(", ")
    )
}

fn run_doctor(args: Vec<String>) -> CliOutput {
    let options = match DoctorOptions::parse(&args) {
        Ok(options) => options,
        Err(message) => return CliOutput::failure(2, message),
    };

    let repo_root = match resolve_repo_root(options.repo_root.as_deref()) {
        Ok(path) => path,
        Err(message) => return CliOutput::failure(1, message),
    };
    let tool_dir = repo_root.join("tools/openhands-server");
    let tooling = match LocalServerTooling::load(&tool_dir) {
        Ok(tooling) => tooling,
        Err(error) => {
            return CliOutput::failure(
                1,
                format!(
                    "OpenSymphony doctor\nrepo root: {}\ntool dir: {}\nerror: {error}",
                    repo_root.display(),
                    tool_dir.display(),
                ),
            );
        }
    };

    let mut report = String::new();
    let _ = writeln!(report, "OpenSymphony doctor");
    let _ = writeln!(report, "repo root: {}", repo_root.display());
    let _ = writeln!(report, "tool dir: {}", tool_dir.display());
    let _ = writeln!(report, "launcher: {}", tooling.metadata.launcher);
    let _ = writeln!(report, "base URL: {}", tooling.base_url(None));
    let _ = writeln!(report, "version: {}", tooling.version);
    let _ = writeln!(
        report,
        "pin ready: {}",
        yes_no(tooling.pin_status.is_ready())
    );

    let blocking_issues = tooling.pin_status.blocking_issues();
    if !blocking_issues.is_empty() {
        let _ = writeln!(report, "blocking issues:");
        for issue in blocking_issues {
            let _ = writeln!(report, "- {issue}");
        }

        return CliOutput::failure(1, report.trim_end().to_string());
    }

    let mut config = SupervisedServerConfig::new(tooling);
    config.startup_timeout = options.startup_timeout;

    let mut supervisor = LocalServerSupervisor::new(SupervisorConfig::Supervised(config));
    let started = match supervisor.start() {
        Ok(started) => started,
        Err(error) => {
            let _ = writeln!(report, "start: failed");
            let _ = writeln!(report, "error: {error}");
            return CliOutput::failure(1, report.trim_end().to_string());
        }
    };

    let _ = writeln!(report, "start: ok");
    if let Some(pid) = started.pid {
        let _ = writeln!(report, "pid: {pid}");
    }

    let running = match supervisor.status() {
        Ok(status) => status,
        Err(error) => {
            let _ = writeln!(report, "status: failed");
            let _ = writeln!(report, "error: {error}");
            return CliOutput::failure(1, report.trim_end().to_string());
        }
    };
    let _ = writeln!(report, "status: {:?}", running.state);

    match supervisor.stop() {
        Ok(stopped) => {
            let _ = writeln!(report, "stop: {:?}", stopped.state);
            CliOutput::success(report.trim_end().to_string())
        }
        Err(error) => {
            let _ = writeln!(report, "stop: failed");
            let _ = writeln!(report, "error: {error}");
            CliOutput::failure(1, report.trim_end().to_string())
        }
    }
}

fn resolve_repo_root(explicit: Option<&Path>) -> Result<PathBuf, String> {
    let start = match explicit {
        Some(path) => path.to_path_buf(),
        None => env::current_dir()
            .map_err(|source| format!("failed to read current directory: {source}"))?,
    };

    find_repo_root(&start).ok_or_else(|| {
        format!(
            "could not locate a repository root above {} containing tools/openhands-server",
            start.display()
        )
    })
}

fn find_repo_root(start: &Path) -> Option<PathBuf> {
    for candidate in start.ancestors() {
        let tool_dir = candidate.join("tools/openhands-server");
        if tool_dir.join("run-local.sh").exists() && tool_dir.join("pyproject.toml").exists() {
            return Some(candidate.to_path_buf());
        }
    }

    None
}

fn yes_no(value: bool) -> &'static str {
    if value { "yes" } else { "no" }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DoctorOptions {
    repo_root: Option<PathBuf>,
    startup_timeout: Duration,
}

impl DoctorOptions {
    fn parse(args: &[String]) -> Result<Self, String> {
        let mut repo_root = None;
        let mut startup_timeout = Duration::from_secs(5);
        let mut index = 0;

        while index < args.len() {
            match args[index].as_str() {
                "--repo-root" => {
                    index += 1;
                    let value = args.get(index).ok_or_else(usage)?;
                    repo_root = Some(PathBuf::from(value));
                }
                "--startup-timeout-ms" => {
                    index += 1;
                    let value = args.get(index).ok_or_else(usage)?;
                    let milliseconds = value.parse::<u64>().map_err(|_| usage())?;
                    startup_timeout = Duration::from_millis(milliseconds);
                }
                _ => return Err(usage()),
            }

            index += 1;
        }

        Ok(Self {
            repo_root,
            startup_timeout,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::Path,
        process::{Command, Stdio},
    };

    use tempfile::TempDir;

    use super::{COMMANDS, is_known_command, placeholder_message, run, usage};

    #[test]
    fn exposes_expected_command_names() {
        assert_eq!(COMMANDS, &["daemon", "tui", "doctor", "linear-mcp"]);
        assert!(is_known_command("daemon"));
        assert!(!is_known_command("merge"));
    }

    #[test]
    fn renders_usage_with_doctor_options() {
        assert!(usage().contains("doctor options"));
    }

    #[test]
    fn renders_placeholder_message_for_subcommands() {
        assert!(placeholder_message("daemon").contains("not implemented yet"));
    }

    #[test]
    fn doctor_reports_unpinned_tooling() {
        let repo = TempDir::new().expect("temp dir");
        write_repo_tooling(repo.path(), "0+bootstrap.placeholder", true, false);

        let output = run([
            "opensymphony",
            "doctor",
            "--repo-root",
            repo.path().to_str().expect("repo path"),
        ]);

        assert_eq!(output.exit_code, 1);
        assert!(output.stderr.contains("pin ready: no"));
        assert!(output.stderr.contains("bootstrap placeholder"));
    }

    #[test]
    fn doctor_starts_and_stops_a_valid_fake_server() {
        let repo = TempDir::new().expect("temp dir");
        write_repo_tooling(repo.path(), "1.2.3", false, true);

        let output = run([
            "opensymphony",
            "doctor",
            "--repo-root",
            repo.path().to_str().expect("repo path"),
            "--startup-timeout-ms",
            "3000",
        ]);

        assert_eq!(output.exit_code, 0, "{}", output.stderr);
        assert!(output.stdout.contains("start: ok"));
        assert!(output.stdout.contains("stop: Stopped"));
    }

    fn write_repo_tooling(
        repo_root: &Path,
        version: &str,
        placeholder_dependency: bool,
        runnable: bool,
    ) {
        let tool_dir = repo_root.join("tools/openhands-server");
        fs::create_dir_all(&tool_dir).expect("tool dir");

        let python = resolve_python();
        let dependency = if placeholder_dependency {
            "openhands-agent-server-placeholder==0+bootstrap.placeholder"
        } else {
            "openhands-agent-server==1.2.3"
        };
        let lockfile = if runnable {
            "version = 1"
        } else {
            "Placeholder bootstrap file."
        };
        let run_local = if runnable {
            format!(
                "#!/usr/bin/env bash\nset -euo pipefail\nport=\"${{OPENHANDS_SERVER_PORT:-8000}}\"\nexec {python} \"$(cd -- \"$(dirname -- \"${{BASH_SOURCE[0]}}\")\" && pwd)/fake_server.py\" \"$port\"\n"
            )
        } else {
            "#!/usr/bin/env bash\nexit 0\n".to_string()
        };
        let fake_server = r#"import json
import sys
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

port = int(sys.argv[1])

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

ThreadingHTTPServer(("127.0.0.1", port), Handler).serve_forever()
"#;

        fs::write(
            tool_dir.join("pyproject.toml"),
            format!(
                "[project]\nname = \"fixture\"\nversion = \"0.0.0\"\n\n[project.optional-dependencies]\nagent-server = [\"{dependency}\"]\n\n[tool.opensymphony.openhands_server]\nmodule = \"openhands.agent_server\"\nruntime_env = \"RUNTIME\"\nruntime = \"process\"\nhost = \"127.0.0.1\"\ndefault_port = 8000\nport_env = \"OPENHANDS_SERVER_PORT\"\nlauncher = \"RUNTIME=process bash run-local.sh\"\n"
            ),
        )
        .expect("pyproject");
        fs::write(tool_dir.join("uv.lock"), lockfile).expect("uv.lock");
        fs::write(tool_dir.join("version.txt"), version).expect("version");
        fs::write(tool_dir.join("run-local.sh"), run_local).expect("run-local");
        fs::write(tool_dir.join("fake_server.py"), fake_server).expect("fake server");
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

        panic!("python3 or python must be available for CLI doctor tests");
    }
}

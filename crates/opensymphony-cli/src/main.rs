use clap::{Parser, Subcommand};
use opensymphony_linear::{LinearGraphqlWriteClient, LinearWriteOperations};
use opensymphony_linear_mcp::{serve_stdio, McpServer};
use opensymphony_testkit::MemoryTracker;
use std::env;
use std::error::Error;
use std::io::{stdin, stdout, BufReader};
use std::path::Path;

#[derive(Debug, Parser)]
#[command(name = "opensymphony")]
#[command(about = "OpenSymphony developer CLI")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    Daemon,
    Tui,
    Doctor,
    LinearMcp {
        #[arg(long)]
        stdio: bool,
    },
}

#[derive(Clone)]
enum CliLinearBackend {
    Memory(MemoryTracker),
    Live(LinearGraphqlWriteClient),
}

impl LinearWriteOperations for CliLinearBackend {
    fn get_issue(
        &self,
        query: &str,
    ) -> Result<opensymphony_domain::Issue, opensymphony_linear::LinearError> {
        match self {
            Self::Memory(backend) => backend.get_issue(query),
            Self::Live(backend) => backend.get_issue(query),
        }
    }

    fn comment_issue(
        &self,
        issue_id: &str,
        body: &str,
    ) -> Result<opensymphony_domain::Issue, opensymphony_linear::LinearError> {
        match self {
            Self::Memory(backend) => backend.comment_issue(issue_id, body),
            Self::Live(backend) => backend.comment_issue(issue_id, body),
        }
    }

    fn transition_issue(
        &self,
        issue_id: &str,
        state_name: &str,
    ) -> Result<opensymphony_domain::Issue, opensymphony_linear::LinearError> {
        match self {
            Self::Memory(backend) => backend.transition_issue(issue_id, state_name),
            Self::Live(backend) => backend.transition_issue(issue_id, state_name),
        }
    }

    fn link_pr(
        &self,
        issue_id: &str,
        url: &str,
        title: Option<&str>,
    ) -> Result<opensymphony_domain::Issue, opensymphony_linear::LinearError> {
        match self {
            Self::Memory(backend) => backend.link_pr(issue_id, url, title),
            Self::Live(backend) => backend.link_pr(issue_id, url, title),
        }
    }

    fn list_project_states(
        &self,
        project_slug: &str,
    ) -> Result<Vec<String>, opensymphony_linear::LinearError> {
        match self {
            Self::Memory(backend) => backend.list_project_states(project_slug),
            Self::Live(backend) => backend.list_project_states(project_slug),
        }
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let cli = Cli::parse();
    match cli.command {
        Commands::LinearMcp { stdio } => {
            if !stdio {
                eprintln!("`opensymphony linear-mcp` currently supports only `--stdio`.");
                return Ok(());
            }

            let backend = if let Ok(path) = env::var("OPENSYMPHONY_LINEAR_FIXTURE") {
                CliLinearBackend::Memory(MemoryTracker::from_fixture_path(Path::new(&path))?)
            } else {
                CliLinearBackend::Live(LinearGraphqlWriteClient::from_env()?)
            };
            let server = McpServer::new(backend);
            let stdin = stdin();
            let stdout = stdout();
            let mut reader = BufReader::new(stdin.lock());
            let mut writer = stdout.lock();
            serve_stdio(&server, &mut reader, &mut writer)?;
        }
        Commands::Daemon => {
            println!("daemon wiring lands in the orchestrator-driven integration tests for this milestone");
        }
        Commands::Tui => {
            println!("the optional FrankenTUI client is not part of COE-254");
        }
        Commands::Doctor => {
            println!("doctor checks are deferred to the validation milestone");
        }
    }

    Ok(())
}

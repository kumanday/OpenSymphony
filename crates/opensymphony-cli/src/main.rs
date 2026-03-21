use clap::{Parser, Subcommand};
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

fn main() -> Result<(), Box<dyn Error>> {
    let cli = Cli::parse();
    match cli.command {
        Commands::LinearMcp { stdio } => {
            if !stdio {
                eprintln!("`opensymphony linear-mcp` currently supports only `--stdio`.");
                return Ok(());
            }

            let backend = if let Ok(path) = env::var("OPENSYMPHONY_LINEAR_FIXTURE") {
                MemoryTracker::from_fixture_path(Path::new(&path))?
            } else {
                MemoryTracker::new(
                    vec![],
                    vec!["Todo".to_string(), "In Progress".to_string()],
                    vec!["Done".to_string(), "Cancelled".to_string()],
                    vec![
                        "Todo".to_string(),
                        "In Progress".to_string(),
                        "Human Review".to_string(),
                        "Done".to_string(),
                    ],
                )
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

#[tokio::main]
async fn main() -> std::process::ExitCode {
    crate::opensymphony_cli::run().await
}

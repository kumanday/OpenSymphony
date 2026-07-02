use std::{
    env, fs,
    path::{Path, PathBuf},
};

use opensymphony::opensymphony_code_intel::{ParsedDocumentSummary, parse_path, parse_rust_source};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let paths = env::args_os()
        .skip(1)
        .map(PathBuf::from)
        .collect::<Vec<_>>();
    if !paths.is_empty() {
        let summaries = paths
            .iter()
            .map(|path| parse_file(path))
            .collect::<Result<Vec<_>, _>>()?;
        println!("{}", serde_json::to_string_pretty(&summaries)?);
        return Ok(());
    }

    let source = include_str!("../crates/opensymphony-code-intel/fixtures/rust/complete.rs");
    let summary = parse_rust_source(
        Some(PathBuf::from(
            "crates/opensymphony-code-intel/fixtures/rust/complete.rs",
        )),
        source,
    )?;

    println!("{}", serde_json::to_string_pretty(&summary)?);
    Ok(())
}

fn parse_file(path: &Path) -> Result<ParsedDocumentSummary, Box<dyn std::error::Error>> {
    let source = fs::read_to_string(path)?;
    Ok(parse_path(path, &source)?)
}

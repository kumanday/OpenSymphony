use std::path::PathBuf;

use opensymphony::opensymphony_code_intel::parse_rust_source;

fn main() -> Result<(), Box<dyn std::error::Error>> {
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

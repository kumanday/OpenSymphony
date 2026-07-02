use std::{
    env,
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    time::Instant,
};

use opensymphony::opensymphony_code_intel::{ParsedDocumentSummary, parse_path, parse_rust_source};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = env::args_os().skip(1).collect::<Vec<_>>();
    if let Some(repeat) = repeat_count(&args)? {
        return parse_fixture_repeatedly(repeat);
    }

    let paths = args.into_iter().map(PathBuf::from).collect::<Vec<_>>();
    if !paths.is_empty() {
        let summaries = paths
            .iter()
            .map(|path| parse_file(path))
            .collect::<Result<Vec<_>, _>>()?;
        println!("{}", serde_json::to_string_pretty(&summaries)?);
        return Ok(());
    }

    let source = include_str!("../crates/opensymphony-code-intel/fixtures/rust/complete.rs");
    let path = PathBuf::from("crates/opensymphony-code-intel/fixtures/rust/complete.rs");
    let summary = parse_rust_source(Some(path), source)?;

    println!("{}", serde_json::to_string_pretty(&summary)?);
    Ok(())
}

fn parse_fixture_repeatedly(repeat: usize) -> Result<(), Box<dyn std::error::Error>> {
    let source = include_str!("../crates/opensymphony-code-intel/fixtures/rust/complete.rs");
    let path = PathBuf::from("crates/opensymphony-code-intel/fixtures/rust/complete.rs");

    if repeat == 1 {
        let summary = parse_rust_source(Some(path), source)?;
        println!("{}", serde_json::to_string_pretty(&summary)?);
        return Ok(());
    }

    let started = Instant::now();
    let mut symbols = 0;
    let mut captures = 0;
    for _ in 0..repeat {
        let summary = parse_rust_source(Some(path.clone()), source)?;
        symbols = summary.symbols.len();
        captures = summary.captures.len();
    }
    let elapsed = started.elapsed();
    let parses_per_second = repeat as f64 / elapsed.as_secs_f64();
    println!(
        "parsed {repeat} iteration(s) in {:.3}s ({parses_per_second:.2} parses/s); last symbols={symbols}; last captures={captures}",
        elapsed.as_secs_f64()
    );
    Ok(())
}

fn parse_file(path: &Path) -> Result<ParsedDocumentSummary, Box<dyn std::error::Error>> {
    let source = fs::read_to_string(path)?;
    Ok(parse_path(path, &source)?)
}

fn repeat_count(args: &[OsString]) -> Result<Option<usize>, Box<dyn std::error::Error>> {
    let Some(first) = args.first() else {
        return Ok(None);
    };
    let Some(first) = first.to_str() else {
        return Ok(None);
    };

    if first == "--repeat" {
        if args.len() != 2 {
            return Err("--repeat requires exactly one count".into());
        }
        let value = args[1]
            .to_str()
            .ok_or("--repeat count must be valid utf-8")?;
        return Ok(Some(parse_repeat_value(value)?));
    }

    if let Some(value) = first.strip_prefix("--repeat=") {
        if args.len() != 1 {
            return Err("--repeat=<count> cannot be combined with paths".into());
        }
        return Ok(Some(parse_repeat_value(value)?));
    }

    Ok(None)
}

fn parse_repeat_value(value: &str) -> Result<usize, Box<dyn std::error::Error>> {
    let repeat = value.parse::<usize>()?;
    if repeat == 0 {
        return Err("--repeat must be greater than zero".into());
    }
    Ok(repeat)
}

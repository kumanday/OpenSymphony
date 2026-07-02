use std::{env, path::PathBuf, time::Instant};

use opensymphony::opensymphony_code_intel::parse_rust_source;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let repeat = repeat_count()?;
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

fn repeat_count() -> Result<usize, Box<dyn std::error::Error>> {
    let mut args = env::args().skip(1);
    match args.next() {
        None => Ok(1),
        Some(flag) if flag == "--repeat" => {
            let value = args.next().ok_or("--repeat requires a count")?;
            let repeat = value.parse::<usize>()?;
            if repeat == 0 {
                return Err("--repeat must be greater than zero".into());
            }
            Ok(repeat)
        }
        Some(flag) if flag.starts_with("--repeat=") => {
            let repeat = flag["--repeat=".len()..].parse::<usize>()?;
            if repeat == 0 {
                return Err("--repeat must be greater than zero".into());
            }
            Ok(repeat)
        }
        Some(flag) => Err(format!("unknown argument: {flag}").into()),
    }
}

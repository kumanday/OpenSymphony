fn main() {
    let output = opensymphony_cli::run(std::env::args());

    if !output.stdout.is_empty() {
        println!("{}", output.stdout);
    }

    if !output.stderr.is_empty() {
        eprintln!("{}", output.stderr);
    }

    std::process::exit(output.exit_code);
}

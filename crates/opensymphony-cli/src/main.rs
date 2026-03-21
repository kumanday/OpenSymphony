fn main() {
    if let Some(command) = std::env::args().nth(1) {
        if opensymphony_cli::is_known_command(&command) {
            eprintln!("{}", opensymphony_cli::placeholder_message(&command));
            std::process::exit(1);
        }

        eprintln!("{}", opensymphony_cli::usage());
        std::process::exit(2);
    }

    println!("{}", opensymphony_cli::usage());
}

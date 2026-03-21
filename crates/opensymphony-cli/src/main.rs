fn main() {
    if let Some(command) = std::env::args().nth(1) {
        if opensymphony_cli::is_known_command(&command) {
            println!("opensymphony bootstrap placeholder: `{command}` is not implemented yet.");
            return;
        }
    }

    println!("{}", opensymphony_cli::usage());
}

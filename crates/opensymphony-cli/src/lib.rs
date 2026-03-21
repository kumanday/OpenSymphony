pub const COMMANDS: &[&str] = &["daemon", "tui", "doctor", "linear-mcp"];

pub fn is_known_command(command: &str) -> bool {
    COMMANDS.contains(&command)
}

pub fn placeholder_message(command: &str) -> String {
    format!("opensymphony bootstrap placeholder: `{command}` is not implemented yet.")
}

pub fn usage() -> String {
    format!(
        "opensymphony bootstrap placeholder\navailable commands: {}",
        COMMANDS.join(", ")
    )
}

#[cfg(test)]
mod tests {
    use super::{COMMANDS, is_known_command, placeholder_message, usage};

    #[test]
    fn exposes_expected_command_names() {
        assert_eq!(COMMANDS, &["daemon", "tui", "doctor", "linear-mcp"]);
        assert!(is_known_command("daemon"));
        assert!(!is_known_command("merge"));
    }

    #[test]
    fn renders_placeholder_usage() {
        assert!(usage().contains("bootstrap placeholder"));
    }

    #[test]
    fn renders_placeholder_message_for_subcommands() {
        assert!(placeholder_message("daemon").contains("not implemented yet"));
    }
}

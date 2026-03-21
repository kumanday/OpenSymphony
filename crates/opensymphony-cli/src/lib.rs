pub const COMMANDS: &[&str] = &["daemon", "tui", "doctor", "linear-mcp"];

pub fn is_known_command(command: &str) -> bool {
    COMMANDS.contains(&command)
}

pub fn usage() -> String {
    format!(
        "opensymphony bootstrap placeholder\navailable commands: {}",
        COMMANDS.join(", ")
    )
}

#[cfg(test)]
mod tests {
    use super::{COMMANDS, is_known_command, usage};

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
}

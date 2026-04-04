/// Static registry of all slash commands with names and descriptions.

#[derive(Debug, Clone)]
pub struct CommandInfo {
    pub name: &'static str,
    pub description: &'static str,
}

/// Return the full list of slash commands.
pub fn all_commands() -> Vec<CommandInfo> {
    vec![
        CommandInfo { name: "/model", description: "Switch model (opus, sonnet, haiku)" },
        CommandInfo { name: "/fast", description: "Toggle fast mode" },
        CommandInfo { name: "/effort", description: "Set effort (high, medium, low)" },
        CommandInfo { name: "/thinking", description: "Toggle thinking display (on/off)" },
        CommandInfo { name: "/compact", description: "Compact context (micro | snip N-M | auto)" },
        CommandInfo { name: "/clear", description: "Clear conversation history" },
        CommandInfo { name: "/status", description: "Show session status" },
        CommandInfo { name: "/cost", description: "Show session cost" },
        CommandInfo { name: "/permissions", description: "Set permission mode (auto/ask/yolo)" },
        CommandInfo { name: "/config", description: "View/edit runtime config" },
        CommandInfo { name: "/memory", description: "Memory operations (list/search/clear)" },
        CommandInfo { name: "/doctor", description: "Run diagnostics" },
        CommandInfo { name: "/export", description: "Export conversation to JSON" },
        CommandInfo { name: "/diff", description: "Show git diff" },
        CommandInfo { name: "/bug", description: "Report a bug" },
        CommandInfo { name: "/login", description: "Re-authenticate" },
        CommandInfo { name: "/vim", description: "Toggle vim mode" },
        CommandInfo { name: "/hooks", description: "Show configured hooks" },
        CommandInfo { name: "/help", description: "Show all commands" },
    ]
}

/// Filter commands by case-insensitive prefix match on name.
pub fn filter_commands(prefix: &str) -> Vec<&'static CommandInfo> {
    // Leak the vec so we can return &'static references — the list is small
    // and lives for the entire program lifetime anyway.
    use std::sync::OnceLock;
    static COMMANDS: OnceLock<Vec<CommandInfo>> = OnceLock::new();
    let commands = COMMANDS.get_or_init(all_commands);

    let lower = prefix.to_ascii_lowercase();
    commands
        .iter()
        .filter(|cmd| cmd.name.to_ascii_lowercase().starts_with(&lower))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filter_returns_all_on_slash() {
        let results = filter_commands("/");
        assert_eq!(results.len(), all_commands().len());
    }

    #[test]
    fn filter_returns_model_on_mo() {
        let results = filter_commands("/mo");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "/model");
    }

    #[test]
    fn filter_returns_empty_on_xyz() {
        let results = filter_commands("/xyz");
        assert!(results.is_empty());
    }

    #[test]
    fn filter_is_case_insensitive() {
        let results = filter_commands("/MO");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "/model");
    }

    #[test]
    fn filter_multiple_matches() {
        // /cost, /compact, /clear, /config — all start with /c
        let results = filter_commands("/c");
        assert!(results.len() > 1);
        assert!(results.iter().all(|cmd| cmd.name.starts_with("/c")));
    }
}

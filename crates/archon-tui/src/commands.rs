/// Command catalog for slash-command autocomplete.
///
/// Populated at startup from the bin crate's `Registry::primaries_with_descriptions()`,
/// NOT hand-maintained. The catalog is injected via [`set_catalog`] before the
/// TUI event loop begins.
use std::sync::RwLock;

#[derive(Debug, Clone)]
pub struct CommandInfo {
    pub name: String,
    pub description: String,
}

/// Global command catalog. Set at startup from the registry.
static CATALOG: RwLock<Vec<CommandInfo>> = RwLock::new(Vec::new());

/// Inject the command catalog. Called at TUI startup; tests may call repeatedly.
pub fn set_catalog(catalog: Vec<CommandInfo>) {
    *CATALOG.write().expect("catalog lock poisoned") = catalog;
}

/// Return a snapshot of the full command list.
pub fn all_commands() -> Vec<CommandInfo> {
    CATALOG.read().expect("catalog lock poisoned").clone()
}

/// Filter commands by case-insensitive prefix match on name.
pub fn filter_commands(prefix: &str) -> Vec<CommandInfo> {
    let catalog = CATALOG.read().expect("catalog lock poisoned");
    let lower = prefix.to_ascii_lowercase();
    catalog
        .iter()
        .filter(|cmd| cmd.name.to_ascii_lowercase().starts_with(&lower))
        .cloned()
        .collect()
}

/// Core filter logic operating on an explicit slice. Tests use this directly
/// to avoid depending on the global catalog.
#[allow(dead_code)]
fn filter(commands: &[CommandInfo], prefix: &str) -> Vec<CommandInfo> {
    let lower = prefix.to_ascii_lowercase();
    commands
        .iter()
        .filter(|cmd| cmd.name.to_ascii_lowercase().starts_with(&lower))
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_catalog() -> Vec<CommandInfo> {
        vec![
            CommandInfo {
                name: "/model".into(),
                description: "Switch model".into(),
            },
            CommandInfo {
                name: "/cost".into(),
                description: "Show cost".into(),
            },
            CommandInfo {
                name: "/compact".into(),
                description: "Compact context".into(),
            },
            CommandInfo {
                name: "/clear".into(),
                description: "Clear history".into(),
            },
            CommandInfo {
                name: "/config".into(),
                description: "View config".into(),
            },
            CommandInfo {
                name: "/help".into(),
                description: "Show help".into(),
            },
        ]
    }

    #[test]
    fn filter_returns_all_on_slash() {
        let catalog = make_catalog();
        let results = filter(&catalog, "/");
        assert_eq!(results.len(), catalog.len());
    }

    #[test]
    fn filter_returns_model_on_mo() {
        let catalog = make_catalog();
        let results = filter(&catalog, "/mo");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "/model");
    }

    #[test]
    fn filter_returns_empty_on_xyz() {
        let catalog = make_catalog();
        let results = filter(&catalog, "/xyz");
        assert!(results.is_empty());
    }

    #[test]
    fn filter_is_case_insensitive() {
        let catalog = make_catalog();
        let results = filter(&catalog, "/MO");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "/model");
    }

    #[test]
    fn filter_multiple_matches() {
        let catalog = make_catalog();
        // /cost, /compact, /clear, /config — all start with /c
        let results = filter(&catalog, "/c");
        assert!(results.len() > 1);
        assert!(results.iter().all(|cmd| cmd.name.starts_with("/c")));
    }
}

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
                name: "/trading data status".into(),
                description: "Trading data status".into(),
            },
            CommandInfo {
                name: "/trading data list".into(),
                description: "Trading data list".into(),
            },
            CommandInfo {
                name: "/trading data show".into(),
                description: "Trading data show".into(),
            },
            CommandInfo {
                name: "/trading data ingest-ohlcv".into(),
                description: "Trading data ingest OHLCV".into(),
            },
            CommandInfo {
                name: "/trading data validate".into(),
                description: "Trading data validate".into(),
            },
            CommandInfo {
                name: "/trading data providers".into(),
                description: "Trading data providers".into(),
            },
            CommandInfo {
                name: "/trading data capability".into(),
                description: "Trading data capability".into(),
            },
            CommandInfo {
                name: "/trading data fetch-native".into(),
                description: "Trading data fetch native".into(),
            },
            CommandInfo {
                name: "/trading data snapshot".into(),
                description: "Trading data snapshot".into(),
            },
            CommandInfo {
                name: "/trading data coverage".into(),
                description: "Trading data coverage".into(),
            },
            CommandInfo {
                name: "/trading data export".into(),
                description: "Trading data export".into(),
            },
            CommandInfo {
                name: "/trading data list --target /tmp/project --json".into(),
                description: "Trading data list target alias".into(),
            },
            CommandInfo {
                name: "/trading data export --target /tmp/project --dataset-id btc-1d --version v1 --out bars.json".into(),
                description: "Trading data export target alias".into(),
            },
            CommandInfo {
                name: "/trading data export-ohlcv".into(),
                description: "Trading data export OHLCV alias".into(),
            },
            CommandInfo {
                name: "/trading data providers --json".into(),
                description: "Trading provider routing evidence".into(),
            },
            CommandInfo {
                name:
                    "/trading data capability --provider openbb --symbol SPY --timeframe 1D --json"
                        .into(),
                description: "Trading provider capability probe".into(),
            },
            CommandInfo {
                name: "/trading data snapshot --provider tradingview --symbol ES".into(),
                description: "Trading snapshot artifact route".into(),
            },
            CommandInfo {
                name: "/trading data fetch-native --provider polygon".into(),
                description: "Trading data fetch native Polygon/OpenBB path".into(),
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

    #[test]
    fn filter_trading_data_aliases() {
        let catalog = make_catalog();
        let results = filter(&catalog, "/trading data");
        let names: Vec<_> = results.iter().map(|cmd| cmd.name.as_str()).collect();
        assert!(names.contains(&"/trading data status"));
        assert!(names.contains(&"/trading data list"));
        assert!(names.contains(&"/trading data show"));
        assert!(names.contains(&"/trading data ingest-ohlcv"));
        assert!(names.contains(&"/trading data validate"));
        assert!(names.contains(&"/trading data providers"));
        assert!(names.contains(&"/trading data capability"));
        assert!(names.contains(&"/trading data fetch-native"));
        assert!(names.contains(&"/trading data snapshot"));
        assert!(names.contains(&"/trading data coverage"));
        assert!(names.contains(&"/trading data export"));
        assert!(names.contains(&"/trading data export-ohlcv"));
        assert!(names.contains(&"/trading data list --target /tmp/project --json"));
        assert!(names.contains(
            &"/trading data export --target /tmp/project --dataset-id btc-1d --version v1 --out bars.json"
        ));
        assert!(
            names
                .iter()
                .any(|name| name.contains("data export --target") && name.contains("--out"))
        );
        assert!(names.contains(&"/trading data fetch-native --provider polygon"));
        assert!(names.contains(&"/trading data providers --json"));
        assert!(
            names
                .iter()
                .any(|name| name.contains("capability --provider openbb"))
        );
        assert!(
            names
                .iter()
                .any(|name| name.contains("snapshot --provider tradingview"))
        );
    }
}

use super::*;

#[test]
fn default_registry_contains_all_commands() {
    let registry = default_registry();
    assert_eq!(
        registry.len(),
        EXPECTED_COMMAND_COUNT,
        "default_registry must register every pre-TASK-AGS-622 slash command"
    );
}

#[test]
fn primaries_with_descriptions_count_matches_expected() {
    let registry = default_registry();
    let catalog = registry.primaries_with_descriptions();
    assert_eq!(
        catalog.len(),
        EXPECTED_COMMAND_COUNT,
        "primaries_with_descriptions().len() must equal EXPECTED_COMMAND_COUNT — \
             catalog↔registry lockstep invariant. If you added/removed a primary, \
             update EXPECTED_COMMAND_COUNT."
    );
}

#[test]
fn default_registry_includes_fast() {
    assert!(default_registry().get("fast").is_some());
}

#[test]
fn default_registry_includes_help() {
    assert!(default_registry().get("help").is_some());
}

#[test]
fn default_registry_includes_config() {
    assert!(default_registry().get("config").is_some());
}

#[test]
fn default_registry_includes_rules() {
    assert!(default_registry().get("rules").is_some());
}

#[test]
fn default_registry_includes_thinking() {
    assert!(default_registry().get("thinking").is_some());
}

#[test]
fn default_registry_includes_compact_and_clear_separately() {
    let registry = default_registry();
    assert!(registry.get("compact").is_some());
    assert!(registry.get("clear").is_some());
}

#[test]
fn unknown_command_returns_none() {
    assert!(default_registry().get("nonexistent").is_none());
}

#[test]
fn handler_description_is_non_empty() {
    let registry = default_registry();
    let handler = registry.get("fast").expect("fast handler registered");
    assert!(!handler.description().is_empty());
}

#[test]
fn registry_lookup_returns_arc() {
    let registry = default_registry();
    let first = registry.get("fast");
    let second = registry.get("fast");
    assert!(first.is_some());
    assert!(second.is_some());
}

/// Handler with no alias override — exercises the default empty-slice
/// implementation on the trait.
struct NoAliasHandler;
impl CommandHandler for NoAliasHandler {
    fn execute(&self, _ctx: &mut CommandContext, _args: &[String]) -> anyhow::Result<()> {
        Ok(())
    }
    fn description(&self) -> &str {
        "no-alias handler (test only)"
    }
}

#[test]
fn aliases_method_default_empty() {
    // A handler that does NOT override aliases() returns &[].
    let h = NoAliasHandler;
    assert_eq!(h.aliases(), &[] as &[&'static str]);
}

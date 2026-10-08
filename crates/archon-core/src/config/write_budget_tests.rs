use super::{ArchonConfig, write_call_time_budget_warnings};

fn config_with_budget(host_call_secs: u32, write_budget_secs: u32) -> ArchonConfig {
    let mut config = ArchonConfig::default();
    config.workflow.generated.host_call_timeout_secs = host_call_secs;
    config.workflow.generated.write_call_time_budget_secs = write_budget_secs;
    config
}

#[test]
fn equal_host_and_write_budgets_warn_with_both_keys_values_and_default() {
    let warnings = write_call_time_budget_warnings(&config_with_budget(28_800, 28_800));

    assert_eq!(warnings.len(), 1, "{warnings:?}");
    let warning = &warnings[0];
    for expected in [
        "workflow.generated.write_call_time_budget_secs",
        "28800",
        "workflow.generated.host_call_timeout_secs",
        "3 ×",
        "86400",
    ] {
        assert!(
            warning.contains(expected),
            "missing {expected:?}: {warning}"
        );
    }
}

#[test]
fn twice_the_host_timeout_does_not_warn() {
    assert!(write_call_time_budget_warnings(&config_with_budget(28_800, 57_600)).is_empty());
}

#[test]
fn less_than_one_host_timeout_warns_with_each_configured_value() {
    let warnings = write_call_time_budget_warnings(&config_with_budget(28_800, 20_000));

    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(warnings[0].contains("= 20000s"), "{:?}", warnings[0]);
    assert!(warnings[0].contains("= 28800s"), "{:?}", warnings[0]);
}

#[test]
fn unset_write_budget_uses_default_and_does_not_warn() {
    assert!(write_call_time_budget_warnings(&ArchonConfig::default()).is_empty());
}

use super::*;

#[test]
fn native_policy_is_strict_and_opt_in() {
    let empty: archon_core::config::ArchonConfig=toml::from_str("").unwrap();
    assert!(empty.workflow.acceptance_execution.is_none());
    assert!(toml::from_str::<archon_core::config::ArchonConfig>(
        "[workflow.acceptance_execution]\nunknown_authority=true\n").is_err());
}

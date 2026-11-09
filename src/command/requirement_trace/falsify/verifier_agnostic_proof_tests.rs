//! Issue #631: assertions describe the correct language-neutral behavior.
use super::*;
#[path = "../../../../tests/fixtures/lang_projects/support.rs"]
mod fixture;

#[test]
fn r5_h4_unscoped_go_falsification_refused() {
    assert!(fixture::fixture("go").is_dir());
    let config: archon_core::config::ArchonConfig =
        toml::from_str("[workflow.verify]\nrefuse_tokens = ['./...']").unwrap();
    assert!(
        vet(
            "go test ./...",
            &VerifyLimits::from(&config.workflow.verify)
        )
        .is_err(),
        "workspace-wide Go verifier admitted for baseline and mutant runs"
    );
}

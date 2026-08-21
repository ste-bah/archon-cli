use super::*;

#[test]
fn an_unchanged_file_proceeds() {
    assert_eq!(
        decide_write_claim("src/a.rs", Some("abc123"), Some("abc123")),
        WriteClaimDecision::Proceed
    );
}

/// The live case: identity.rs moved between baseline and write. Catching it
/// here costs one re-read; catching it at apply time cost a whole turn.
#[test]
fn a_file_that_moved_since_baseline_is_flagged_before_the_edit() {
    let decision = decide_write_claim(
        "crates/archon-trading/src/data_lake/identity.rs",
        Some("02b342b14"),
        Some("d7d5036ae"),
    );

    assert!(!decision.should_proceed());
    let guidance = decision.guidance().expect("guidance");
    assert!(guidance.contains("identity.rs"), "{guidance}");
    assert!(guidance.contains("Re-read"), "{guidance}");
}

/// A brand-new file exists at neither end and is free to write — this is the
/// case a claim taken up front cannot predict at all.
#[test]
fn a_file_that_never_existed_proceeds() {
    assert_eq!(
        decide_write_claim("src/new.rs", None, None),
        WriteClaimDecision::Proceed
    );
}

/// Created since baseline is divergence: the agent believes it is creating a
/// file that now exists, so its patch will not apply.
#[test]
fn a_file_created_since_baseline_is_flagged() {
    let decision = decide_write_claim("src/new.rs", None, Some("abc123"));
    assert!(!decision.should_proceed());
    assert_eq!(
        decision,
        WriteClaimDecision::Restale {
            path: "src/new.rs".to_string(),
            baseline_digest: "<absent>".to_string(),
            current_digest: "abc123".to_string(),
        }
    );
}

/// Deleted since baseline is divergence in the other direction.
#[test]
fn a_file_deleted_since_baseline_is_flagged() {
    let decision = decide_write_claim("src/gone.rs", Some("abc123"), None);
    assert!(!decision.should_proceed());
    assert!(decision.guidance().is_some());
}

/// Proceed carries no guidance — the gate must be silent when it has nothing
/// to say, or every write turns into a warning the agent learns to ignore.
#[test]
fn proceeding_says_nothing() {
    assert!(
        decide_write_claim("src/a.rs", Some("x"), Some("x"))
            .guidance()
            .is_none()
    );
}

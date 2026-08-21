use super::*;

/// The live loop. Triage classified all twenty-four outcomes as
/// `retry_resolved_by_sibling_evidence` and returned no implementation
/// failures. That is a decision, not a missing answer.
#[test]
fn a_triage_that_routed_everything_elsewhere_has_classified() {
    let routes = VerificationTriageRoutes {
        implementation_failures: Vec::new(),
        retry_items: vec![serde_json::json!({
            "item_id": "verify-1",
            "classification": "retry_resolved_by_sibling_evidence",
        })],
        superseded_items: Vec::new(),
        terminal_blockers: Vec::new(),
    };

    assert!(triage_classified_any(&routes));
}

/// A triage that returned nothing at all is a shape failure, and the caller is
/// right to fall back on the actionable set.
#[test]
fn a_silent_triage_has_classified_nothing() {
    assert!(!triage_classified_any(&VerificationTriageRoutes::default()));
}

/// Any populated route counts, not just implementation failures.
#[test]
fn every_route_counts_as_a_classification() {
    for routes in [
        VerificationTriageRoutes {
            implementation_failures: vec![serde_json::json!({ "item_id": "impl" })],
            ..Default::default()
        },
        VerificationTriageRoutes {
            superseded_items: vec![serde_json::json!({ "item_id": "superseded" })],
            ..Default::default()
        },
        VerificationTriageRoutes {
            terminal_blockers: vec![serde_json::json!({ "item_id": "blocker" })],
            ..Default::default()
        },
    ] {
        assert!(triage_classified_any(&routes), "{routes:?}");
    }
}

/// With the verdict preserved, an empty inventory is "nothing to do" rather
/// than "not ready", so the loop exits instead of regenerating forever.
#[test]
fn no_implementation_failures_means_remediation_is_not_needed() {
    let routes = VerificationTriageRoutes {
        implementation_failures: Vec::new(),
        retry_items: vec![serde_json::json!({
            "item_id": "verify-1",
            "classification": "retry_resolved_by_sibling_evidence",
        })],
        superseded_items: Vec::new(),
        terminal_blockers: Vec::new(),
    };
    let plan = triage_route_plan(&routes);

    assert_eq!(
        remediation_inventory_route(&plan, false),
        RemediationInventoryRoute::NotNeeded,
        "an empty inventory must not regenerate when no write was requested"
    );
}

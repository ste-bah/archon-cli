use super::*;

fn is_check(error: &str) -> bool {
    matches!(check_fault(error), CheckFault::Check { .. })
}

#[test]
fn what_the_checks_own_run_caused_is_the_checks() {
    for error in [
        OUTPUT_LIMIT_EXCEEDED.to_string(),
        SCRATCH_LIMIT_EXCEEDED.to_string(),
        PIPES_HELD.to_string(),
        format!("{LEFT_BEHIND}: pid 7 (/scratch/tmp/held)"),
        identity_changed(&["source"]),
        identity_changed(&["target", "target links", "build configuration"]),
        format!("invalid spec: {TARGET_MAPPING_CHANGED}"),
    ] {
        assert!(is_check(&error), "{error}");
    }
}

#[test]
fn a_teardown_the_host_could_not_verify_is_ambiguous() {
    let error = format!("{TEARDOWN_UNVERIFIED} (still alive: 7, 8)");
    assert_eq!(check_fault(&error), CheckFault::Ambiguous);
}

#[test]
fn what_the_host_caused_or_could_not_observe_is_the_hosts() {
    for error in [
        "scratch size audit failed: permission denied".to_string(),
        "scratch holder probe could not finish in time".to_string(),
        "observation parent closed or cancellation requested".to_string(),
        "scratch teardown exceeded its deadline; survivors unknown".to_string(),
        identity_changed(&["tools"]),
        identity_changed(&["source", "environment"]),
        "native scratch source or build identity could not be captured after the check (io error); warm target reuse refused".to_string(),
        String::new(),
    ] {
        assert_eq!(check_fault(&error), CheckFault::Host, "{error}");
    }
}

use super::{run, site};

#[cfg(unix)]
#[test]
fn holder_probe_failure_is_a_host_error() {
    let mut error = None;
    let mut fault = None;
    super::super::record_holder_audit(
        super::super::confine::HolderAudit::HostFailure(
            "scratch holder probe could not finish in time".into(),
        ),
        &mut error,
        &mut fault,
    );
    assert_eq!(
        error.as_deref(),
        Some("scratch holder probe could not finish in time")
    );
    assert_eq!(fault, None);
}

#[cfg(unix)]
#[test]
fn teardown_stall_is_typed_only_when_its_evidence_has_the_unverified_prefix() {
    use super::super::super::faults::{FaultCause, TEARDOWN_UNVERIFIED};
    assert_eq!(
        super::super::finish::teardown_fault(&format!("{TEARDOWN_UNVERIFIED}: member survived")),
        Some(FaultCause::TeardownUnverified)
    );
    assert_eq!(
        super::super::finish::teardown_fault("scratch teardown exceeded its deadline"),
        None
    );
}

#[tokio::test]
async fn process_limits_record_typed_causes_when_detected() {
    let output_root = tempfile::tempdir().unwrap();
    let mut output_site = site(output_root.path(), true, 20);
    output_site.output_bytes = 2;
    let output = run(&output_site, "printf 12345").await;
    assert_eq!(
        output.fault,
        Some(super::super::super::faults::FaultCause::OutputLimitExceeded),
        "{output:?}"
    );
    assert!(matches!(
        super::super::super::faults::classify(output.fault.as_ref(), "unrelated host message"),
        super::super::super::faults::CheckFault::Check { .. }
    ));

    let scratch_root = tempfile::tempdir().unwrap();
    let mut scratch_site = site(scratch_root.path(), true, 20);
    scratch_site.scratch_bytes = 3;
    let scratch = run(&scratch_site, "printf 12345 > large").await;
    assert_eq!(
        scratch.fault,
        Some(super::super::super::faults::FaultCause::ScratchLimitExceeded),
        "{scratch:?}"
    );
    assert!(matches!(
        super::super::super::faults::classify(scratch.fault.as_ref(), "unrelated host message"),
        super::super::super::faults::CheckFault::Check { .. }
    ));
}

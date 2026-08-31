//! Staged-path invariants for `workflow freeze-*` (child module via #[path];
//! file-size guard).

use super::*;

#[test]
fn both_json_staged_paths_report_operational_failure_through_the_envelope() {
    let whole = include_str!("workflow_freeze_cli.rs");
    let source = &whole[..whole.find("#[cfg(test)]").expect("test module marker")];
    for command in ["freeze-acceptance", "freeze-skeleton"] {
        let reported = source
            .split("return report_operational_failure(")
            .skip(1)
            .any(|block| block[..block.len().min(120)].contains(command));
        assert!(
            reported,
            "{command} must surface an operational failure as the envelope's reason"
        );
    }
    assert_eq!(
        source.matches("return report_operational_failure(").count(),
        2,
        "both staged paths report operationally rather than exiting non-zero"
    );
}

#[test]
fn both_json_staged_paths_refuse_the_candidate_instead_of_failing_the_run() {
    let whole = include_str!("workflow_freeze_cli.rs");
    // Only the production half counts: this module's own literals would
    // otherwise satisfy the assertion about the code it is checking.
    let source = &whole[..whole.find("#[cfg(test)]").expect("test module marker")];
    for (command, gate) in [
        ("freeze-acceptance", "GateId::FreezeAcceptance"),
        ("freeze-skeleton", "GateId::FreezeSkeleton"),
    ] {
        let refused = source
            .split("return refuse_candidate_artifact(")
            .skip(1)
            .any(|block| {
                let head = &block[..block.len().min(400)];
                head.contains(command) && head.contains(gate)
            });
        assert!(
            refused,
            "{command} must refuse a malformed candidate through the findings channel"
        );
    }
    // Acceptance refuses twice — once for a candidate that will not parse and
    // once for one the freeze itself rejects — so the total is a floor, not
    // a fixed number. What must hold is that no candidate problem leaves by
    // any other exit.
    assert!(
        source.matches("return refuse_candidate_artifact(").count() >= 2,
        "every JSON staged path routes candidate problems through the findings channel"
    );
}

/// A staged freeze must leave its findings readable on disk.
///
/// The staged path is what the decomposition drives through `hostCommand`, and
/// unlike the interactive path it never calls `run_sync_gate`, so nothing wrote
/// shadow records for it. The lock a staged freeze publishes carries only
/// `finding_count` and `findings_digest`, so a run could report "1 finding" and
/// make it permanently unreadable — which is exactly what happened to the
/// acceptance finding that a week of proof runs was then built on top of.
#[test]
fn a_staged_freeze_records_its_findings_where_a_human_can_read_them() {
    let temp = tempfile::tempdir().expect("tempdir");
    let cwd = temp.path();
    let staging_root = cwd.join("staging");
    std::fs::create_dir_all(&staging_root).expect("staging root");
    let gate_envelope = staging_root.join("envelope.json");

    let finding = crate::command::workflow_gate::GateFinding::new(
        crate::command::workflow_gate::GateId::FreezeAcceptance,
        "check 'AC-X-001' floor is not falsifiable: deliverable contract has neither a verifier nor a positive instance obligation",
        "AC-X-001",
        None,
        archon_workflow::RemediationScope::CandidateArtifact,
    );
    let evaluation =
        crate::command::workflow_gate::GateEvaluation::new("staged", vec![finding]);

    write_staged_manifest(
        cwd,
        StagedArgs {
            staging_root: &staging_root,
            gate_envelope: &gate_envelope,
            call_id: "call-1",
        },
        "freeze-acceptance",
        evaluation,
        Vec::new(),
    )
    .expect("staged manifest");

    let log = crate::command::workflow_gate::shadow_log_path(cwd);
    let text = std::fs::read_to_string(&log).unwrap_or_else(|error| {
        panic!(
            "a staged freeze must write its findings to {}: {error}",
            log.display()
        )
    });
    assert!(
        text.contains("floor is not falsifiable"),
        "the shadow record must carry the finding text, not just a count: {text}"
    );
    assert!(
        text.contains("freeze_acceptance"),
        "the record must name the gate that produced it: {text}"
    );
}

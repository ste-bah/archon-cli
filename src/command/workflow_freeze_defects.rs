//! Preserve the complete host-owned structured refusal list across freeze staging.
use super::*;
use crate::command::workflow_task_set_candidate::CandidateDefects;

pub(super) fn refuse_candidate_error(
    cwd: &Path,
    staged: StagedArgs<'_>,
    command_id: &str,
    gate_id: crate::command::workflow_gate::GateId,
    subject: &str,
    error: &anyhow::Error,
) -> Result<()> {
    let defects = if let Some(error) =
        error.downcast_ref::<archon_workflow::task_skeleton::TaskSkeletonError>()
    {
        &error.defects
    } else if let Some(error) =
        error.downcast_ref::<archon_workflow::task_set_contract::TaskSetContractError>()
    {
        &error.defects
    } else if let Some(error) = error.downcast_ref::<CandidateDefects>() {
        &error.0
    } else {
        return refuse_candidate_artifact(
            cwd,
            staged,
            command_id,
            gate_id,
            subject,
            &format!("{error:#}"),
        );
    };
    if defects.is_empty() {
        return refuse_candidate_artifact(
            cwd,
            staged,
            command_id,
            gate_id,
            subject,
            &format!("{error:#}"),
        );
    }
    let findings = defects
        .iter()
        .map(|defect| {
            crate::command::workflow_gate::GateFinding::new(
                gate_id,
                format!("candidate artifact was refused: {}", defect.message),
                subject,
                None,
                archon_workflow::RemediationScope::CandidateArtifact,
            )
            .with_defect(defect.identity.clone())
        })
        .collect();
    write_staged_manifest(
        cwd,
        staged,
        command_id,
        crate::command::workflow_gate::GateEvaluation::new(
            "candidate refused before staging",
            findings,
        ),
        Vec::new(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    // Guard for the new wire contract: the validator completeness regression
    // and author-loop 70->0 regression each fail independently on the old code.
    #[test]
    fn workflow_host_command_complete_refusal_envelopes_drive_seventy_repairs() {
        use archon_workflow::task_skeleton::{FrozenTask, TaskSkeleton, validate_skeleton};
        let temp = tempfile::tempdir().expect("fixture");
        let mut skeleton = TaskSkeleton {
            schema_version: 1,
            acceptance_digest: "d".into(),
            tasks: (1..=70)
                .map(|n| FrozenTask {
                    task_id: format!("TASK-X-{n:03}"),
                    file_name: format!("bad{n}"),
                    depends_on: vec![],
                    blocks: vec![],
                    implements: vec![],
                    deliverable_contracts: vec![],
                })
                .collect(),
        };
        let mut envelopes = Vec::new();
        for repaired in 0..70 {
            let error = validate_skeleton(&skeleton, "d").expect_err("remaining defects");
            let identities: Vec<_> = error
                .defects
                .iter()
                .map(|defect| defect.identity.clone())
                .collect();
            skeleton.tasks[repaired].file_name = format!("renamed-bad{repaired}");
            let renamed = validate_skeleton(&skeleton, "d").expect_err("renaming cannot fix");
            assert_eq!(
                identities,
                renamed
                    .defects
                    .iter()
                    .map(|defect| defect.identity.clone())
                    .collect::<Vec<_>>()
            );
            let tagged = crate::command::workflow_task_set_candidate::CandidateRejected::tag::<()>(
                Err(error.into()),
            )
            .expect_err("tag");
            let staging = temp.path().join(format!("stage-{repaired}"));
            std::fs::create_dir(&staging).expect("staging");
            let path = staging.join("gate.json");
            refuse_candidate_error(
                temp.path(),
                StagedArgs {
                    staging_root: &staging,
                    gate_envelope: &path,
                    call_id: "call",
                },
                "freeze-skeleton",
                crate::command::workflow_gate::GateId::FreezeSkeleton,
                "skeleton",
                &tagged,
            )
            .expect("refusal");
            let envelope: serde_json::Value =
                serde_json::from_slice(&std::fs::read(&path).expect("bytes")).expect("envelope");
            let findings = envelope["policy_findings"].as_array().expect("findings");
            assert_eq!(findings.len(), 70 - repaired);
            for finding in findings {
                assert_eq!(
                    finding["deterministic_defect"]["provenance"],
                    "host_validator"
                );
                assert_eq!(finding["deterministic_defect"]["code"], "invalid_filename");
                assert!(!finding["deterministic_defect"].to_string().contains("bad"));
            }
            envelopes.push(envelope);
            let task = &mut skeleton.tasks[repaired];
            task.file_name = format!("{}.md", task.task_id);
        }
        assert!(validate_skeleton(&skeleton, "d").is_ok());
        envelopes.push(serde_json::json!({"policy_findings":[]}));
        let path = temp.path().join("envelopes.json");
        std::fs::write(&path, serde_json::to_vec(&envelopes).expect("json")).expect("snapshots");
        let output = std::process::Command::new("node")
            .arg(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/src/command/workflow_decompose_round6_test.cjs"
            ))
            .arg(&path)
            .output()
            .expect("node");
        assert!(
            output.status.success(),
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

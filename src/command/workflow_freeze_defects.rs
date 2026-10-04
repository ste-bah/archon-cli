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
            "candidate_refused",
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
            "candidate_refused",
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

/// Every element of `lists` that does not read as `E`, each its own shape
/// defect (Issue 261): serde stops at the first error of a whole document, so
/// four tasks each missing a field would otherwise surface one per attempt.
pub(crate) fn element_shape_defects<E: serde::de::DeserializeOwned>(
    candidate: &[u8],
    lists: &[&str],
) -> Vec<archon_workflow::defect::ValidationDefect> {
    let document = candidate_document(candidate);
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(&document) else {
        return Vec::new();
    };
    let mut defects = Vec::new();
    for list in lists {
        let items = value.get(*list).and_then(serde_json::Value::as_array);
        for (index, item) in items.into_iter().flatten().enumerate() {
            if let Err(error) = serde_json::from_value::<E>(item.clone()) {
                let slot = format!("{list}/{index}");
                let message = format!("{slot} does not match the required shape ({error})");
                defects.push(archon_workflow::defect::ValidationDefect::new(
                    "invalid_candidate_shape",
                    &slot,
                    "shape",
                    message,
                ));
            }
        }
    }
    defects
}

/// Refuses the candidate with every element shape defect, or `None` when
/// every element reads as `E`.
pub(super) fn refuse_element_shapes<E: serde::de::DeserializeOwned>(
    cwd: &Path,
    staged: StagedArgs<'_>,
    (command_id, gate_id, subject): (&str, crate::command::workflow_gate::GateId, &str),
    candidate: &[u8],
    lists: &[&str],
) -> Option<Result<()>> {
    let defects = element_shape_defects::<E>(candidate, lists);
    if defects.is_empty() {
        return None;
    }
    let error = anyhow::Error::new(CandidateDefects(defects));
    Some(refuse_candidate_error(
        cwd, staged, command_id, gate_id, subject, &error,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Issue 261 round 8: four tasks each missing `file_name` are four shape
    /// defects at once, not one serde error per attempt.
    #[test]
    fn workflow_host_command_every_misshapen_task_is_its_own_shape_defect() {
        let task = |n: usize, named: bool| {
            let mut task = serde_json::json!({"task_id": format!("TASK-X-{n:03}"), "depends_on": [],
                "blocks": [], "implements": [], "deliverable_contracts": []});
            if named {
                task["file_name"] = serde_json::json!(format!("TASK-X-{n:03}.md"));
            }
            task
        };
        for fixed in 0..=4 {
            let tasks: Vec<_> = (0..4).map(|n| task(n, n < fixed)).collect();
            let candidate =
                serde_json::json!({"schema_version": 1, "acceptance_digest": "d", "tasks": tasks});
            let defects = element_shape_defects::<archon_workflow::task_skeleton::FrozenTask>(
                &serde_json::to_vec(&candidate).expect("json"),
                &["tasks"],
            );
            assert_eq!(defects.len(), 4 - fixed, "{defects:?}");
            for defect in &defects {
                assert_eq!(defect.identity.code, "invalid_candidate_shape");
                assert!(defect.message.contains("file_name"), "{}", defect.message);
            }
        }
    }

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

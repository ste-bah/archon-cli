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

#[path = "workflow_freeze_shape.rs"]
mod shape;
pub(crate) use shape::{ENTRY_SHAPE, ElementShape, TASK_SHAPE, element_shape_defects};

/// Refuses the candidate with every element shape defect, or `None` when
/// every element has the required shape.
pub(super) fn refuse_element_shapes(
    cwd: &Path,
    staged: StagedArgs<'_>,
    (command_id, gate_id, subject): (&str, crate::command::workflow_gate::GateId, &str),
    candidate: &[u8],
    shape: &ElementShape,
) -> Option<Result<()>> {
    let defects = element_shape_defects(candidate, shape);
    if defects.is_empty() {
        return None;
    }
    let error = anyhow::Error::new(CandidateDefects(defects));
    Some(refuse_candidate_error(
        cwd, staged, command_id, gate_id, subject, &error,
    ))
}

#[cfg(test)]
#[path = "workflow_freeze_shape_tests.rs"]
mod shape_tests;

#[cfg(test)]
mod tests {
    use super::*;

    /// The real refusal envelope for `candidate` under `shape`.
    pub(super) fn envelope(
        dir: &Path,
        name: &str,
        candidate: &serde_json::Value,
        shape: &ElementShape,
    ) -> serde_json::Value {
        let staging = dir.join(name);
        std::fs::create_dir_all(&staging).expect("staging");
        let path = staging.join("gate.json");
        let staged = StagedArgs {
            staging_root: &staging,
            gate_envelope: &path,
            call_id: "call",
        };
        let gate = (
            "freeze-skeleton",
            crate::command::workflow_gate::GateId::FreezeSkeleton,
            "skeleton",
        );
        let bytes = serde_json::to_vec(candidate).expect("json");
        match refuse_element_shapes(dir, staged, gate, &bytes, shape) {
            None => serde_json::json!({ "policy_findings": [] }),
            Some(result) => {
                result.expect("refusal");
                serde_json::from_slice(&std::fs::read(&path).expect("bytes")).expect("envelope")
            }
        }
    }

    /// Feeds real envelopes, in order, to the fixed script's author loop: the
    /// loop must accept, never pause, while the host defects fall.
    pub(super) fn assert_author_loop_keeps_running(envelopes: &[serde_json::Value]) {
        let scenario = format!(
            "{{ findings: (n) => (({}[n - 1] || {{}}).policy_findings || []) }}",
            serde_json::to_string(envelopes).expect("json")
        );
        let out = crate::command::workflow_decompose::progress_tests::run(
            "enforce",
            &scenario,
            crate::command::workflow_decompose::progress_tests::BODY,
        );
        assert_eq!(out["accepted"], true, "{out}");
        assert_eq!(out["calls"], envelopes.len() as u64, "{out}");
    }

    /// Issue 261 round 9: every missing required field is its own identity,
    /// named by its JSON pointer, in the real envelope the freeze writes.
    #[test]
    fn workflow_host_command_each_missing_field_is_its_own_shape_identity() {
        let dir = tempfile::tempdir().expect("fixture");
        let fills = [
            ("task_id", serde_json::json!("TASK-X-001")),
            ("file_name", serde_json::json!("TASK-X-001.md")),
        ];
        let mut task = serde_json::json!({ "deliverable_contracts": [{}] });
        let mut envelopes = Vec::new();
        for step in 0..=4 {
            let candidate = serde_json::json!({ "schema_version": 1, "acceptance_digest": "d", "tasks": [task.clone()] });
            let value = envelope(dir.path(), &format!("step-{step}"), &candidate, &TASK_SHAPE);
            let subjects: std::collections::BTreeSet<_> = value["policy_findings"]
                .as_array()
                .expect("findings")
                .iter()
                .map(|finding| {
                    finding["deterministic_defect"]["subject"]
                        .as_str()
                        .expect("subject")
                        .to_string()
                })
                .collect();
            assert_eq!(subjects.len(), 4 - step, "{value}");
            for finding in value["policy_findings"].as_array().expect("findings") {
                assert_eq!(
                    finding["deterministic_defect"]["code"],
                    "invalid_candidate_shape"
                );
                assert_eq!(finding["deterministic_defect"]["stage"], "shape");
            }
            envelopes.push(value);
            match step {
                0 | 1 => task[fills[step].0] = fills[step].1.clone(),
                2 => task["deliverable_contracts"][0]["kind"] = serde_json::json!("file"),
                _ => {
                    task["deliverable_contracts"][0]["artifact_path"] =
                        serde_json::json!("out.json")
                }
            }
        }
        assert!(
            envelopes[0]
                .to_string()
                .contains("tasks/0/deliverable_contracts/0/kind")
        );
        assert_author_loop_keeps_running(&envelopes);
    }

    /// Issue 261 round 9: authored acceptance entries are checked field by
    /// field before assembly reads the entries vector whole.
    #[test]
    fn workflow_host_command_each_entry_missing_its_criterion_is_its_own_identity() {
        let dir = tempfile::tempdir().expect("fixture");
        let entry = |n: usize, complete: bool| {
            let mut entry = serde_json::json!({ "id": format!("AC-X-00{n}"),
                "check": { "kind": "command", "command": "true", "cwd": "project_root" } });
            if complete {
                entry["criterion"] = serde_json::json!("criterion");
            }
            entry
        };
        let mut envelopes = Vec::new();
        for repaired in 0..=4 {
            let entries: Vec<_> = (0..4).map(|n| entry(n, n < repaired)).collect();
            let candidate = serde_json::json!({ "entries": entries });
            let value = envelope(
                dir.path(),
                &format!("entries-{repaired}"),
                &candidate,
                &ENTRY_SHAPE,
            );
            assert_eq!(
                value["policy_findings"].as_array().expect("findings").len(),
                4 - repaired,
                "{value}"
            );
            envelopes.push(value);
        }
        assert_author_loop_keeps_running(&envelopes);
        let whole = include_str!("workflow_freeze_cli.rs");
        let staged = &whole[whole.find("async fn stage_acceptance(").expect("stage")..];
        let shape = staged.find("ENTRY_SHAPE").expect("entry shape check");
        let assembly = staged
            .find("acceptance_candidate_for_validation(")
            .expect("assembly");
        assert!(
            shape < assembly,
            "the entry shape check must run before assembly"
        );
    }

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
            let defects =
                element_shape_defects(&serde_json::to_vec(&candidate).expect("json"), &TASK_SHAPE);
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

//! Authoritative current postconditions and receipt checks for HostCommand reuse.

use std::path::PathBuf;

use archon_workflow::{
    CommandPostconditionEvaluation, HostCommandResult, HostCommandSubject, WorkflowError,
    WorkflowResult,
};

use super::workflow_host_command_catalog::HostCommandResolutionContext;

pub(super) fn read_acceptance_pin(
    context: &HostCommandResolutionContext,
) -> WorkflowResult<archon_workflow::task_set_contract::AcceptancePin> {
    let path =
        super::workflow_task_set::acceptance_pin_path(&context.project_root, &context.task_root);
    let bytes = std::fs::read(&path).map_err(|source| WorkflowError::Io {
        path: path.clone(),
        source,
    })?;
    serde_json::from_slice(&bytes).map_err(Into::into)
}

pub(super) fn evaluate_postcondition(
    context: &HostCommandResolutionContext,
    command_id: &str,
) -> WorkflowResult<(Vec<HostCommandSubject>, CommandPostconditionEvaluation)> {
    let pin = read_acceptance_pin(context)?;
    if command_id == "freeze-acceptance" {
        use archon_workflow::obligation_ids::acceptance_ids;
        use archon_workflow::task_set_contract::{
            ACCEPTANCE_CONTRACT_FILE, AcceptanceContract, content_digest,
            validate_acceptance_bundle,
        };
        let contract_path = context.task_root.join(ACCEPTANCE_CONTRACT_FILE);
        let contract: AcceptanceContract =
            serde_json::from_slice(&std::fs::read(&contract_path).map_err(|source| {
                WorkflowError::Io {
                    path: contract_path.clone(),
                    source,
                }
            })?)?;
        let prd = std::fs::read(&context.prd_path).map_err(|source| WorkflowError::Io {
            path: context.prd_path.clone(),
            source,
        })?;
        let expected = acceptance_ids(std::str::from_utf8(&prd).map_err(|error| {
            WorkflowError::SpecInvalid(format!("frozen PRD is not UTF-8: {error}"))
        })?);
        let satisfied = content_digest(&prd) == contract.prd.digest
            && validate_acceptance_bundle(&context.task_root, Some(&pin), &expected).is_ok();
        return Ok((
            Vec::new(),
            CommandPostconditionEvaluation {
                satisfied,
                summary: "acceptance contract, lock, PRD digest, and host pin evaluated".into(),
            },
        ));
    }
    let skeleton = archon_workflow::task_skeleton::validate_full_chain(&context.task_root, &pin)
        .map_err(|error| WorkflowError::SpecInvalid(error.to_string()))?;
    if command_id == "freeze-skeleton" {
        return Ok((
            skeleton
                .tasks
                .iter()
                .map(|task| HostCommandSubject {
                    task_id: task.task_id.clone(),
                    file_name: task.file_name.clone(),
                })
                .collect(),
            CommandPostconditionEvaluation {
                satisfied: true,
                summary: "authoritative frozen skeleton chain evaluated".into(),
            },
        ));
    }
    if command_id == "land-task-body" {
        let path = context.frozen_task_file.as_ref().ok_or_else(|| {
            WorkflowError::SpecInvalid("land-task-body has no frozen task file".to_string())
        })?;
        let raw = std::fs::read_to_string(path).map_err(|source| WorkflowError::Io {
            path: path.clone(),
            source,
        })?;
        let task = archon_workflow::task_universe::parsing::parse_task_file(path, &raw)?;
        let frozen = skeleton
            .tasks
            .iter()
            .find(|frozen| frozen.task_id == task.canonical_task_id)
            .ok_or_else(|| {
                WorkflowError::SpecInvalid(format!(
                    "published body {} is absent from frozen skeleton",
                    task.canonical_task_id
                ))
            })?;
        let satisfied =
            archon_workflow::task_skeleton::compare_frozen_task(&task, frozen).is_empty();
        return Ok((
            vec![HostCommandSubject {
                task_id: frozen.task_id.clone(),
                file_name: frozen.file_name.clone(),
            }],
            CommandPostconditionEvaluation {
                satisfied,
                summary: "authoritative landed body postcondition evaluated".into(),
            },
        ));
    }
    let mut tasks = Vec::new();
    for path in archon_workflow::task_universe::task_files_under(&context.task_root)? {
        let raw = std::fs::read_to_string(&path).map_err(|source| WorkflowError::Io {
            path: path.clone(),
            source,
        })?;
        tasks.push(archon_workflow::task_universe::parsing::parse_task_file(
            &path, &raw,
        )?);
    }
    let satisfied = archon_workflow::task_skeleton::compare_task_set(&tasks, &skeleton).is_empty();
    Ok((
        skeleton
            .tasks
            .iter()
            .map(|task| HostCommandSubject {
                task_id: task.task_id.clone(),
                file_name: task.file_name.clone(),
            })
            .collect(),
        CommandPostconditionEvaluation {
            satisfied,
            summary: format!("authoritative {command_id} postcondition evaluated"),
        },
    ))
}

pub(super) fn receipt_matches_live(
    receipt: Option<&archon_workflow::PublicationReceiptV1>,
) -> WorkflowResult<bool> {
    let Some(receipt) = receipt else {
        return Ok(false);
    };
    for entry in &receipt.entries {
        let path = PathBuf::from(&entry.destination_path);
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(source) => return Err(WorkflowError::Io { path, source }),
        };
        if bytes.len() as u64 != entry.byte_len
            || archon_workflow::task_set_contract::content_digest(&bytes) != entry.blake3
        {
            return Ok(false);
        }
    }
    Ok(true)
}

pub(super) fn fixed_subject_is_terminal(
    run_root: &PathBuf,
    command_id: &str,
    outcome: &HostCommandResult,
) -> WorkflowResult<bool> {
    let path = run_root.join(crate::command::workflow_decompose_state::FIXED_STATE_PATH);
    if !path.exists() {
        return Ok(true);
    }
    let state: archon_workflow::FixedDecompositionStateV1 =
        serde_json::from_slice(&std::fs::read(&path).map_err(|source| WorkflowError::Io {
            path: path.clone(),
            source,
        })?)?;
    let subject = match command_id {
        "freeze-acceptance" => "acceptance",
        "freeze-skeleton" => "skeleton",
        "land-task-body" => outcome
            .subjects
            .first()
            .map(|subject| subject.task_id.as_str())
            .unwrap_or("body"),
        other => other,
    };
    Ok(matches!(
        state.dispositions.get(subject),
        Some(
            archon_workflow::SubjectDisposition::Accepted
                | archon_workflow::SubjectDisposition::AcceptedWithShadowFindings
        )
    ))
}

use super::*;
pub(super) async fn stage_skeleton(
    cwd: &Path,
    tasks: &Path,
    prd: &Path,
    config: &ArchonConfig,
    env_vars: &ArchonEnvVars,
    staged: StagedArgs<'_>,
) -> Result<()> {
    let gate = (
        "freeze-skeleton",
        crate::command::workflow_gate::GateId::FreezeSkeleton,
        "skeleton",
    );
    let result = async {
        if config.workflow.gate_mode == archon_core::config::GateMode::Off {
            return Err(anyhow!(
                "gate_mode=off must return before staged skeleton preparation"
            ));
        }
        let resume = staged_freeze_resume(gate.0, &mut std::io::stderr());
        let candidate = read_bounded_stdin(
            archon_workflow::HostCommandRequest::MAX_STDIN_BYTES,
            staged.staging_root,
        )?;
        crate::command::workflow_freeze_candidate::record_candidate_extraction(
            &candidate,
            &mut std::io::stderr().lock(),
        )?;
        let tasks_root = absolute(cwd, tasks);
        let prd_path = absolute(cwd, prd);
        if let Some(refused) =
            defects::refuse_element_shapes(cwd, staged, gate, &candidate, &defects::TASK_SHAPE)
        {
            return refused;
        }
        let candidate = archon_workflow::task_skeleton::read_candidate(&candidate)
            .ok()
            .and_then(|v| serde_json::to_vec(&v).ok())
            .unwrap_or(candidate);
        if let Some((code, reason)) =
            candidate_refusal::<archon_workflow::task_skeleton::TaskSkeleton>(&candidate)
        {
            return refuse_candidate_artifact(
                cwd,
                staged,
                "freeze-skeleton",
                crate::command::workflow_gate::GateId::FreezeSkeleton,
                "skeleton",
                code,
                &reason,
            );
        }
        let prepared =
            match crate::command::workflow_task_set::prepare_skeleton_freeze_from_candidate_with_resume(
                cwd,
                &tasks_root,
                &prd_path,
                config.workflow.gate_mode,
                candidate_document(&candidate).into_owned(),
                &resume,
            ) {
                Ok(prepared) => prepared,
                Err(error)
                    if crate::command::workflow_task_set::CandidateRejected::caused(&error) =>
                {
                    return defects::refuse_candidate_error(
                        cwd,
                        staged,
                        "freeze-skeleton",
                        crate::command::workflow_gate::GateId::FreezeSkeleton,
                        "skeleton",
                        &error,
                    );
                }
                Err(error) => {
                    return report_operational_failure(cwd, staged, "freeze-skeleton", &error);
                }
            };
        let prepared = match skeleton_proof::prove_and_decide(
            cwd, config, env_vars, prepared, &resume,
        ).await {
            Ok(p) => p,
            Err(error) => {
                if let Some(incomplete) =
                    crate::command::workflow_freeze_budget::FreezeIncomplete::caused(&error)
                {
                    exit_incomplete_resumable(incomplete);
                }
                return report_operational_failure(cwd, staged, "freeze-skeleton", &error);
            }
        };
        let (evaluation, outputs) = prepared.into_staged_parts();
        write_staged_manifest(cwd, staged, "freeze-skeleton", evaluation, outputs)
    }.await;
    result.map_err(crate::command::workflow_host_fault::child_failure)
}

pub(super) async fn freeze_skeleton(
    cwd: &Path,
    tasks: &Path,
    prd: &Path,
    config: &ArchonConfig,
    env_vars: &ArchonEnvVars,
) -> Result<()> {
    if config.workflow.gate_mode == archon_core::config::GateMode::Off {
        print!("{}", crate::command::workflow_gate::OFF_MESSAGE);
        return Ok(());
    }
    let tasks_root = absolute(cwd, tasks);
    let prd_path = absolute(cwd, prd);
    let prepared = crate::command::workflow_task_set::prepare_skeleton_freeze(
        cwd,
        &tasks_root,
        &prd_path,
        config.workflow.gate_mode,
    )?;
    let prepared = skeleton_proof::prove_and_decide(
        cwd,
        config,
        env_vars,
        prepared,
        &crate::command::workflow_freeze_budget::FreezeResume::none(),
    )
    .await?;
    let findings = prepared.findings.clone();
    let publication_identity = prepared.publication_identity();
    let mut disposition = crate::command::workflow_gate::run_sync_gate(
        cwd,
        config.workflow.gate_mode,
        crate::command::workflow_gate::GateId::FreezeSkeleton,
        || {
            Ok(
                crate::command::workflow_gate::GateEvaluation::new("", findings)
                    .with_publication_identity(publication_identity),
            )
        },
    )?;
    for diagnostic in disposition.diagnostics() {
        eprintln!("{diagnostic}");
    }
    disposition.require_allowed()?;
    let permit = disposition
        .take_publication_permit()
        .ok_or_else(|| anyhow!("skeleton freeze received no publication permit"))?;
    let result = crate::command::workflow_task_set::publish_skeleton_freeze(prepared, permit)?;
    println!(
        "task skeleton frozen: digest={} acceptance_digest={}",
        result.skeleton_digest, result.acceptance_digest
    );
    Ok(())
}

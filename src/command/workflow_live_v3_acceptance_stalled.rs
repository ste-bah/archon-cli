//! Passability recheck for accepted checks that are still failing at a stall.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use archon_workflow::acceptance_scratch::CheckResult;
use archon_workflow::task_set_contract::{AcceptanceContract, JudgeDecision};
use archon_workflow::v2::acceptance_stage::{AcceptanceRoundRecordV1, progress::LoopDecision};
use archon_workflow::{WorkflowLlmClient, WorkflowResult, WorkflowStore};

use super::exec;
use super::output::Frozen;
use super::repair;
use super::result;
use super::{WorkflowV2ScriptRuntime, with_frozen_identity};
use crate::command::workflow_task_set::executability::BaselineRuns;
use crate::command::workflow_task_set::executability::{Baseline, CheckEvidence};

#[allow(clippy::too_many_arguments)]
pub(super) async fn on_pause(
    runtime: &WorkflowV2ScriptRuntime,
    store: &WorkflowStore,
    run_id: &str,
    owner: archon_workflow::control_pause::PauseOwner,
    task_universe: Option<&archon_workflow::task_universe::WorkflowV2TaskUniverse>,
    llm: Option<&dyn WorkflowLlmClient>,
    run_dir: &Path,
    record: &mut AcceptanceRoundRecordV1,
    frozen: &mut Option<Frozen>,
    record_path: &Path,
    decision: &LoopDecision,
    cause: &'static str,
) -> WorkflowResult<archon_workflow::WorkflowV2Result> {
    if apply(
        runtime,
        store,
        run_id,
        task_universe,
        llm,
        run_dir,
        record,
        frozen,
        decision.stalled_rounds as usize,
    )
    .await?
    {
        let record_path = super::relative_record_path(run_dir, record_path);
        let mut result = result::result_for(record, &record_path, decision);
        result::with_task_scope(&mut result, task_universe, record);
        return Ok(with_frozen_identity(record, frozen.as_ref(), result));
    }
    let record_path = super::relative_record_path(run_dir, record_path);
    Err(result::pause_on_stall(
        store,
        run_id,
        owner,
        record,
        &record_path,
        decision,
        cause,
    ))
}

/// Recheck current round output, then pass any refutation through the same
/// re-author gate and republish path used for other contract repairs.
#[allow(clippy::too_many_arguments)]
pub(super) async fn apply(
    runtime: &super::WorkflowV2ScriptRuntime,
    store: &WorkflowStore,
    run_id: &str,
    task_universe: Option<&archon_workflow::task_universe::WorkflowV2TaskUniverse>,
    llm: Option<&dyn WorkflowLlmClient>,
    run_dir: &Path,
    record: &mut AcceptanceRoundRecordV1,
    frozen: &mut Option<Frozen>,
    stalled_rounds: usize,
) -> WorkflowResult<bool> {
    let Some(client) = llm else {
        record
            .operational_errors
            .push("passability re-judge did not run: no language model client is available".into());
        return Ok(false);
    };
    let Some(current) = frozen.as_ref() else {
        record
            .operational_errors
            .push("passability re-judge did not run: no frozen contract is available".into());
        return Ok(false);
    };
    let contract = current.contract.clone();
    let failed: BTreeSet<String> = record
        .checks
        .iter()
        .filter(|check| check.ran_and_failed())
        .filter(|check| {
            entries(&contract).any(|entry| {
                entry.id == check.check_id && entry.judgment.verdict == JudgeDecision::Accepted
            })
        })
        .map(|check| check.check_id.clone())
        .collect();
    if failed.is_empty() {
        return Ok(false);
    }
    let Some(execution) = record.execution.as_ref() else {
        record
            .operational_errors
            .push("passability re-judge did not run: the round has no execution record".into());
        return Ok(false);
    };
    let context = match exec::resolve_context(
        store,
        run_id,
        runtime.target_repository_root.as_deref(),
        task_universe,
        runtime.cargo_limits.clone(),
    ) {
        Ok(context) => context,
        Err(error) => {
            record.operational_errors.push(format!("passability re-judge did not run: could not resolve the acceptance context: {error}"));
            return Ok(false);
        }
    };
    let attempt_dir =
        super::round_dir(run_dir, record.round).join(format!("attempt-{:02}", record.attempt));
    let mut failures = BTreeMap::new();
    for id in &failed {
        let Some(check) = record.checks.iter().find(|check| check.check_id == *id) else {
            continue;
        };
        let safe: String = id
            .chars()
            .map(|ch| {
                if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_') {
                    ch
                } else {
                    '_'
                }
            })
            .collect();
        let stdout = std::fs::read(attempt_dir.join(format!("{safe}.stdout")))
            .unwrap_or_else(|_| check.stdout_tail.as_bytes().to_vec());
        let stderr = std::fs::read(attempt_dir.join(format!("{safe}.stderr")))
            .unwrap_or_else(|_| check.stderr_tail.as_bytes().to_vec());
        failures.insert(
            id.clone(),
            CheckEvidence {
                result: CheckResult {
                    acceptance_id: id.clone(),
                    exit_code: check.exit_code,
                    quota_walk_count: 0,
                    stdout,
                    stderr,
                    environment_note: None,
                    operational_error: check.operational_error.clone(),
                    fault: None,
                    classification: None,
                },
                copy_layout: None,
            },
        );
    }
    if failures.is_empty() {
        return Ok(false);
    }
    let evidence_tree = execution
        .source_commit
        .clone()
        .unwrap_or_else(|| "working-tree".to_string());
    let runs = BaselineRuns {
        reuse: reuse_audit(&failures, "stalled round evidence"),
        commit: evidence_tree.clone(),
        failures,
        environment: BTreeMap::from([(
            "PATH".to_string(),
            std::env::var("PATH").unwrap_or_default(),
        )]),
        forwarded: Vec::new(),
    };
    let prd_path = if Path::new(&contract.prd.path).is_absolute() {
        Path::new(&contract.prd.path).to_path_buf()
    } else {
        context.project.join(&contract.prd.path)
    };
    let prd_text = std::fs::read_to_string(prd_path).unwrap_or_default();
    let Some(model) = contract
        .acceptance
        .iter()
        .chain(&contract.supplementary)
        .filter(|entry| entry.judgment.verdict == JudgeDecision::Accepted)
        .find_map(|entry| entry.judgment.sampling.as_ref()?.get("model")?.as_str())
    else {
        record.operational_errors.push(
            "passability re-judge verdicts: the frozen contract has no recorded judge model".into(),
        );
        return Ok(false);
    };
    let Some(base_commit) =
        Baseline::round_commit(&context.repository, &context.task_root, run_dir)
    else {
        record.operational_errors.push("passability re-judge did not run: no usable base commit is available to corroborate a stalled-check refutation".into());
        return Ok(false);
    };
    let probe = crate::command::workflow_task_set::executability::HostProbe::at_with_limits(
        context.project.clone(),
        context.repository.clone(),
        context.binding.clone(),
        context.cargo_limits.clone(),
    );
    let base_results = match probe
        .run_on_tree(
            &contract,
            &failed,
            &Baseline {
                commit: base_commit.clone(),
                repository: context.repository.clone(),
            },
        )
        .await
    {
        Ok(results) => results,
        Err(error) => {
            record.operational_errors.push(format!("passability re-judge did not run: could not run checks on base commit {base_commit}: {error}"));
            return Ok(false);
        }
    };
    let base_runs = BaselineRuns {
        reuse: reuse_audit(&base_results, "stalled check base-tree evidence"),
        commit: base_commit.clone(),
        failures: base_results,
        environment: runs.environment.clone(),
        forwarded: Vec::new(),
    };
    let audit_dir = crate::command::workflow_task_set::reauthor::cache::passability_audit_dir(
        &run_dir.join("v2"),
    );
    let silent_ids: BTreeSet<String> = failed
        .iter()
        .filter(|id| {
            let implemented = runs.failures.get(*id).map(|item| &item.result);
            let base = base_runs.failures.get(*id).map(|item| &item.result);
            [implemented, base].into_iter().all(|result| {
                result.is_some_and(|result| {
                    result.exit_code.is_some_and(|code| code != 0)
                        && String::from_utf8_lossy(&result.stdout).trim().is_empty()
                        && String::from_utf8_lossy(&result.stderr).trim().is_empty()
                })
            })
        })
        .cloned()
        .collect();
    let mut findings = BTreeMap::new();
    for id in &silent_ids {
        if let Some(finding) = crate::command::workflow_task_set::passability::silent_stall_finding(
            client,
            model,
            &contract,
            id,
            &base_runs,
            &prd_text,
            &audit_dir,
            stalled_rounds,
            &evidence_tree,
        )
        .map_err(|error| {
            archon_workflow::WorkflowError::SpecInvalid(format!(
                "recording silent stalled-check finding: {error:#}"
            ))
        })? {
            findings.insert(id.clone(), finding);
        }
    }
    let judge_ids: BTreeSet<String> = failed.difference(&silent_ids).cloned().collect();
    let current_findings = crate::command::workflow_task_set::passability::cannot_pass_findings(
        client,
        model,
        &context.repository,
        &context.project,
        &contract,
        &judge_ids,
        &runs,
        &prd_text,
        Some(&audit_dir),
    )
    .await;
    let current_findings = match current_findings {
        Ok(findings) => findings,
        Err(error) => {
            record.operational_errors.push(format!(
                "passability re-judge verdicts: could not complete the re-judge: {error:#}"
            ));
            return Ok(false);
        }
    };
    let base_findings = crate::command::workflow_task_set::passability::cannot_pass_findings(
        client,
        model,
        &context.repository,
        &context.project,
        &contract,
        &judge_ids,
        &base_runs,
        &prd_text,
        Some(&audit_dir),
    )
    .await;
    let base_findings = match base_findings {
        Ok(findings) => findings,
        Err(error) => {
            record.operational_errors.push(format!("passability re-judge did not run: could not corroborate findings on base commit {base_commit}: {error:#}"));
            return Ok(false);
        }
    };
    findings.extend(current_findings.into_iter().filter_map(|(id, finding)| {
        if base_findings.contains_key(&id) { Some((id, finding)) } else {
            record.operational_errors.push(format!("passability re-judge: check '{id}' was not corroborated on the base tree {base_commit}"));
            None
        }
    }));
    if findings.is_empty() {
        let verdicts = failed
            .iter()
            .filter_map(|id| {
                let (verdict, reason) =
                    crate::command::workflow_task_set::passability::audited_verdict(
                        client, model, &contract, id, &runs, &prd_text, &audit_dir,
                    )?;
                Some(format!("{id}: {verdict} — {reason}"))
            })
            .collect::<Vec<_>>();
        if !verdicts.is_empty() {
            record.operational_errors.push(format!(
                "passability re-judge verdicts: {}",
                verdicts.join("; ")
            ));
        } else {
            record.operational_errors.push(format!(
                "passability re-judge verdicts: {}",
                failed.iter().map(|id| format!("{id}: Accepted — neither the implemented nor base-tree evidence established a refutation")).collect::<Vec<_>>().join("; ")
            ));
        }
        return Ok(false);
    }
    let ids: BTreeSet<String> = findings.keys().cloned().collect();
    let site = probe.check_site(&contract);
    let mut originals = crate::command::workflow_task_set::executability::originals(
        &site,
        &contract,
        runs.failures.values().map(|e| e.result.clone()).collect(),
    )
    .await;
    for id in &ids {
        originals.insert(
            id.clone(),
            crate::command::workflow_task_set::executability::Original::Defect,
        );
    }
    let failed_tree = crate::command::workflow_task_set::executability::FailedTree {
        commit: None,
        originals,
    };
    let outcome = repair::republish(
        Some(client),
        &context,
        &contract,
        &ids,
        &findings,
        Some(&base_commit),
        Some(failed_tree),
    )
    .await;
    match outcome {
        Ok(reauthored) => {
            record.contract_repairs.push(
                archon_workflow::v2::acceptance_stage::AcceptanceContractRepairV1 {
                    check_ids: ids.iter().cloned().collect(),
                    trigger: "stalled accepted check refuted".into(),
                    repaired: reauthored.published,
                    freeze_event_id: reauthored.freeze_event_id,
                    failure: reauthored
                        .unproven
                        .values()
                        .chain(reauthored.authored_pending.values())
                        .cloned()
                        .collect::<Vec<_>>()
                        .join(" | "),
                    diagnostics: reauthored.diagnostics,
                },
            );
            if reauthored.published {
                match exec::load_contract(&context) {
                    Ok((updated, digest, _)) => {
                        *frozen = Some(Frozen {
                            contract: updated,
                            digest,
                        });
                    }
                    Err(error) => {
                        record
                            .operational_errors
                            .push(format!("re-authored contract is not usable: {error}"));
                        return Ok(false);
                    }
                }
                return Ok(true);
            }
            let failure = reauthored
                .unproven
                .values()
                .chain(reauthored.authored_pending.values())
                .cloned()
                .collect::<Vec<_>>()
                .join(" | ");
            record.operational_errors.push(format!(
                "stalled-check re-authoring did not publish a usable contract: {}",
                if failure.is_empty() {
                    "republish reported published=false".to_string()
                } else {
                    failure
                }
            ));
        }
        Err(error) => {
            let message = format!("stalled-check re-authoring failed to republish: {error:#}");
            record.operational_errors.push(message.clone());
            record.contract_repairs.push(
                archon_workflow::v2::acceptance_stage::AcceptanceContractRepairV1 {
                    check_ids: ids.iter().cloned().collect(),
                    trigger: "stalled accepted check refuted".into(),
                    repaired: false,
                    freeze_event_id: String::new(),
                    failure: format!("{error:#}"),
                    diagnostics: Vec::new(),
                },
            );
        }
    }
    Ok(false)
}

fn entries(
    contract: &AcceptanceContract,
) -> impl Iterator<Item = &archon_workflow::task_set_contract::AcceptanceCriterion> {
    contract.acceptance.iter().chain(&contract.supplementary)
}

fn reuse_audit(
    failures: &BTreeMap<String, CheckEvidence>,
    why: &str,
) -> BTreeMap<String, crate::command::workflow_task_set::executability::ReuseAudit> {
    failures
        .iter()
        .map(|(id, evidence)| {
            let input = serde_json::json!([
                id,
                evidence.result.exit_code,
                evidence.result.stdout,
                evidence.result.stderr,
                evidence.result.classification,
            ]);
            (
                id.clone(),
                crate::command::workflow_task_set::executability::ReuseAudit {
                    reused: true,
                    why: why.to_string(),
                    evidence: None,
                    input_key: Some(archon_workflow::task_set_contract::content_digest(
                        input.to_string().as_bytes(),
                    )),
                },
            )
        })
        .collect()
}

//! Proof composition shared by staged and manual publication.
use super::*;
use crate::command::workflow_freeze_budget::{FreezeIncomplete, FreezeResume};
use crate::command::workflow_task_set::{
    PreparedSkeletonFreeze,
    executability::{HostProbe, hermetic::breakage::breaker},
};
use archon_workflow::breakage_proof::{MutantKind, MutantRun};
use archon_workflow::breaker_verdict::{UnavailableCause, Verdict};
use archon_workflow::requirement_state::*;
use archon_workflow::task_set_contract::{
    ACCEPTANCE_CONTRACT_FILE, ACCEPTANCE_LOCK_FILE, AcceptanceContract, AcceptanceLock,
};
pub(crate) async fn prove_and_decide(
    cwd: &Path,
    config: &ArchonConfig,
    env: &ArchonEnvVars,
    prepared: PreparedSkeletonFreeze,
    resume: &FreezeResume,
) -> Result<PreparedSkeletonFreeze> {
    let blocked = if resume.persist {
        let envelope =
            crate::command::workflow_gate::GateEvaluation::new("", prepared.findings.clone())
                .into_envelope()?;
        crate::command::workflow_host_command_decision::candidate_findings_prevent_publication(
            "freeze-skeleton",
            config.workflow.gate_mode,
            &envelope,
        )
    } else {
        config.workflow.gate_mode == archon_core::config::GateMode::Enforce
            && !prepared.findings.is_empty()
    };
    if blocked {
        return Ok(prepared);
    }
    let Some(classes) = prepared.classification() else {
        return Ok(prepared);
    };
    let tasks = prepared.task_root();
    let contract: AcceptanceContract =
        serde_json::from_slice(&std::fs::read(tasks.join(ACCEPTANCE_CONTRACT_FILE))?)?;
    let lock: AcceptanceLock =
        serde_json::from_slice(&std::fs::read(tasks.join(ACCEPTANCE_LOCK_FILE))?)?;
    classification_ready(classes.error.as_ref())?;
    let text = std::fs::read_to_string(absolute(cwd, Path::new(&contract.prd.path)))?;
    let mut probe = HostProbe::for_task_set_with_limits(cwd, tasks, config.tools.cargo.to_limits())
        .with_resume(resume);
    let disabled = config.workflow.proof.max_runs_per_attempt == 0;
    // No candidate pairs means no provider construction or request.
    let prove = !disabled
        && classes.entries.iter().any(|e| {
            e.class == archon_workflow::v2::requirement_classes::RequirementClass::CandidateDone
                && !e.deliverables.iter().any(|d| d.error.is_some())
                && classes.base_snapshot.is_some()
        });
    let workspace = if prove {
        Some(probe.private_parent()?)
    } else {
        None
    };
    let report = if prove {
        let factory =
            crate::command::pipeline_workflow_llm::SubagentPipelineClientFactory::configured_only(
                config, env,
            )
            .without_project_tools();
        let client = factory
            .build_client(WorkflowLlmClientRequest {
                cwd: cwd.into(),
                origin: "proof".into(),
                session_id: uuid::Uuid::new_v4().to_string(),
                read_roots: workspace.iter().map(|p| p.path().to_path_buf()).collect(),
            })
            .await?;
        Some(
            breaker::prove_requirements(
                &probe,
                client.as_ref(),
                breaker::VerdictRequest {
                    contract: &contract,
                    chain_digest: &lock.digest,
                    lock: &lock,
                    classification: classes,
                    prd_text: &text,
                    max_runs: config.workflow.proof.max_runs_per_attempt,
                    min_deadline_secs: 60,
                    llm_deadline_secs: 120,
                    call_limits: config.workflow.breaker_calls.clone(),
                },
            )
            .await,
        )
    } else {
        None
    };
    let repository = if prove {
        crate::command::workflow_task_set::executability::diagnostic_repository(cwd, tasks)
            .map_err(anyhow::Error::msg)?
    } else {
        cwd.to_path_buf()
    };
    let outcome = decisions(
        classes,
        &contract,
        &lock,
        &text,
        disabled,
        report.as_ref(),
        (cwd, &repository),
    )?;
    if resume.persist
        && report
            .as_ref()
            .is_some_and(|r| r.verdicts.entries.iter().any(retry))
    {
        return Err(FreezeIncomplete::pending("proof work remains", &resume.progress).into());
    }
    prepared.with_decisions(&outcome, Vec::new())
}
fn retry(pair: &archon_workflow::breaker_verdict::PairVerdict) -> bool {
    pair.run_budget_exhausted
        || matches!(
            pair.verdict,
            Verdict::BreakerUnavailable | Verdict::JudgeUnavailable
        ) && unavailable_causes(pair).any(|c| c == UnavailableCause::Transient)
}
pub(crate) fn decisions(
    classes: &archon_workflow::v2::requirement_classes::RequirementClassification,
    contract: &AcceptanceContract,
    lock: &AcceptanceLock,
    text: &str,
    disabled: bool,
    report: Option<&breaker::VerdictReport>,
    roots: (&Path, &Path),
) -> Result<DecisionOutcome> {
    let (project, repository) = roots;
    let constraints = archon_workflow::obligation_ids::constraint_texts(text);
    let mut outcome = DecisionOutcome::default();
    for entry in &classes.entries {
        if let Some(error) = entry
            .deliverables
            .iter()
            .find_map(|d| d.error.as_deref())
            .or(classes.error.as_deref())
        {
            let mut decision =
                RequirementDecision::new(RequirementState::Undecided, "classification_unavailable");
            decision.notes.push(error.to_string());
            outcome.undecided.push(UndecidedNote {
                task_id: entry.task_id.clone(),
                id: entry.id.clone(),
                reason: format!("classification_unavailable: {error}"),
                survivors: Vec::new(),
                equivalence_unjudged: 0,
            });
            outcome
                .decisions
                .insert((entry.task_id.clone(), entry.id.clone()), decision);
            continue;
        }
        let pair = report.and_then(|r| {
            r.verdicts
                .entries
                .iter()
                .find(|p| p.task_id == entry.task_id && p.id == entry.id)
        });
        let requirement = archon_workflow::breaker_verdict::rule::requirement_text(text, &entry.id);
        let literal_proof = requirement
            .as_deref()
            .and_then(|t| text_proof(entry, contract, lock, t, project, repository));
        let breakage = report
            .zip(pair)
            .filter(|(_, p)| p.verdict == Verdict::DoneCandidate)
            .and_then(|(r, p)| breakage_proof(p, r, lock));
        let evidence_missing = pair.is_some_and(|p| p.verdict == Verdict::DoneCandidate)
            && breakage.is_none()
            && literal_proof.is_none();
        let refusal = report
            .and_then(|r| r.refusal)
            .or(evidence_missing.then_some("evidence_missing"));
        let proven_pair =
            pair.filter(|p| p.verdict != Verdict::DoneCandidate || breakage.is_some());
        let mut decision = decide(
            entry,
            proven_pair,
            DecisionContext {
                inspection: constraints
                    .get(&entry.id)
                    .is_some_and(|t| inspection_only(t, true)),
                disabled,
                refusal,
                text_evidence: literal_proof.is_some(),
            },
        );
        if entry.text_item
            && literal_proof.is_none()
            && decision.state == RequirementState::Undecided
            && matches!(decision.reason.as_str(), "host_unrun" | "no_base_result")
            && entry
                .checks
                .values()
                .any(|p| *p == Some(archon_workflow::baseline_results::BaselineOutcome::Passed))
            && entry
                .checks
                .iter()
                .filter(|(_, p)| {
                    **p == Some(archon_workflow::baseline_results::BaselineOutcome::Passed)
                })
                .all(|(id, _)| {
                    contract
                        .acceptance
                        .iter()
                        .chain(&contract.supplementary)
                        .find(|c| &c.id == id)
                        .is_some_and(|c| {
                            !crate::command::workflow_task_set::link_diagnostics::reads_deliverable(
                                c,
                                &entry.deliverables,
                                project,
                                repository,
                            )
                        })
                })
        {
            decision.reason = "check_dependency_unproven".into();
        }
        if let Some(pair) = pair {
            decision.notes.extend(pair.attempts.iter().filter_map(
                |attempt| match &attempt.outcome {
                    archon_workflow::breaker_verdict::AttemptOutcome::Unavailable {
                        reason,
                        ..
                    } => Some(reason.clone()),
                    _ => None,
                },
            ));
            let count = pair
                .survivors
                .iter()
                .filter(|s| s.equivalence_unjudged)
                .count();
            if count > 0 {
                decision.notes.push(format!("equivalence_unjudged={count}"));
            }
            if pair.verdict == Verdict::UndecidedEquivalence {
                decision.notes.extend(
                    pair.equivalence
                        .iter()
                        .filter_map(|m| m.first.reason.clone()),
                );
            }
        }
        let key = (entry.task_id.clone(), entry.id.clone());
        if decision.state == RequirementState::Done {
            let proof = if decision.reason == "text_evidence" {
                literal_proof
            } else {
                breakage
            };
            if let Some(proof) = proof {
                outcome.proofs.insert(key.clone(), proof);
            } else {
                decision = decide(
                    entry,
                    None,
                    DecisionContext {
                        inspection: false,
                        disabled,
                        refusal: Some("evidence_missing"),
                        text_evidence: false,
                    },
                );
            }
        }
        if decision.state == RequirementState::Undecided {
            outcome.undecided.push(UndecidedNote {
                task_id: entry.task_id.clone(),
                id: entry.id.clone(),
                reason: decision.reason.clone(),
                survivors: report
                    .zip(pair)
                    .map(|(r, p)| {
                        p.survivors
                            .iter()
                            .filter_map(|s| breaker::survivor_patch(r, &s.patch))
                            .filter(|s| s.len() <= 4096)
                            .take(2)
                            .collect()
                    })
                    .unwrap_or_default(),
                equivalence_unjudged: pair.map_or(0, |p| {
                    p.survivors
                        .iter()
                        .filter(|s| s.equivalence_unjudged)
                        .count()
                }),
            });
        }
        outcome.decisions.insert(key, decision);
    }
    Ok(outcome)
}
fn empty_proof(
    id: &str,
    check: &archon_workflow::task_set_contract::AcceptanceCriterion,
    lock: &AcceptanceLock,
    kind: DoneKind,
) -> DoneProof {
    DoneProof {
        id: id.into(),
        kind,
        check_id: check.id.clone(),
        check_digest: archon_workflow::baseline_results::check_digest(&check.check),
        base_commit: lock.baseline_commit.clone().unwrap_or_default(),
        mutant_id: None,
        body_gut_mutant_id: None,
        patch_digest: None,
        patch: None,
        output_tail: None,
        output_digest: None,
        literals: Vec::new(),
        evidence_truncated: false,
    }
}
fn text_proof(
    entry: &archon_workflow::v2::requirement_classes::RequirementEntry,
    contract: &AcceptanceContract,
    lock: &AcceptanceLock,
    text: &str,
    project: &Path,
    repository: &Path,
) -> Option<DoneProof> {
    if !entry.text_item {
        return None;
    }
    let normalized = text.split_whitespace().collect::<Vec<_>>().join(" ");
    for check in contract.acceptance.iter().chain(&contract.supplementary) {
        if entry.checks.get(&check.id)
            != Some(&Some(
                archon_workflow::baseline_results::BaselineOutcome::Passed,
            ))
            || !lock.baseline_results.get(&check.id).is_some_and(|r| {
                r.check_digest == archon_workflow::baseline_results::check_digest(&check.check)
            })
        {
            continue;
        }
        let archon_workflow::task_set_contract::AcceptanceCheck::Command { command, .. } =
            &check.check
        else {
            continue;
        };
        if !crate::command::workflow_task_set::link_diagnostics::reads_deliverable(
            check,
            &entry.deliverables,
            project,
            repository,
        ) {
            continue;
        }
        let literals: Vec<_> =
            crate::command::workflow_task_set::executability::verdict_shell::quoted_words(command)
                .into_iter()
                .map(|s| s.split_whitespace().collect::<Vec<_>>().join(" "))
                .filter(|s| s.len() >= 4)
                .collect();
        if !literals.is_empty() && literals.iter().all(|s| normalized.contains(s)) {
            let mut proof = empty_proof(&entry.id, check, lock, DoneKind::TextEvidence);
            proof.literals = literals;
            return Some(proof);
        }
    }
    None
}
fn breakage_proof(
    pair: &archon_workflow::breaker_verdict::PairVerdict,
    report: &breaker::VerdictReport,
    lock: &AcceptanceLock,
) -> Option<DoneProof> {
    let mut runs: Vec<&MutantRun> = report
        .runs
        .iter()
        .filter(|r| r.task_id == pair.task_id && r.requirement_id == pair.id && r.counted)
        .collect();
    runs.sort_by(|a, b| (&a.check_id, &a.mutant_id).cmp(&(&b.check_id, &b.mutant_id)));
    let gut = runs.iter().find(|r| r.kind == MutantKind::BodyGut)?;
    let run = runs.iter().find(|r| {
        r.kind == MutantKind::Behaviour
            && pair.relevance.iter().any(|m| {
                m.mutant_id == r.mutant_id
                    && m.check_id == r.check_id
                    && m.relevance == archon_workflow::breaker_verdict::Relevance::Relevant
            })
    })?;
    let (patch, output) = breaker::evidence_text(report, run).ok()?;
    let mut proof = DoneProof {
        id: pair.id.clone(),
        kind: DoneKind::Breakage,
        check_id: run.check_id.clone(),
        check_digest: lock
            .baseline_results
            .get(&run.check_id)?
            .check_digest
            .clone(),
        base_commit: lock.baseline_commit.clone().unwrap_or_default(),
        mutant_id: Some(run.mutant_id.clone()),
        body_gut_mutant_id: Some(gut.mutant_id.clone()),
        patch_digest: Some(archon_workflow::task_set_contract::content_digest(
            patch.as_bytes(),
        )),
        patch: None,
        output_tail: None,
        output_digest: output
            .as_ref()
            .map(|s| archon_workflow::task_set_contract::content_digest(s.as_bytes())),
        literals: Vec::new(),
        evidence_truncated: patch.len() > 8192,
    };
    if patch.len() <= 8192 {
        proof.patch = Some(patch);
    }
    proof.output_tail = output.map(|s| {
        let mut at = s.len().saturating_sub(2048);
        while !s.is_char_boundary(at) {
            at += 1;
        }
        s[at..].into()
    });
    Some(proof)
}

#[cfg(test)]
#[path = "workflow_freeze_skeleton_proof_tests.rs"]
mod tests;

fn classification_ready(
    error: Option<&archon_workflow::v2::requirement_classes::ClassificationError>,
) -> Result<()> {
    if let Some(archon_workflow::v2::requirement_classes::ClassificationError::Transient {
        transient,
    }) = error
    {
        return Err(crate::command::workflow_host_fault::transient(
            transient.clone(),
        ));
    }
    Ok(())
}

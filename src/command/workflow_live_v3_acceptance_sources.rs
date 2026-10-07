//! PLAN-11: before an acceptance round runs any check, it settles every
//! change to a source a frozen check runs (`check_source_settle`): changes a
//! landing had held (`v2::write::check_source_hold`) and changes found in the
//! tree, made outside any landing. The judge decides each; an accepted change
//! is applied and re-pinned with recorded lineage, a refused one stays out or
//! is restored. A check whose source still differs from its pin afterwards
//! is a contract defect for the round: it fails and does not run.
//!
//! Every settlement is recorded on the round as a contract repair with the
//! trigger [`REPAIR_TRIGGER_CHECK_SOURCE`].

use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

use archon_workflow::check_source_pins::load_for_run;
use archon_workflow::check_source_requests::VERDICT_ACCEPTED;
use archon_workflow::check_source_resolve::Roots;
use archon_workflow::check_source_settle::{
    Settle, SourceJudge, SourceJudgeInput, SourceVerdict, settle,
};
use archon_workflow::task_set_contract::{AcceptanceContract, JudgeDecision};
use archon_workflow::task_set_publish_lock::PublishLockError;
use archon_workflow::v2::acceptance_stage::{
    AcceptanceCheckStatus, AcceptanceContractRepairV1, AcceptanceRoundRecordV1,
};
use archon_workflow::{WorkflowError, WorkflowLlmClient, WorkflowResult};

use super::exec::StageContext;

/// A round's settlement of a pinned check-source change.
pub(super) const REPAIR_TRIGGER_CHECK_SOURCE: &str = "check_source_change";

/// Wall clock for one source judgment.
const SOURCE_JUDGE_TIMEOUT_SECS: u64 = 1_800;

/// What a round's settlement leaves for the round.
#[derive(Default)]
pub(super) struct Outcome {
    /// Contract-defect text per check that cannot run on its pinned sources.
    pub(super) defects: BTreeMap<String, String>,
    /// Per check that may run but must not pass: why.
    pub(super) must_fail: BTreeMap<String, String>,
}

/// Settle every pinned check-source change for this round. A publish of the
/// task set a crash left that no read can settle pauses the run with the
/// evidence (Issue 338): it is the host's environment, never the contract's
/// defect, so no check is failed for it.
pub(super) async fn apply(
    llm: Option<&dyn WorkflowLlmClient>,
    context: &StageContext,
    contract: &AcceptanceContract,
    run_dir: &Path,
    record: &mut AcceptanceRoundRecordV1,
) -> WorkflowResult<Outcome> {
    let roots = Roots {
        repository: &context.repository,
        project: &context.project,
    };
    let (store, pins) = match archon_workflow::stage_write::with_write(|| {
        WorkflowResult::Ok(load_for_run(
            run_dir,
            &context.project,
            &context.task_root,
            &roots,
        ))
    })? {
        Ok(Some(loaded)) => loaded,
        Ok(None) => return Ok(Outcome::default()),
        Err(PublishLockError::Unsettled(evidence)) => return Err(paused(&evidence)),
        Err(PublishLockError::Failed(error)) => {
            // Fail closed: no check runs on sources nobody can show it was
            // frozen with.
            let why = format!(
                "contract defect: the frozen checks' pinned sources cannot be read ({error}), so no check can be shown to run on the source it was frozen with"
            );
            record.operational_errors.push(why.clone());
            let defects = (contract.acceptance.iter())
                .chain(&contract.supplementary)
                .map(|entry| (entry.id.clone(), why.clone()))
                .collect();
            return Ok(Outcome {
                defects,
                must_fail: BTreeMap::new(),
            });
        }
    };
    let (judge, judge_note) = match (llm, recorded_judge(contract)) {
        (None, _) => (None, "the round has no model client".to_string()),
        (Some(_), Err(why)) => (None, why),
        (Some(client), Ok((model, provider))) => match client.provider_id() {
            Some(actual) if actual == provider => {
                (Some(LlmSourceJudge { client, model }), String::new())
            }
            actual => (
                None,
                format!(
                    "the freeze-time judge was {model} on provider {provider}, but this round's client serves {}",
                    actual.as_deref().unwrap_or("an unreported provider")
                ),
            ),
        },
    };
    let settled = settle(
        &Settle {
            run_root: run_dir,
            roots,
            store: &store,
            contract,
            judge: judge.as_ref().map(|judge| judge as &dyn SourceJudge),
            judge_note,
        },
        pins,
    )
    .await;
    if let Some(evidence) = &settled.paused {
        return Err(paused(evidence));
    }
    record
        .operational_errors
        .extend(settled.errors.iter().cloned());
    for settlement in &settled.settlements {
        let Some(resolution) = &settlement.resolution else {
            continue;
        };
        let accepted = resolution.verdict == VERDICT_ACCEPTED;
        record.contract_repairs.push(AcceptanceContractRepairV1 {
            check_ids: settlement.request.check_ids.iter().cloned().collect(),
            trigger: REPAIR_TRIGGER_CHECK_SOURCE.into(),
            repaired: accepted,
            freeze_event_id: String::new(),
            failure: if accepted {
                String::new()
            } else {
                format!("{}: {}", resolution.verdict, resolution.reason)
            },
            diagnostics: vec![format!(
                "request {} ({}) for {}: {}{}",
                settlement.request.request_id,
                settlement.request.origin,
                settlement.request.label(),
                resolution.verdict,
                if resolution.applied {
                    ", applied to the tree"
                } else {
                    ""
                }
            )],
        });
    }
    Ok(Outcome {
        defects: (settled.defects.into_iter())
            .map(|(id, why)| (id, format!("contract defect: {why}")))
            .collect(),
        must_fail: settled.must_fail,
    })
}

/// The pause of a round whose task set holds a journal no read can settle.
fn paused(evidence: &str) -> WorkflowError {
    tracing::warn!(%evidence, "pausing: the task set's interrupted publish could not be settled");
    WorkflowError::ControlPaused(format!(
        "the frozen checks' pinned sources cannot be read as one version: {evidence}"
    ))
}

/// A check that passed although it names a test nothing defines fails: a
/// run that selected no test proves nothing.
pub(super) fn fail_unsatisfied(
    record: &mut AcceptanceRoundRecordV1,
    must_fail: &BTreeMap<String, String>,
) {
    for check in &mut record.checks {
        if let Some(why) = must_fail.get(&check.check_id)
            && check.status == AcceptanceCheckStatus::Passed
        {
            check.status = AcceptanceCheckStatus::Failed;
            check.stdout_tail = format!("[host] {why}\n{}", check.stdout_tail);
        }
    }
}

/// The freeze-time judge -- the one (model, provider) every judged check
/// records -- so a source is judged like the checks it serves; none, or
/// more than one, and no source is judged this round.
fn recorded_judge(contract: &AcceptanceContract) -> Result<(String, String), String> {
    let judges: std::collections::BTreeSet<(String, String)> = (contract.acceptance.iter())
        .chain(&contract.supplementary)
        .filter(|entry| entry.judgment.verdict == JudgeDecision::Accepted)
        .filter_map(|entry| {
            let sampling = entry.judgment.sampling.as_ref()?;
            Some((
                sampling.get("model")?.as_str()?.to_string(),
                sampling.get("provider")?.as_str()?.to_string(),
            ))
        })
        .collect();
    let mut judges = judges.into_iter();
    match (judges.next(), judges.next()) {
        (Some(judge), None) => Ok(judge),
        (Some(_), Some(_)) => Err("the frozen contract was judged by more than one judge, so no source judge matches them all".into()),
        (None, _) => Err("the frozen contract records no judge model and provider".into()),
    }
}

struct LlmSourceJudge<'a> {
    client: &'a dyn WorkflowLlmClient,
    model: String,
}

pub(super) fn source_judge_prompt(input: &SourceJudgeInput) -> String {
    let version = |text: &Option<String>, absent: &str| match text {
        Some(text) => format!("```\n{text}\n```"),
        None => absent.to_string(),
    };
    let pinned = if input.pinned_retained {
        version(
            &input.pinned,
            "(the source did not exist when the check was frozen)",
        )
    } else {
        "(the frozen version was not retained)".to_string()
    };
    [
        "Adversarially judge a proposed change to a source that a frozen acceptance check runs. The check decides whether its criterion holds; the source is the test or script it executes, and it was frozen so that no implementation can weaken what judges it.".to_string(),
        format!("Check {}: criterion: {}", input.check_id, input.criterion),
        format!("Check command: {}", input.command),
        format!("Source: {} (the change was {})", input.source, match input.origin.as_str() {
            "landing" => "proposed by an implementing task's landing",
            _ => "found in the tree, made outside any landing",
        }),
        match (&input.diff, input.part) {
            (Some(diff), Some((index, count))) => format!(
                "The source is too large to show whole. The change from the frozen version to the proposed one, as a unified diff (part {index} of {count}; every part is judged, and all must be accepted):\n```diff\n{diff}\n```"
            ),
            _ => format!(
                "Frozen version:\n{pinned}\n\nProposed version:\n{}",
                version(&input.proposed, "(the proposal deletes the source)")
            ),
        },
        "Accept ONLY when, with the proposed version, the check still fails whenever its criterion is false: every assertion of the frozen version that bears on the criterion is kept or strengthened, nothing that runs is skipped, ignored, stubbed, short-circuited, made conditional or trivially true, and the source still exercises the deliverable rather than a stand-in. A source created for the first time is accepted only when it genuinely tests the criterion as written. Otherwise refute it, and say which assertion is weakened.".to_string(),
        "Return JSON only as {\"verdict\":\"accepted|refuted\",\"counterexample\":\"...\",\"reason\":\"...\"}; counterexample and reason are each a non-empty single-line sentence: the closest passing-but-false state the proposal allows, or that none is constructible.".to_string(),
    ]
    .join("\n\n")
}

#[derive(serde::Deserialize)]
struct Reply {
    verdict: JudgeDecision,
    counterexample: String,
    reason: String,
}

#[async_trait::async_trait]
impl SourceJudge for LlmSourceJudge<'_> {
    async fn judge(&self, input: &SourceJudgeInput) -> Result<SourceVerdict, String> {
        let prompt = source_judge_prompt(input);
        let outcome = tokio::time::timeout(
            Duration::from_secs(SOURCE_JUDGE_TIMEOUT_SECS),
            self.client.send_message_with_temperature(
                vec![serde_json::json!({ "role": "user", "content": prompt })],
                Vec::new(),
                Vec::new(),
                &self.model,
                0.0,
            ),
        )
        .await
        .map_err(|_| format!("the source judge timed out after {SOURCE_JUDGE_TIMEOUT_SECS}s"))?
        .map_err(|error| error.to_string())?;
        if !matches!(
            outcome.stop_reason.as_deref(),
            Some("end_turn" | "stop" | "completed")
        ) {
            return Err(format!(
                "the source judge ended with stop reason {:?}; a possibly truncated verdict is never read",
                outcome.stop_reason
            ));
        }
        let document = crate::command::workflow_freeze_candidate::candidate_document(
            outcome.content.trim().as_bytes(),
        );
        let reply: Reply = serde_json::from_slice(&document).map_err(|error| {
            format!("the source judge's reply is not the verdict JSON: {error}")
        })?;
        if reply.reason.trim().is_empty() {
            return Err("the source judge gave no reason".into());
        }
        Ok(SourceVerdict {
            accepted: reply.verdict == JudgeDecision::Accepted,
            reason: reply.reason,
            counterexample: reply.counterexample,
        })
    }
}

#[cfg(all(test, unix))]
#[path = "workflow_live_v3_acceptance_sources_tests.rs"]
mod tests;

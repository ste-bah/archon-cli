//! Requirement proof orchestration used by freeze decisions.
use super::super::breaker_view::{BaseView, materialise_view, view_file_keys};
use super::*;
use crate::command::workflow_task_set::judge::JUDGE_MODEL_ALIAS;
use archon_workflow::WorkflowLlmClient;
const FIRST_MODEL: &str = "opus";
const SECOND_MODEL: &str = JUDGE_MODEL_ALIAS;
#[path = "workflow_acceptance_breaker_history.rs"]
mod history;
use archon_workflow::breaker_verdict::{rule, *};
#[path = "workflow_acceptance_breaker_author.rs"]
mod author;
#[path = "workflow_acceptance_breaker_store.rs"]
mod cache;
#[path = "workflow_acceptance_breaker_call.rs"]
mod call;
#[path = "workflow_acceptance_breaker_finish.rs"]
mod finish;
#[path = "workflow_acceptance_breaker_gut.rs"]
mod gut;
#[path = "workflow_acceptance_breaker_judges.rs"]
mod judges;
#[path = "workflow_acceptance_breaker_judging.rs"]
mod judging;
#[path = "workflow_acceptance_breaker_named.rs"]
mod named;
#[path = "workflow_acceptance_breaker_prompt.rs"]
mod prompt;
pub(crate) struct VerdictRequest<'a> {
    pub contract: &'a AcceptanceContract,
    pub chain_digest: &'a str,
    pub lock: &'a AcceptanceLock,
    pub classification: &'a RequirementClassification,
    pub prd_text: &'a str,
    pub max_runs: u64,
    pub min_deadline_secs: u64,
    pub llm_deadline_secs: u64,
    pub call_limits: archon_core::config::BreakerCallLimits,
}
pub(crate) struct VerdictReport {
    pub refusal: Option<&'static str>,
    pub verdicts: ProofVerdicts,
    pub store: Option<PathBuf>,
    pub runs: Vec<MutantRun>,
}
struct Context<'a> {
    pending: Vec<(usize, String, String)>,
    probe: &'a HostProbe,
    client: &'a dyn WorkflowLlmClient,
    req: VerdictRequest<'a>,
    verdicts: ProofVerdicts,
    store: Option<PathBuf>,
    circuit: call::Circuit,
}
fn identity(client: &dyn WorkflowLlmClient, role: &str, default: &str) -> ModelIdentity {
    ModelIdentity {
        provider_id: client.provider_id(),
        model: client.resolve_model_alias(&client.model_for_role(role, default)),
    }
}
async fn view(ctx: &Context<'_>, index: usize) -> Result<BaseView, String> {
    let entry = &ctx.req.classification.entries[index];
    let snapshot = ctx
        .req
        .classification
        .base_snapshot
        .clone()
        .ok_or("snapshot_missing")?;
    let keys = selection::keys(entry, &snapshot, &ctx.probe.repository, false);
    let checks: Vec<_> = selection::passing_checks(entry, ctx.req.contract, ctx.req.lock)
        .iter()
        .filter_map(|id| {
            ctx.req
                .contract
                .acceptance
                .iter()
                .chain(&ctx.req.contract.supplementary)
                .find(|c| &c.id == id)
                .cloned()
        })
        .collect();
    let (parent, repository, project) = (
        ctx.probe.copy_parent.clone(),
        ctx.probe.repository.clone(),
        ctx.probe.project.clone(),
    );
    run::blocking(move || {
        let files = view_file_keys(&parent, &repository, &snapshot, &keys).map_err(patch::host)?;
        let exclude: Vec<_> = checks
            .iter()
            .flat_map(|c| named::from_keys(c, &repository, &project, &files))
            .filter(|k| !selection::contains(&keys, k))
            .collect();
        materialise_view(&parent, &repository, &snapshot, &keys, &exclude).map_err(patch::host)
    })
    .await
    .map_err(|e| match e {
        RunOutcome::Unrun { detail, .. } => detail,
        other => format!("{other:?}"),
    })
}

pub(crate) async fn prove_requirements(
    probe: &HostProbe,
    client: &dyn WorkflowLlmClient,
    req: VerdictRequest<'_>,
) -> VerdictReport {
    let breaker = identity(client, "breaker", JUDGE_MODEL_ALIAS);
    let relevance = identity(client, "breaker_relevance", FIRST_MODEL);
    let equivalence = vec![
        identity(client, "breaker_equivalence", FIRST_MODEL),
        identity(client, "breaker_equivalence_second", SECOND_MODEL),
    ];
    let distinct = equivalence[0].distinct(&equivalence[1]);
    let mut refusals = vec![];
    if !distinct {
        refusals.push("equivalence_judges_not_distinct".into());
    }
    if !breaker.distinct(&relevance) {
        refusals.push("relevance_same_as_breaker".into());
    }
    let verdicts = ProofVerdicts {
        schema: VERDICT_SCHEMA,
        base_commit: req.lock.baseline_commit.clone().unwrap_or_default(),
        acceptance_digest: req.lock.digest.clone(),
        baseline_results_digest: results_digest(&req.lock.baseline_results),
        snapshot_digest: req
            .classification
            .base_snapshot
            .as_ref()
            .map(|s| s.digest.clone())
            .unwrap_or_default(),
        call_limits: Some(
            serde_json::json!({"per_pair":req.call_limits.per_pair,"global":req.call_limits.global,"sources":req.call_limits.resolved_sources()}),
        ),
        rounds: 0,
        runs_executed: 0,
        runs_reused: 0,
        judges: JudgeIdentities {
            breaker,
            relevance,
            equivalence,
            equivalence_distinct: distinct,
        },
        refusals,
        entries: vec![],
    };
    let root = probe
        .resume
        .persist
        .then(|| super::store::root(&probe.project, &verdicts.base_commit))
        .flatten();
    let circuit = call::Circuit::new(req.llm_deadline_secs, &req.call_limits);
    let mut ctx = Context {
        probe,
        client,
        req,
        verdicts,
        store: root,
        circuit,
        pending: vec![],
    };
    let empty = MutantSet {
        schema: BREAKAGE_SCHEMA,
        base_commit: ctx.verdicts.base_commit.clone(),
        mutants: vec![],
    };
    let refusal = breakage_refusal(
        probe,
        &BreakageRequest {
            contract: ctx.req.contract,
            chain_digest: ctx.req.chain_digest,
            lock: ctx.req.lock,
            classification: ctx.req.classification,
            mutants: &empty,
            max_runs: ctx.req.max_runs,
            min_deadline_secs: ctx.req.min_deadline_secs,
        },
    );
    if let Some(code) = refusal {
        ctx.verdicts.refusals.push(code.into());
    }
    let mut pairs: Vec<_> = ctx
        .req
        .classification
        .entries
        .iter()
        .map(|e| PairVerdict {
            task_id: e.task_id.clone(),
            id: e.id.clone(),
            verdict: Verdict::HostUnrun,
            run_budget_exhausted: false,
            requirement_text_digest: rule::requirement_text(ctx.req.prd_text, &e.id)
                .map(|t| content_digest(t.as_bytes())),
            mutants: vec![],
            attempts: vec![],
            relevance: vec![],
            equivalence: vec![],
            killed_by: BTreeMap::new(),
            patch_keys: BTreeMap::new(),
            owed: None,
            survivors: Vec::new(),
        })
        .collect();
    let mut states = vec![vec![]; pairs.len()];
    for (index, pair) in pairs.iter_mut().enumerate() {
        let entry = &ctx.req.classification.entries[index];
        if selection::passing_checks(entry, ctx.req.contract, ctx.req.lock).is_empty() {
            states[index].push(ProofState::NoPassingCheck);
        } else if ctx
            .req
            .classification
            .base_snapshot
            .as_ref()
            .is_none_or(|s| selection::keys(entry, s, &probe.repository, false).is_empty())
        {
            states[index].push(ProofState::NoCountableDeliverable);
        }
        pair.verdict = rule::decide(rule::Decision {
            refused: refusal.is_some(),
            has_text: pair.requirement_text_digest.is_some(),
            states: &states[index],
            pair,
            runs: &[],
            distinct,
        });
    }
    let previous = history::load(&ctx);
    let (restored, mut carried) = if refusal.is_none() {
        history::restore(&ctx, &previous, &mut pairs)
    } else {
        (BTreeSet::new(), Vec::new())
    };
    let order = history::order(&pairs, &previous);
    let mut all = Vec::new();
    for attempt in 1..=BREAKER_ATTEMPTS {
        finish::decisions(&mut pairs, &states, &all, refusal.is_some(), distinct);
        let mut mutants = std::mem::take(&mut carried);
        if refusal.is_none() {
            for &index in &order {
                let pair = &mut pairs[index];
                ctx.circuit.pair = index;
                if attempt == 2 {
                    mutants.extend(gut::retry(&mut ctx, index, pair, &all).await);
                }
                if restored.contains(&index) {
                    if attempt == 1 && history::needs_gut(&previous, pair) {
                        mutants.extend(gut::resume(&mut ctx, index, pair).await);
                    }
                    continue;
                }
                if states[index].iter().any(|s| {
                    matches!(
                        s,
                        ProofState::NoPassingCheck | ProofState::NoCountableDeliverable
                    )
                }) || pair.requirement_text_digest.is_none()
                {
                    continue;
                }
                if attempt == 2
                    && !pair.attempts.iter().any(|a| {
                        !a.reply_key.starts_with("body_gut:")
                            && matches!(a.outcome, AttemptOutcome::Unusable { .. })
                    })
                    && !matches!(
                        pair.verdict,
                        Verdict::BreakerUnavailable
                            | Verdict::BreakerUnusable
                            | Verdict::Unmutable
                            | Verdict::NoViableBehaviourMutant
                            | Verdict::UndecidedRelevance
                    )
                {
                    continue;
                }
                if pair.attempts.iter().any(|a| {
                    matches!(
                        a.outcome,
                        AttemptOutcome::Unavailable {
                            cause: UnavailableCause::Transient,
                            ..
                        }
                    )
                }) {
                    continue;
                }
                mutants.extend(author::author(&mut ctx, index, pair, attempt, &all).await);
            }
        }
        if mutants.is_empty() {
            continue;
        }
        history::save_pending(&mut ctx, &mut pairs, &mutants);
        let retried: BTreeSet<_> = mutants
            .iter()
            .map(|m| (m.task_id.clone(), m.requirement_id.clone()))
            .collect();
        let set = MutantSet {
            schema: BREAKAGE_SCHEMA,
            base_commit: ctx.verdicts.base_commit.clone(),
            mutants,
        };
        let report = prove_breakage(
            probe,
            BreakageRequest {
                contract: ctx.req.contract,
                chain_digest: ctx.req.chain_digest,
                lock: ctx.req.lock,
                classification: ctx.req.classification,
                mutants: &set,
                max_runs: ctx.req.max_runs.saturating_sub(ctx.verdicts.runs_executed),
                min_deadline_secs: ctx.req.min_deadline_secs,
            },
        )
        .await;
        ctx.verdicts.rounds += 1;
        if let Some(summary) = &report.proofs.summary {
            ctx.verdicts.runs_executed += summary.runs_executed;
            ctx.verdicts.runs_reused += summary.runs_reused;
            ctx.verdicts.refusals.extend(summary.refusals.clone());
            for (index, pair) in pairs.iter().enumerate() {
                if let Some(p) = summary
                    .entries
                    .iter()
                    .find(|p| p.task_id == pair.task_id && p.id == pair.id)
                    && p.state != ProofState::NoMutants
                {
                    states[index].push(p.state);
                }
            }
        }
        all.extend(report.proofs.runs.values().cloned());
        gut::record_failures(&mut pairs, &all);
        judging::run(
            &mut ctx,
            &mut pairs,
            &mut states,
            &all,
            &retried,
            report.store.as_deref(),
            distinct,
            &order,
        )
        .await;
    }
    history::retain_unrun(&previous, &mut pairs, &all);
    finish::decisions(&mut pairs, &states, &all, refusal.is_some(), distinct);
    finish::supplements(&mut ctx, &mut pairs, &all, distinct);
    for (index, key, reply) in &ctx.pending {
        let entry = &ctx.req.classification.entries[*index];
        if pairs.iter().any(|p| {
            p.task_id == entry.task_id
                && p.id == entry.id
                && (matches!(
                    p.verdict,
                    Verdict::DoneCandidate
                        | Verdict::WeakTest
                        | Verdict::BudgetExhausted
                        | Verdict::HostUnrun
                ) || !p.survivors.is_empty()
                    && matches!(
                        p.verdict,
                        Verdict::UndecidedEquivalence | Verdict::JudgeUnavailable
                    ))
        }) {
            cache::save(
                probe,
                ctx.store.as_deref(),
                key,
                serde_json::Value::String(reply.clone()),
                &mut ctx.verdicts.refusals,
            );
        }
    }
    ctx.verdicts.entries = pairs;
    let evidence_saved = ctx.store.as_ref().is_none_or(|root| {
        let present = |key: &str| {
            !archon_workflow::base_snapshot::outside_key(key)
                && root
                    .join(archon_workflow::base_snapshot::key_path(key))
                    .is_file()
        };
        ctx.verdicts
            .entries
            .iter()
            .flat_map(|p| &p.survivors)
            .all(|s| present(&s.patch) && s.output.as_deref().is_none_or(present))
    });
    if let Some(path) = ctx
        .store
        .as_ref()
        .and_then(|r| super::store::summary_path(r, &ctx.req.lock.digest))
        .map(|p| p.with_file_name("verdicts.json"))
        && (!evidence_saved
            || serde_json::to_vec_pretty(&ctx.verdicts)
                .map_err(|e| e.to_string())
                .and_then(|b| super::store::atomic(&path, &b))
                .is_err())
    {
        ctx.verdicts.refusals.push("verdicts_not_saved".into());
    }
    history::save(&mut ctx);
    VerdictReport {
        refusal,
        verdicts: ctx.verdicts,
        store: ctx.store,
        runs: all,
    }
}
#[cfg(test)]
#[path = "workflow_acceptance_breaker_test_client.rs"]
pub(crate) mod test_client;
#[cfg(test)]
#[path = "workflow_acceptance_breaker_tests.rs"]
mod tests;

#[cfg(all(test, unix))]
#[path = "workflow_acceptance_breaker_recorder_tests.rs"]
mod recorder_tests;
#[cfg(test)]
#[path = "workflow_acceptance_breaker_view_tests.rs"]
mod view_tests;

#[cfg(all(test, unix))]
#[path = "workflow_acceptance_breaker_judge_tests.rs"]
mod judge_tests;
#[cfg(test)]
#[path = "workflow_acceptance_breaker_parse_tests.rs"]
mod parse_tests;
#[cfg(all(test, unix))]
#[path = "workflow_acceptance_breaker_retry_tests.rs"]
mod retry_tests;

#[cfg(test)]
#[path = "workflow_acceptance_breaker_boundary_tests.rs"]
mod boundary_tests;

#[cfg(all(test, unix))]
#[path = "workflow_acceptance_breaker_round_two_tests.rs"]
mod round_two_tests;

#[cfg(test)]
#[path = "workflow_acceptance_breaker_corpus_tests.rs"]
mod corpus_tests;

#[cfg(all(test, unix))]
#[path = "workflow_acceptance_breaker_history_tests.rs"]
mod history_tests;

#[cfg(all(test, unix))]
#[path = "workflow_acceptance_breaker_round_six_tests.rs"]
mod round_six_tests;

pub(crate) fn evidence_text(
    report: &VerdictReport,
    run: &MutantRun,
) -> Result<(String, Option<String>), String> {
    store::evidence_text_at(report.store.as_deref(), run)
}
pub(crate) fn survivor_patch(report: &VerdictReport, patch: &str) -> Option<String> {
    match &report.store {
        None => Some(patch.into()),
        Some(root) if !archon_workflow::base_snapshot::outside_key(patch) => {
            std::fs::read_to_string(root.join(archon_workflow::base_snapshot::key_path(patch))).ok()
        }
        Some(_) => None,
    }
}

#[cfg(all(test, unix))]
#[path = "workflow_acceptance_breaker_decision_tests.rs"]
mod decision_tests;

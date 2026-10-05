//! Obligation fidelity — does passing every claiming task make the PRD's
//! obligation true? — asked of a critic model, cached, waivable, blocking.
//!
//! # Where this sits among the lint sections
//!
//! Coverage (`coverage.rs`) is a set comparison: is every obligation claimed.
//! It holds for any PRD because it never reads prose. This section reads
//! prose, so it cannot be deterministic, and it costs tokens, so `workflow
//! lint` runs it only on `--fidelity`. The set gate the decomposition drives
//! (`workflow lint --tasks … --gate-envelope …`) runs it always: that gate is
//! where a task set with bodies is accepted, and a body that honours its claim
//! only inside a temporary root is exactly what it exists to refuse.
//!
//! # Clusters, cache, waivers
//!
//! Obligations are grouped by the exact set of tasks audited for them, so
//! each task's full text travels once per cluster rather than once per
//! obligation. Each call batch's verdicts are saved under the project's
//! `.archon/lint-cache` the moment they arrive, keyed by a digest of the
//! obligation texts, the task texts and the skeleton plus the binary and the
//! critic (`fidelity_store.rs`): re-running lint over an unchanged set costs
//! nothing, and editing one task re-asks only the clusters that task is in.
//! The staged set gate runs the calls under its host wall clock and stops
//! resumable before it, so a retry continues from the saved batches
//! (`fidelity_resume.rs`, Issue 259). A false verdict blocks
//! unless the operator has waived that obligation id; the waiver is recorded
//! verbatim in the freeze pin so the override is as auditable as the stamp it
//! overrides.
//!
//! # The ownership chain (Issue-43)
//!
//! A cluster's tasks are the claiming tasks plus every task of the same set
//! that a claiming task names in its text — one hop, never recursive. Live,
//! twelve of eighteen blocking verdicts read "the claiming task says task X
//! produces this, and X is not listed": the result was deferred to a named
//! sibling the critic never saw, so an owned result read as a gap. A named
//! sibling that does not itself claim the obligation earns a non-blocking
//! `NOTE`, so a chain that ends nowhere is still visible.
//!
//! # The per-body audit (Issue-44)
//!
//! The set gate has no repair loop: a false verdict there stops the run, and
//! live that was six blocking obligations found after seven hours of body
//! authoring, with no path back to any author. So the body gate
//! (`land-task-body`) asks the same question of the candidate body first,
//! for the obligations that body claims, while its author still has attempts:
//! [`audit_task_file_candidate`]. Clusters are built exactly as the set gate
//! builds them over the bodies landed so far plus the candidate, so a cluster
//! nothing later touches is served from the cache when the set gate re-asks.
//!
//! # The frozen skeleton (Issue-45)
//!
//! Every audit reads `task-skeleton.json` beside the tasks once and puts the
//! whole set's frozen ordering and ownership in front of the critic, so a
//! sibling whose body is not yet written is still a known task with known
//! dependencies rather than a gap. A set without a frozen skeleton (legacy
//! `workflow lint --tasks`) is still audited; the prompt says so. The
//! skeleton section is part of every cluster digest.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result, anyhow};
use archon_workflow::fidelity_audit::{
    ClaimedObligation, ClaimingTask, ObligationWaiver, SkeletonSummary, fidelity_cluster_digest,
    fidelity_finding,
};
use archon_workflow::llm_client_port::WorkflowLlmClient;
use archon_workflow::obligation_ids::obligation_texts;
use archon_workflow::task_universe::parsing::parse_task_file;

use super::LintSource;
use super::fidelity_critic::CRITIC_MODEL_ALIAS;
#[cfg(test)]
use super::fidelity_critic::FIDELITY_ATTEMPTS;
use super::fidelity_resume::{LintIncomplete, Resolved, resolve};
use super::fidelity_store::{StoreIdentity, VerdictStore, store_dir};
use crate::command::topology_task_graph::task_requirement_claims_tolerant;
use crate::command::workflow_freeze_budget::FreezeResume;
use crate::command::workflow_gate::{GateEvaluation, GateFinding, GateId};

/// Most obligations asked in one call. A cluster of one task claiming
/// twenty-three ids drew replies that dropped an id or invented one, twice;
/// eight verdicts per reply is a size the critic answers completely, and the
/// task texts it re-sends per batch are what provider prompt caches absorb.
const FIDELITY_MAX_OBLIGATIONS_PER_CALL: usize = 8;

pub(super) struct FidelityFinding {
    pub(super) text: String,
    pub(super) subject: String,
    pub(super) source_path: PathBuf,
}

pub(super) struct FidelityAudit {
    pub(super) report: String,
    pub(super) findings: Vec<FidelityFinding>,
}

/// The ordinary lint plus the fidelity section. A client that could not be
/// built, or an audit that could not complete, is an operational error on the
/// evaluation rather than an absent section: the set gate must never read a
/// provider outage as a pass.
pub(crate) async fn evaluate_lint_with_fidelity(
    cwd: &Path,
    source: &LintSource,
    mode: archon_core::config::GateMode,
    client: Result<Arc<dyn WorkflowLlmClient>>,
    waivers: &[ObligationWaiver],
) -> Result<GateEvaluation> {
    let resume = FreezeResume::none();
    evaluate_lint_with_fidelity_resumable(cwd, source, mode, client, waivers, &resume).await
}

/// [`evaluate_lint_with_fidelity`] under `resume`'s budget (Issue 259): the
/// staged set gate's. When the budget runs out before every call batch has
/// a verdict the result is an error carrying [`LintIncomplete`], never an
/// evaluation: the batches answered so far are saved, and a retry of the
/// same call continues from them.
pub(crate) async fn evaluate_lint_with_fidelity_resumable(
    cwd: &Path,
    source: &LintSource,
    mode: archon_core::config::GateMode,
    client: Result<Arc<dyn WorkflowLlmClient>>,
    waivers: &[ObligationWaiver],
    resume: &FreezeResume,
) -> Result<GateEvaluation> {
    let mut evaluation = super::evaluate_lint(cwd, source, mode)?;
    let LintSource::Tasks(path) = source else {
        return Err(anyhow!(
            "obligation fidelity is only defined for --tasks <DIR>: it reads every task body that claims a PRD obligation"
        ));
    };
    let root = super::absolute(cwd, path);
    let outcome = match client {
        Ok(client) => {
            let scope = AuditScope {
                resume: Some(resume),
                ..AuditScope::default()
            };
            audit_scoped(cwd, &root, client.as_ref(), waivers, scope).await
        }
        Err(error) => Err(error.context("building the obligation fidelity critic client")),
    };
    match outcome {
        Ok(audit) => {
            evaluation.report.push_str(&audit.report);
            evaluation
                .findings
                .extend(audit.findings.into_iter().map(|finding| {
                    GateFinding::new(
                        GateId::WorkflowLintTaskSet,
                        finding.text,
                        finding.subject,
                        Some(finding.source_path),
                        archon_workflow::RemediationScope::Body,
                    )
                }));
            Ok(evaluation)
        }
        Err(error) if LintIncomplete::caused(&error).is_some() => Err(error),
        Err(error) => {
            let text = format!("obligation fidelity audit failed operationally: {error:#}");
            let text = match evaluation.operational_error() {
                Some(existing) => format!("{existing}; {text}"),
                None => text,
            };
            Ok(evaluation.with_operational_error(text))
        }
    }
}

/// The body gate's fidelity section (Issue-44): the candidate body, not yet
/// on disk, is judged for the obligations it claims, against the bodies
/// already landed beside it. Each false verdict is a `Body` finding on the
/// claiming task, worded exactly as the set gate words it, so the script's
/// routing sends it back to the body author. A critic that cannot be built
/// or cannot answer is an error the caller must record as operational.
pub(crate) async fn audit_task_file_candidate(
    cwd: &Path,
    path: &Path,
    candidate: &str,
    client: Result<Arc<dyn WorkflowLlmClient>>,
    waivers: &[ObligationWaiver],
) -> Result<(String, Vec<GateFinding>)> {
    let client = client.context("building the obligation fidelity critic client")?;
    let path = super::absolute(cwd, path);
    let tasks_root = path
        .parent()
        .ok_or_else(|| anyhow!("{} has no parent task directory", path.display()))?;
    let task_id = parse_task_file(&path, candidate)
        .map_err(|error| anyhow!("parsing candidate body {}: {error}", path.display()))?
        .canonical_task_id;
    let scope = AuditScope {
        candidate: Some((&path, candidate)),
        only_claimed_by: Some(std::slice::from_ref(&task_id)),
        resume: None,
    };
    let audit = audit_scoped(cwd, tasks_root, client.as_ref(), waivers, scope).await?;
    let findings = audit
        .findings
        .into_iter()
        .map(|finding| {
            GateFinding::new(
                GateId::WorkflowLintTaskFile,
                finding.text,
                task_id.clone(),
                Some(finding.source_path),
                archon_workflow::RemediationScope::Body,
            )
        })
        .collect();
    Ok((audit.report, findings))
}

/// What one audit reads beyond the task directory, and which verdicts it
/// reports. The default is the set gate: every body on disk, every claim.
#[derive(Default, Clone, Copy)]
pub(super) struct AuditScope<'a> {
    /// A body not yet published: read in place of the file at its path.
    pub(super) candidate: Option<(&'a Path, &'a str)>,
    /// Report only obligations one of these tasks claims. Cluster membership
    /// is unaffected: an obligation's other claimants and named siblings are
    /// still read, so the digest matches the set gate's for the same texts.
    pub(super) only_claimed_by: Option<&'a [String]>,
    /// The budget the critic calls run under and the progress they report;
    /// `None` is unlimited and silent ([`FreezeResume::none`]).
    pub(super) resume: Option<&'a FreezeResume>,
}

/// The set gate's audit with no budget: what the staged gate runs, less its
/// clock. Production goes through [`evaluate_lint_with_fidelity_resumable`].
#[cfg(test)]
pub(super) async fn audit(
    cwd: &Path,
    tasks_root: &Path,
    client: &dyn WorkflowLlmClient,
    waivers: &[ObligationWaiver],
) -> Result<FidelityAudit> {
    audit_scoped(cwd, tasks_root, client, waivers, AuditScope::default()).await
}

async fn audit_scoped(
    cwd: &Path,
    tasks_root: &Path,
    client: &dyn WorkflowLlmClient,
    waivers: &[ObligationWaiver],
    scope: AuditScope<'_>,
) -> Result<FidelityAudit> {
    // Issue 294: the bodies and skeleton are read as one version, before any
    // model call.
    let read = crate::command::workflow_task_set::ChainRead::of(cwd, tasks_root)?;
    let (mut claims, _skipped) = task_requirement_claims_tolerant(tasks_root).map_err(|error| {
        anyhow!(
            "reading task claims under {}: {error}",
            tasks_root.display()
        )
    })?;
    let mut candidate_text = None;
    if let Some((path, text)) = scope.candidate {
        let task = parse_task_file(path, text)
            .map_err(|error| anyhow!("parsing candidate body {}: {error}", path.display()))?;
        let source_path = path.to_string_lossy().into_owned();
        claims.retain(|claim| {
            claim.task_id != task.canonical_task_id && claim.source_path != source_path
        });
        claims.push(crate::command::topology_task_graph::TaskRequirementClaims {
            task_id: task.canonical_task_id.clone(),
            source_path,
            implements: task.implements,
        });
        candidate_text = Some((task.canonical_task_id, text.to_string()));
    }
    let skeleton = read_skeleton_summary(tasks_root)?;
    let skeleton = &skeleton;
    let prd_path = super::coverage::resolve_prd(tasks_root, &claims)?
        .ok_or_else(|| anyhow!("no PRD resolves for {}", tasks_root.display()))?;
    let prd = std::fs::read_to_string(&prd_path)
        .with_context(|| format!("reading PRD {}", prd_path.display()))?;
    let texts = obligation_texts(&prd);
    let mut task_texts = BTreeMap::new();
    let mut claiming: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for claim in &claims {
        let text = match &candidate_text {
            Some((id, text)) if *id == claim.task_id => text.clone(),
            _ => std::fs::read_to_string(&claim.source_path)
                .with_context(|| format!("reading task {}", claim.source_path))?,
        };
        task_texts.insert(
            claim.task_id.clone(),
            (text, PathBuf::from(&claim.source_path)),
        );
        for id in &claim.implements {
            if texts.contains_key(id) {
                claiming
                    .entry(id.clone())
                    .or_default()
                    .insert(claim.task_id.clone());
            }
        }
    }
    drop(read);
    // Issue-43: each obligation is audited over its claimants plus the set
    // tasks a claimant names, so a result deferred to a sibling is read where
    // the sibling states it; a named task that claims nothing is noted.
    // Issue-44: a scoped audit asks only about the obligations its tasks
    // claim, but over the same cluster the set gate would build for them.
    let mut clusters: BTreeMap<Vec<String>, Vec<String>> = BTreeMap::new();
    let mut notes = Vec::new();
    let audited = claiming.iter().filter(|(_, claimants)| {
        scope
            .only_claimed_by
            .is_none_or(|only| only.iter().any(|task| claimants.contains(task)))
    });
    for (id, claimants) in audited {
        let mut tasks: Vec<String> = claimants.iter().cloned().collect();
        for claimant in claimants {
            for named in named_set_tasks(claimant, &task_texts[claimant].0, &task_texts) {
                if !claimants.contains(&named) {
                    notes.push(format!(
                        "obligation {id} is claimed by {claimant}, whose text names {named} as owning the result, but {named} does not claim {id}"
                    ));
                }
                tasks.push(named);
            }
        }
        tasks.sort();
        tasks.dedup();
        clusters.entry(tasks).or_default().push(id.clone());
    }
    // Every cluster is resolved before any is rendered, so the report keeps
    // cluster order while the provider calls run a few at a time: one call
    // took minutes on the live host, and sixteen in sequence is an hour a set
    // gate should not spend when the provider serves them side by side.
    let texts = &texts;
    let inputs: Vec<(Vec<ClaimedObligation>, Vec<ClaimingTask>, String)> = clusters
        .iter()
        .flat_map(|(task_ids, obligation_ids)| {
            let tasks = task_ids
                .iter()
                .map(|id| ClaimingTask {
                    task_id: id.clone(),
                    text: task_texts[id].0.clone(),
                })
                .collect::<Vec<_>>();
            obligation_ids
                .chunks(FIDELITY_MAX_OBLIGATIONS_PER_CALL)
                .map(move |batch| {
                    let obligations = batch
                        .iter()
                        .map(|id| ClaimedObligation {
                            id: id.clone(),
                            text: texts[id].clone(),
                        })
                        .collect::<Vec<_>>();
                    let digest = fidelity_cluster_digest(&obligations, &tasks, skeleton);
                    (obligations, tasks.clone(), digest)
                })
                .collect::<Vec<_>>()
        })
        .collect();
    let unlimited = FreezeResume::none();
    let store = VerdictStore::new(store_dir(cwd), StoreIdentity::of(client));
    let Resolved {
        verdicts: resolved,
        asked,
        cached,
    } = resolve(
        client,
        &store,
        &inputs,
        skeleton,
        scope.resume.unwrap_or(&unlimited),
    )
    .await?;
    let mut report = String::from("\n## obligation fidelity\n");
    let mut findings = Vec::new();
    for ((_, tasks, _), verdicts) in inputs.iter().zip(resolved) {
        let task_ids: Vec<String> = tasks.iter().map(|task| task.task_id.clone()).collect();
        let task_ids = &task_ids;
        for verdict in verdicts {
            if verdict.necessarily_true {
                report.push_str(&format!(
                    "  {}: necessarily true given {}\n",
                    verdict.obligation_id,
                    task_ids.join(", ")
                ));
                continue;
            }
            let claimants: Vec<String> = claiming[&verdict.obligation_id].iter().cloned().collect();
            let text = fidelity_finding(&verdict, &claimants);
            if let Some(waiver) = waivers
                .iter()
                .find(|w| w.obligation_id == verdict.obligation_id)
            {
                report.push_str(&format!(
                    "  WAIVED {text}\n    waived at {} by operator: \"{}\"\n",
                    waiver.waived_at, waiver.reason
                ));
                continue;
            }
            report.push_str(&format!("  BLOCKING {text}\n"));
            let source_path = task_texts
                .get(&verdict.weakest_task_id)
                .map(|(_, path)| path.clone())
                .unwrap_or_else(|| prd_path.clone());
            findings.push(FidelityFinding {
                text,
                subject: verdict.obligation_id,
                source_path,
            });
        }
    }
    for note in &notes {
        report.push_str(&format!("  NOTE {note}\n"));
    }
    report.push_str(&format!(
        "  {} claimed obligation(s) in {} cluster(s), {} call batch(es): {asked} asked of {}, {cached} served from {}\n",
        clusters.values().map(Vec::len).sum::<usize>(),
        clusters.len(),
        inputs.len(),
        client.resolve_model_alias(CRITIC_MODEL_ALIAS),
        store.dir().display()
    ));
    if clusters.is_empty() {
        report.push_str("  no task claims an obligation the PRD defines; nothing to audit.\n");
    }
    Ok(FidelityAudit { report, findings })
}

/// The set's frozen skeleton as the critic reads it. Absent is a legacy set
/// and is said so; present but unreadable is an error, never a silent
/// narrowing of what the critic is told.
fn read_skeleton_summary(tasks_root: &Path) -> Result<SkeletonSummary> {
    let path = tasks_root.join(archon_workflow::task_set_contract::TASK_SKELETON_FILE);
    if !path.exists() {
        return Ok(SkeletonSummary::absent());
    }
    let bytes =
        std::fs::read(&path).with_context(|| format!("reading skeleton {}", path.display()))?;
    let skeleton: archon_workflow::task_skeleton::TaskSkeleton = serde_json::from_slice(&bytes)
        .with_context(|| format!("parsing skeleton {}", path.display()))?;
    Ok(SkeletonSummary::from_skeleton(&skeleton))
}

/// The canonical ids of the other tasks in the set that `text` names — the
/// one hop of the ownership chain (Issue-43). An id counts only whole: a text
/// naming `TASK-A-0021` does not name `TASK-A-002`, and an id outside the set
/// is not a task the audit can read, so it is never pulled in.
fn named_set_tasks(
    own_id: &str,
    text: &str,
    set: &BTreeMap<String, (String, PathBuf)>,
) -> Vec<String> {
    set.keys()
        .filter(|id| id.as_str() != own_id && mentions_whole(text, id))
        .cloned()
        .collect()
}

fn mentions_whole(text: &str, id: &str) -> bool {
    let bounded = |c: Option<char>| c.is_none_or(|c| !c.is_ascii_alphanumeric());
    text.match_indices(id).any(|(start, _)| {
        bounded(text[..start].chars().next_back())
            && bounded(text[start + id.len()..].chars().next())
    })
}

#[cfg(test)]
#[path = "fidelity_tests.rs"]
mod tests;

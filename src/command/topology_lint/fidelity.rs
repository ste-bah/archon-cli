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
//! Obligations are grouped by the exact set of tasks claiming them, so each
//! task's full text travels once per cluster rather than once per obligation.
//! Each cluster's verdicts are cached under the project's `.archon/lint-cache`
//! by a digest of the obligation texts and the task texts: re-running lint over
//! an unchanged set costs nothing, and editing one task re-asks only the
//! clusters that task is in. A false verdict blocks unless the operator has
//! waived that obligation id; the waiver is recorded verbatim in the freeze
//! pin so the override is as auditable as the stamp it overrides.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use archon_workflow::fidelity_audit::{
    ClaimedObligation, ClaimingTask, FidelityVerdict, ObligationWaiver, fidelity_cluster_digest,
    fidelity_finding, fidelity_prompt, parse_fidelity_response,
};
use archon_workflow::llm_client_port::{WorkflowAgentOutcome, WorkflowLlmClient};
use archon_workflow::obligation_ids::obligation_texts;
use archon_workflow::task_set_contract::AcceptancePin;
use futures_util::{StreamExt, TryStreamExt};

use super::LintSource;
use crate::command::topology_task_graph::task_requirement_claims_tolerant;
use crate::command::workflow_gate::{GateEvaluation, GateFinding, GateId};

/// The alias the audit asks for. The critic reads whole task files and is
/// asked to find the sentence that lets a claim go hollow; that is the
/// strongest tier the provider offers, whatever it resolves to.
const CRITIC_MODEL_ALIAS: &str = "opus";
/// One re-ask on a malformed reply, then the failure is operational. A
/// formatting slip often corrects on a second pass; a third pass is spend.
const FIDELITY_ATTEMPTS: usize = 2;
const FIDELITY_CALL_TIMEOUT_SECS: u64 =
    crate::command::workflow_task_set::judge::JUDGE_TIMEOUT_SECS;
/// Clusters in flight at once. Enough to overlap the provider's latency,
/// few enough that a serving endpoint sized for one decomposition is not
/// asked to hold sixteen long prompts at the same time.
const FIDELITY_CONCURRENCY: usize = 4;
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

#[derive(serde::Serialize, serde::Deserialize)]
struct CachedCluster {
    digest: String,
    model: String,
    provider: Option<String>,
    audited_at: String,
    verdicts: Vec<FidelityVerdict>,
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
    let mut evaluation = super::evaluate_lint(cwd, source, mode)?;
    let LintSource::Tasks(path) = source else {
        return Err(anyhow!(
            "obligation fidelity is only defined for --tasks <DIR>: it reads every task body that claims a PRD obligation"
        ));
    };
    let root = super::absolute(cwd, path);
    let outcome = match client {
        Ok(client) => audit(cwd, &root, client.as_ref(), waivers).await,
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

pub(super) async fn audit(
    cwd: &Path,
    tasks_root: &Path,
    client: &dyn WorkflowLlmClient,
    waivers: &[ObligationWaiver],
) -> Result<FidelityAudit> {
    let (claims, _skipped) = task_requirement_claims_tolerant(tasks_root).map_err(|error| {
        anyhow!(
            "reading task claims under {}: {error}",
            tasks_root.display()
        )
    })?;
    let prd_path = super::coverage::resolve_prd(tasks_root, &claims)?
        .ok_or_else(|| anyhow!("no PRD resolves for {}", tasks_root.display()))?;
    let prd = std::fs::read_to_string(&prd_path)
        .with_context(|| format!("reading PRD {}", prd_path.display()))?;
    let texts = obligation_texts(&prd);
    let mut task_texts = BTreeMap::new();
    let mut claiming: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for claim in &claims {
        let text = std::fs::read_to_string(&claim.source_path)
            .with_context(|| format!("reading task {}", claim.source_path))?;
        task_texts.insert(
            claim.task_id.clone(),
            (text, PathBuf::from(&claim.source_path)),
        );
        for id in &claim.implements {
            if texts.contains_key(id) {
                claiming
                    .entry(id.clone())
                    .or_default()
                    .push(claim.task_id.clone());
            }
        }
    }
    let mut clusters: BTreeMap<Vec<String>, Vec<String>> = BTreeMap::new();
    for (id, mut tasks) in claiming {
        tasks.sort();
        tasks.dedup();
        clusters.entry(tasks).or_default().push(id);
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
                    let digest = fidelity_cluster_digest(&obligations, &tasks);
                    (obligations, tasks.clone(), digest)
                })
                .collect::<Vec<_>>()
        })
        .collect();
    let cache = cache_dir(cwd);
    let cache = cache.as_path();
    let mut resolved: Vec<Option<Vec<FidelityVerdict>>> = inputs
        .iter()
        .map(|(_, _, digest)| read_cached(&cache.join(format!("{digest}.json")), digest))
        .collect();
    let cached = resolved.iter().filter(|entry| entry.is_some()).count();
    let asked = resolved.len() - cached;
    let pending = inputs
        .iter()
        .enumerate()
        .filter(|(index, _)| resolved[*index].is_none())
        .map(|(index, (obligations, tasks, digest))| async move {
            let verdicts = ask(client, cache, digest, obligations, tasks).await?;
            write_cached(
                &cache.join(format!("{digest}.json")),
                client,
                digest,
                &verdicts,
            )?;
            Ok::<_, anyhow::Error>((index, verdicts))
        });
    let answered: Vec<(usize, Vec<FidelityVerdict>)> = futures_util::stream::iter(pending)
        .buffer_unordered(FIDELITY_CONCURRENCY)
        .try_collect()
        .await?;
    for (index, verdicts) in answered {
        resolved[index] = Some(verdicts);
    }
    let mut report = String::from("\n## obligation fidelity\n");
    let mut findings = Vec::new();
    for ((_, tasks, _), verdicts) in inputs.iter().zip(resolved) {
        let task_ids: Vec<String> = tasks.iter().map(|task| task.task_id.clone()).collect();
        let task_ids = &task_ids;
        let verdicts = verdicts.expect("every batch resolved from cache or critic");
        for verdict in verdicts {
            if verdict.necessarily_true {
                report.push_str(&format!(
                    "  {}: necessarily true given {}\n",
                    verdict.obligation_id,
                    task_ids.join(", ")
                ));
                continue;
            }
            let text = fidelity_finding(&verdict, task_ids);
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
    report.push_str(&format!(
        "  {} claimed obligation(s) in {} cluster(s), {} call batch(es): {asked} asked of {}, {cached} served from {}\n",
        clusters.values().map(Vec::len).sum::<usize>(),
        clusters.len(),
        inputs.len(),
        client.resolve_model_alias(CRITIC_MODEL_ALIAS),
        cache_dir(cwd).display()
    ));
    if clusters.is_empty() {
        report.push_str("  no task claims an obligation the PRD defines; nothing to audit.\n");
    }
    Ok(FidelityAudit { report, findings })
}

/// Ask once, re-ask once on a malformed reply, and keep every rejected reply
/// under `rejected/` in the cache directory — the operator who is told "no
/// usable verdict" needs to see what the critic actually said.
async fn ask(
    client: &dyn WorkflowLlmClient,
    cache: &Path,
    digest: &str,
    obligations: &[ClaimedObligation],
    tasks: &[ClaimingTask],
) -> Result<Vec<FidelityVerdict>> {
    let prompt = fidelity_prompt(obligations, tasks);
    let mut last = String::from("never asked");
    for attempt in 1..=FIDELITY_ATTEMPTS {
        let outcome = tokio::time::timeout(
            Duration::from_secs(FIDELITY_CALL_TIMEOUT_SECS),
            client.send_message_with_temperature(
                vec![serde_json::json!({ "role": "user", "content": prompt.clone() })],
                Vec::new(),
                Vec::new(),
                CRITIC_MODEL_ALIAS,
                0.0,
            ),
        )
        .await
        .map_err(|_| anyhow!("fidelity critic timed out after {FIDELITY_CALL_TIMEOUT_SECS}s"))?
        .map_err(anyhow::Error::new)?;
        // A truncated reply is not re-asked: the budget that cut it off has
        // not changed, and partial JSON is never repaired into a verdict.
        require_complete(&outcome)?;
        let document = crate::command::workflow_freeze_candidate::candidate_document(
            outcome.content.trim().as_bytes(),
        );
        let document = String::from_utf8_lossy(&document);
        match parse_fidelity_response(&document, obligations, tasks) {
            Ok(verdicts) => return Ok(verdicts),
            Err(error) => {
                let rejected = cache.join("rejected");
                let path = rejected.join(format!("{digest}-attempt-{attempt}.txt"));
                let kept = std::fs::create_dir_all(&rejected)
                    .and_then(|()| std::fs::write(&path, &outcome.content))
                    .map(|()| path.display().to_string())
                    .unwrap_or_else(|error| format!("not kept: {error}"));
                last = format!("{error} (reply kept at {kept})");
            }
        }
    }
    Err(anyhow!(
        "fidelity critic returned no usable verdict for {:?} after {FIDELITY_ATTEMPTS} attempts: {last}",
        obligations
            .iter()
            .map(|o| o.id.as_str())
            .collect::<Vec<_>>()
    ))
}

fn require_complete(outcome: &WorkflowAgentOutcome) -> Result<()> {
    match outcome.stop_reason.as_deref() {
        Some("end_turn" | "stop" | "completed") => Ok(()),
        Some(reason) => Err(anyhow!(
            "fidelity critic reply ended with stop reason '{reason}'; a truncated verdict is never parsed"
        )),
        None => Err(anyhow!(
            "fidelity critic returned no finish reason; refusing to parse a possibly truncated verdict"
        )),
    }
}

fn cache_dir(cwd: &Path) -> PathBuf {
    cwd.join(".archon").join("lint-cache").join("fidelity")
}

fn read_cached(path: &Path, digest: &str) -> Option<Vec<FidelityVerdict>> {
    let cached: CachedCluster = serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
    (cached.digest == digest).then_some(cached.verdicts)
}

fn write_cached(
    path: &Path,
    client: &dyn WorkflowLlmClient,
    digest: &str,
    verdicts: &[FidelityVerdict],
) -> Result<()> {
    let parent = path.parent().expect("cache path has a parent");
    std::fs::create_dir_all(parent)
        .with_context(|| format!("creating fidelity cache {}", parent.display()))?;
    let cached = CachedCluster {
        digest: digest.to_string(),
        model: client.resolve_model_alias(CRITIC_MODEL_ALIAS),
        provider: client.provider_id(),
        audited_at: chrono::Utc::now().to_rfc3339(),
        verdicts: verdicts.to_vec(),
    };
    std::fs::write(path, serde_json::to_vec_pretty(&cached)?)
        .with_context(|| format!("writing fidelity cache {}", path.display()))
}

/// `--waive-obligation <ID>… --waive-reason <TEXT>` as recorded waivers.
pub(crate) fn waivers_from_flags(
    ids: &[String],
    reason: Option<&str>,
) -> Result<Vec<ObligationWaiver>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let reason = reason
        .map(str::trim)
        .filter(|reason| !reason.is_empty())
        .ok_or_else(|| anyhow!("--waive-obligation requires --waive-reason \"<text>\"; a waiver without a reason is not auditable"))?;
    let waived_at = chrono::Utc::now().to_rfc3339();
    Ok(ids
        .iter()
        .map(|id| ObligationWaiver {
            obligation_id: id.trim().to_string(),
            reason: reason.to_string(),
            waived_at: waived_at.clone(),
            binary_commit: env!("ARCHON_GIT_HASH").into(),
        })
        .collect())
}

/// Record waivers verbatim in the task set's freeze pin, replacing an earlier
/// waiver for the same id. A set that was never frozen has no pin to carry
/// the record, so the waiver is refused rather than kept somewhere unaudited.
pub(crate) fn record_waivers(
    cwd: &Path,
    tasks_root: &Path,
    waivers: &[ObligationWaiver],
) -> Result<PathBuf> {
    let pin_path = crate::command::workflow_task_set::acceptance_pin_path(cwd, tasks_root);
    let mut pin: AcceptancePin = serde_json::from_slice(&std::fs::read(&pin_path).with_context(|| {
        format!(
            "no acceptance freeze pin at {} for {}; a waiver attaches to a frozen task set, so freeze first",
            pin_path.display(),
            tasks_root.display()
        )
    })?)
    .with_context(|| format!("acceptance pin {} is malformed", pin_path.display()))?;
    for waiver in waivers {
        pin.fidelity_waivers
            .retain(|existing| existing.obligation_id != waiver.obligation_id);
        pin.fidelity_waivers.push(waiver.clone());
    }
    let temp = pin_path.with_extension("json.tmp");
    std::fs::write(&temp, serde_json::to_vec_pretty(&pin)?)
        .with_context(|| format!("writing {}", temp.display()))?;
    std::fs::rename(&temp, &pin_path)
        .with_context(|| format!("publishing {}", pin_path.display()))?;
    Ok(pin_path)
}

/// The waivers a frozen task set already carries; none when it has no pin.
pub(crate) fn recorded_waivers(cwd: &Path, tasks_root: &Path) -> Vec<ObligationWaiver> {
    let pin_path = crate::command::workflow_task_set::acceptance_pin_path(cwd, tasks_root);
    std::fs::read(&pin_path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<AcceptancePin>(&bytes).ok())
        .map(|pin| pin.fidelity_waivers)
        .unwrap_or_default()
}

#[cfg(test)]
#[path = "fidelity_tests.rs"]
mod tests;

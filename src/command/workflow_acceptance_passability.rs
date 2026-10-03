//! A check proven able to fail must also be able to pass (Issue 275).
//!
//! The pre-implementation probe (`workflow_acceptance_executability`)
//! counts ANY failure on the tree before implementation as the proof that a
//! check can fail. A check whose failure there comes from its own setup --
//! a fixture the product refuses, a flag or a minimum the product enforces
//! and a correct implementation keeps -- passes that proof, is frozen, and
//! can never pass. The probe already holds the output that states the
//! broken rule; it used to be thrown away.
//!
//! So every judge-accepted check that failed on the baseline is judged once
//! more, shown that run's exit code and its bounded stderr and stdout: is
//! the failure explained by the criterion's feature being absent, or by the
//! check's own setup breaking a rule a correct implementation keeps? A check
//! that cannot pass as written is refuted -- never published, like every
//! refutation -- and its finding carries the baseline output, so the author
//! sees the rule it broke.
//!
//! Order: the adversarial judge first (its batch, its saved verdicts and
//! their keys are unchanged), then the probe (which runs only the checks
//! that judge accepted), then this pass over the accepted checks that
//! failed on the baseline. Each verdict here is saved per check under the
//! digest of its exact input -- this instruction, the check, its baseline
//! output, the model and the provider -- so a verdict given without that
//! output (or on other output) never answers for it, and a retry that
//! reuses the saved probe verdicts reuses these too.

use std::collections::BTreeMap;
use std::time::Duration;

use archon_workflow::acceptance_scratch::CheckResult;
use archon_workflow::task_set_contract::{AcceptanceCriterion, JudgeDecision};
use serde::{Deserialize, Serialize};

use super::coverage_gate::PreImplementation;
use super::judge::{judge_prompted, require_judged_prose};
use super::*;
use crate::command::workflow_freeze_budget::{FREEZE_CACHE_DIR, FreezeIncomplete, FreezeResume};

/// What every finding for a check that cannot pass says, after its id.
pub(super) const CANNOT_PASS: &str = "cannot pass as written";
/// Bytes of the baseline stderr, then stdout, the judge and the author see.
const STDERR_BYTES: usize = 2_000;
const STDOUT_BYTES: usize = 600;
/// The judge's prose in a finding, flattened to one line and capped.
const PROSE_CAP: usize = 400;
/// The least budget the pass starts with: on the live deployment the
/// adversarial judge took 26 minutes for 57 checks. With less left the
/// freeze stops resumable, every probe verdict saved, and its retry starts
/// here with a full budget.
const EVIDENCE_JUDGE_WINDOW_SECS: u64 = 30 * 60;
const SCHEMA: u32 = 1;

const INSTRUCTION: &str = "Each acceptance check below already ran once on the repository tree BEFORE any implementation of its criterion, and failed there; `baseline` gives that run's exit code and the end of its stderr and stdout, bounded. Failing there is required, but it shows the check sound only when the failure comes from what the implementation must still add or change. For each id decide what the output shows. Verdict \"accepted\": the failure is explained by the criterion's feature, data or behaviour being absent, partial or wrong on that tree, so a correct implementation of the criterion can make the check pass. Verdict \"refuted\": the check cannot pass as written, because the output shows the check's OWN setup -- data or fixtures it creates, inputs, arguments or flags it passes, a threshold it asks for -- refused by a rule the product enforces (a validation, gate, minimum, limit, required flag or provenance requirement) that a correct implementation of the criterion keeps: neither the criterion nor a requirement the check covers asks to remove or relax that rule, so the check would fail the same way once the work is done. Refute only when the output itself states the refusing rule and the check's own setup is what breaks it; when the output is ambiguous, or the rule is one the criterion asks to add or change, accept. Return JSON only as {\"decisions\":[{\"id\":\"...\",\"verdict\":\"accepted|refuted\",\"counterexample\":\"...\",\"reason\":\"...\"}]} with exactly one decision for every input id and no extra ids. counterexample names the refusing rule as the output states it, or says that no rule refused the check's own setup; reason is one sentence: what in the output decides the verdict and, when refuted, what the check must change in its own setup to meet that rule. Every string must be a single line with newlines escaped as \\n; emit the JSON document alone.";

/// One check's run on the baseline, bounded, as the judge and author see it.
#[derive(Clone, Serialize)]
struct Evidence {
    commit: String,
    exit_code: Option<i32>,
    stderr: String,
    stdout: String,
}

impl Evidence {
    fn of(commit: &str, result: &CheckResult) -> Self {
        let excerpt = archon_workflow::failure_evidence::failure_evidence;
        Self {
            commit: commit.to_string(),
            exit_code: result.exit_code,
            stderr: excerpt(&result.stderr, STDERR_BYTES),
            stdout: excerpt(&result.stdout, STDOUT_BYTES),
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
struct Decision {
    verdict: JudgeDecision,
    counterexample: String,
    reason: String,
    sampling: Option<serde_json::Value>,
}

#[derive(Serialize, Deserialize)]
struct Saved {
    schema: u32,
    key: String,
    decision: Decision,
}

/// Saved verdicts, one file per check input digest.
struct Store {
    dir: PathBuf,
}

impl Store {
    fn load(&self, key: &str) -> Option<Decision> {
        let bytes = std::fs::read(self.dir.join(format!("{key}.json"))).ok()?;
        let saved: Saved = serde_json::from_slice(&bytes).ok()?;
        (saved.schema == SCHEMA && saved.key == key).then_some(saved.decision)
    }

    fn save(&self, key: &str, decision: &Decision) -> bool {
        let saved = Saved {
            schema: SCHEMA,
            key: key.to_string(),
            decision: decision.clone(),
        };
        let staging = self.dir.join(format!(".{key}.{}.tmp", std::process::id()));
        let written = std::fs::create_dir_all(&self.dir).is_ok()
            && serde_json::to_vec(&saved)
                .is_ok_and(|bytes| std::fs::write(&staging, bytes).is_ok())
            && std::fs::rename(&staging, self.dir.join(format!("{key}.json"))).is_ok();
        if !written {
            let _ = std::fs::remove_file(&staging);
        }
        written
    }
}

fn entries(contract: &AcceptanceContract) -> impl Iterator<Item = &AcceptanceCriterion> {
    contract.acceptance.iter().chain(&contract.supplementary)
}

/// The digest of everything the verdict on `entry` is a function of.
fn key(client: &dyn WorkflowLlmClient, entry: &AcceptanceCriterion, evidence: &Evidence) -> String {
    let input = serde_json::json!([
        "acceptance-passability-v1",
        INSTRUCTION,
        entry.id,
        entry.criterion,
        entry.check,
        entry.covers,
        evidence,
        client.resolve_model_alias("sonnet"),
        client.provider_id(),
    ]);
    content_digest(input.to_string().as_bytes())
}

fn prompt(contract: &AcceptanceContract, evidence: &BTreeMap<&str, &Evidence>) -> Result<String> {
    let checks = entries(contract)
        .map(|entry| {
            let mut check = serde_json::json!({
                "id": entry.id,
                "criterion": entry.criterion,
                "check": entry.check,
            });
            if !entry.covers.is_empty() {
                check["covers"] = serde_json::json!(entry.covers);
            }
            check["baseline"] = serde_json::json!(evidence.get(entry.id.as_str()));
            check
        })
        .collect::<Vec<_>>();
    Ok(format!(
        "{INSTRUCTION} Checks: {}",
        serde_json::to_string(&checks)?
    ))
}

/// Judge prose or program output inside a finding: it can never name
/// another check to the router that reads `check '<id>'` out of findings.
fn inert(text: &str) -> String {
    text.replace("check '", "check `")
}

fn prose(text: &str) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let flat = match flat.char_indices().nth(PROSE_CAP) {
        Some((cut, _)) => format!("{}…", &flat[..cut]),
        None => flat,
    };
    inert(&flat)
}

fn finding_text(id: &str, evidence: &Evidence, decision: &Decision) -> String {
    let short: String = evidence.commit.chars().take(12).collect();
    let exit = evidence.exit_code.map_or_else(
        || "no exit status".to_string(),
        |code| format!("exit {code}"),
    );
    let output = |text: &str| {
        if text.is_empty() {
            "(empty)".to_string()
        } else {
            inert(text)
        }
    };
    format!(
        "check '{id}': {CANNOT_PASS}: on the tree before any implementation ({short}) it failed ({exit}), and the host judge found that failure caused by the check's own setup breaking a rule a correct implementation keeps, not by its criterion's feature being absent, so it would fail the same way once the work is done; rule: \"{}\"; reason: \"{}\"; change the check's own setup (the data, fixtures, inputs or flags it supplies) so that it meets that rule, and keep every assertion of its criterion. Its stderr on that tree:\n{}\nIts stdout on that tree:\n{}",
        prose(&decision.counterexample),
        prose(&decision.reason),
        output(&evidence.stderr),
        output(&evidence.stdout),
    )
}

/// The probe's findings, and one more for each accepted check of `contract`
/// that failed on the baseline in a way it cannot pass as written; each
/// such check is refuted in `contract` (see the module docs).
pub(super) async fn judge_baseline_failures(
    project_root: &Path,
    tasks_root: &Path,
    client: &dyn WorkflowLlmClient,
    contract: &mut AcceptanceContract,
    probed: PreImplementation,
    resume: &FreezeResume,
) -> Result<Vec<GateFinding>> {
    let PreImplementation {
        mut findings,
        baseline_failures,
    } = probed;
    let Some((commit, failures)) = baseline_failures else {
        return Ok(findings);
    };
    // A check the probe already sends back is its author's either way.
    let flagged: BTreeSet<&str> = findings.iter().map(|f| f.subject.as_str()).collect();
    let candidates: Vec<(String, Evidence, String)> = entries(contract)
        .filter(|entry| entry.judgment.verdict == JudgeDecision::Accepted)
        .filter(|entry| !flagged.contains(entry.id.as_str()))
        .filter_map(|entry| {
            let evidence = Evidence::of(&commit, failures.get(&entry.id)?);
            let key = key(client, entry, &evidence);
            Some((entry.id.clone(), evidence, key))
        })
        .collect();
    if candidates.is_empty() {
        return Ok(findings);
    }
    let store = (resume.persist).then(|| Store {
        dir: project_root.join(FREEZE_CACHE_DIR).join("passability"),
    });
    let mut decided: BTreeMap<String, Decision> = BTreeMap::new();
    for (id, _, key) in &candidates {
        if let Some(decision) = store.as_ref().and_then(|store| store.load(key)) {
            resume.progress.reused(true);
            decided.insert(id.clone(), decision);
        }
    }
    let pending: Vec<&(String, Evidence, String)> = (candidates.iter())
        .filter(|(id, _, _)| !decided.contains_key(id))
        .collect();
    eprintln!(
        "acceptance judge: {} accepted check(s) failed on the pre-implementation tree; judging whether each can pass ({} verdict(s) reused)",
        candidates.len(),
        decided.len()
    );
    if !pending.is_empty() {
        let mut judged = ask(client, contract, &pending, resume).await?;
        for (id, _, key) in pending {
            let decision = judged.remove(id).expect("one decision per judged id");
            if store
                .as_ref()
                .is_some_and(|store| store.save(key, &decision))
            {
                resume.progress.saved(true);
            }
            decided.insert(id.clone(), decision);
        }
    }
    let contract_path = tasks_root.join(ACCEPTANCE_CONTRACT_FILE);
    for (id, evidence, _) in &candidates {
        let decision = &decided[id];
        if decision.verdict == JudgeDecision::Accepted {
            continue;
        }
        let entry = (contract.acceptance.iter_mut())
            .chain(&mut contract.supplementary)
            .find(|entry| &entry.id == id)
            .expect("a candidate is an entry of the contract");
        entry.judgment.verdict = JudgeDecision::Refuted;
        entry.judgment.reason = format!("{CANNOT_PASS}: {}", decision.reason);
        entry
            .judgment
            .counterexample
            .clone_from(&decision.counterexample);
        entry.judgment.host_call_id = format!("acceptance-passability-batch:{id}");
        entry.judgment.sampling.clone_from(&decision.sampling);
        findings.push(GateFinding::new(
            GateId::FreezeAcceptance,
            finding_text(id, evidence, decision),
            id.clone(),
            Some(contract_path.clone()),
            archon_workflow::RemediationScope::CandidateArtifact,
        ));
    }
    Ok(findings)
}

/// One batch over `pending`, within the freeze's budget: too little left to
/// start, or a call the budget cuts off, ends the freeze resumable.
async fn ask(
    client: &dyn WorkflowLlmClient,
    contract: &AcceptanceContract,
    pending: &[&(String, Evidence, String)],
    resume: &FreezeResume,
) -> Result<BTreeMap<String, Decision>> {
    let ids: BTreeSet<&str> = pending.iter().map(|(id, _, _)| id.as_str()).collect();
    let incomplete = || {
        let ids = ids.iter().map(|id| id.to_string()).collect();
        anyhow::Error::from(FreezeIncomplete::awaiting_judge(
            &resume.budget,
            &resume.progress,
            ids,
        ))
    };
    let left = resume.budget.remaining_secs();
    if left.is_some_and(|left| left < EVIDENCE_JUDGE_WINDOW_SECS) {
        return Err(incomplete());
    }
    let mut subset = contract.clone();
    subset
        .acceptance
        .retain(|entry| ids.contains(entry.id.as_str()));
    (subset.supplementary).retain(|entry| ids.contains(entry.id.as_str()));
    let evidence: BTreeMap<&str, &Evidence> = (pending.iter())
        .map(|(id, evidence, _)| (id.as_str(), evidence))
        .collect();
    let task = prompt(&subset, &evidence)?;
    let call = judge_prompted(client, subset, &task, "sonnet", require_judged_prose);
    let judged = match left {
        Some(left) => tokio::time::timeout(Duration::from_secs(left), call)
            .await
            .map_err(|_| incomplete())??,
        None => call.await?,
    };
    Ok(entries(&judged)
        .map(|entry| {
            let decision = Decision {
                verdict: entry.judgment.verdict,
                counterexample: entry.judgment.counterexample.clone(),
                reason: entry.judgment.reason.clone(),
                sampling: entry.judgment.sampling.clone(),
            };
            (entry.id.clone(), decision)
        })
        .collect())
}

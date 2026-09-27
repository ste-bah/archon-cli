//! Bounded re-author and re-judge of named acceptance checks.
//!
//! A check the judge refuted can never run (`acceptance_world::resolve_command`
//! refuses it), so publishing one only schedules implementation work that
//! cannot turn it green. The only party that can repair such a check is its
//! author, told what the judge said. This module is that loop for the host:
//! each named entry goes back to a read-only author agent with the judge's
//! reason and counterexample, the reply is re-judged, and after
//! [`REAUTHOR_ATTEMPTS`] rounds a check still not accepted fails the whole
//! operation with a per-check report. Every entry not named is returned
//! exactly as it was, judgment included.
//!
//! The authoring rules are the decomposition author's (`workflow_decompose_v1.js`,
//! acceptance phase): same entry shapes, same grounding, same "fail when the
//! criterion is false" rule. Nothing here knows a PRD.

use std::collections::BTreeMap;
use std::time::Duration;

use archon_core::agents::harness::{ACCEPTANCE_REAUTHOR_AGENT, HOST_READ_ONLY_TOOLS};
use archon_workflow::llm_client_port::{
    WorkflowAgentCall, WorkflowAgentSpec, WorkflowAgentToolAccess,
};
use archon_workflow::task_set_contract::{
    AcceptanceCheck, AcceptanceCriterion, JudgeDecision, JudgeVerdict, refuted_check_message,
};

use super::*;

/// Author-then-judge rounds per named check before the operation fails.
pub(crate) const REAUTHOR_ATTEMPTS: usize = 3;

/// The exact-tool marker the subagent adapters honor (`archon-pipeline`
/// `subagent_adapter.rs`, `workflow_live_v2_client.rs`): the author reads the
/// repository and the PRD and writes nothing. The tools are the host agent's
/// own definition (`archon_core::agents::harness`), so the call's allowlist
/// and the resolved agent cannot drift apart.
const EXACT_TOOL_POLICY_MARKER: &str = "__ARCHON_EXACT_TOOLS__";
const AUTHOR_TOOLS: [&str; 3] = HOST_READ_ONLY_TOOLS;

/// The two entry shapes the decomposition author is shown.
const ENTRY_SHAPES: &str = r#"[{"id":"<exact acceptance id>","criterion":"","check":{"kind":"floor","contract":{"kind":"<deliverable kind>","artifact_path":"<repository-relative artifact path>","artifact_format":"json","required_true_fields":["<field that must be true>"],"typed_verifier_command":"<command that exercises the deliverable and fails when the criterion is false>"}},"gap_permitted":false,"judgment":{"verdict":"accepted","counterexample":"","reason":"","host_call_id":""}},{"id":"<exact acceptance id>","criterion":"","check":{"kind":"command","command":"<shell command that exercises the deliverable and exits non-zero when the criterion is false>","cwd":"project_root"},"gap_permitted":false,"judgment":{"verdict":"accepted","counterexample":"","reason":"","host_call_id":""}}]"#;

/// Where the author reads.
pub(crate) struct AuthorScope {
    pub(crate) prd_path: PathBuf,
    pub(crate) project_root: PathBuf,
    pub(crate) repository_root: PathBuf,
}

impl AuthorScope {
    /// The repository the task set was decomposed against (its
    /// `repository.lock`), else the project root.
    pub(crate) fn for_task_set(project_root: &Path, tasks_root: &Path, prd_path: &Path) -> Self {
        let repository_root =
            archon_workflow::repository_record::read_repository_record(tasks_root)
                .ok()
                .flatten()
                .map(|record| PathBuf::from(record.repository_root))
                .filter(|root| root.is_dir())
                .unwrap_or_else(|| project_root.to_path_buf());
        Self {
            prd_path: prd_path.to_path_buf(),
            project_root: project_root.to_path_buf(),
            repository_root,
        }
    }
}

/// One line per named check: its id and what the judge said about it.
pub(crate) fn not_accepted_lines(
    contract: &AcceptanceContract,
    ids: &BTreeSet<String>,
) -> Vec<String> {
    contract
        .acceptance
        .iter()
        .chain(&contract.supplementary)
        .filter(|entry| ids.contains(&entry.id))
        .map(|entry| {
            refuted_check_message(
                &entry.id,
                &entry.judgment.reason,
                &entry.judgment.counterexample,
            )
        })
        .collect()
}

/// The per-check refusal every non-accepted publication reports.
pub(crate) fn not_accepted_report(
    contract: &AcceptanceContract,
    ids: &BTreeSet<String>,
    attempts: usize,
) -> String {
    report(&not_accepted_lines(contract, ids), attempts)
}

fn report(lines: &[String], attempts: usize) -> String {
    let after = if attempts == 0 {
        String::new()
    } else {
        format!(" after {attempts} re-author attempt(s)")
    };
    format!(
        "acceptance contract not published: {} check(s) not accepted by the judge{after}; a check the judge did not accept can never run, so nothing was written:\n  - {}",
        lines.len(),
        lines.join("\n  - ")
    )
}

fn entry<'a>(contract: &'a AcceptanceContract, id: &str) -> Option<&'a AcceptanceCriterion> {
    contract
        .acceptance
        .iter()
        .chain(&contract.supplementary)
        .find(|entry| entry.id == id)
}

fn entry_mut<'a>(
    contract: &'a mut AcceptanceContract,
    id: &str,
) -> Option<&'a mut AcceptanceCriterion> {
    contract
        .acceptance
        .iter_mut()
        .chain(&mut contract.supplementary)
        .find(|entry| entry.id == id)
}

/// Re-author and re-judge exactly `ids` in `contract`, bounded. Returns the
/// contract with those entries replaced by accepted ones and every other
/// entry untouched, or the per-check report of what is still not accepted.
pub(crate) async fn reauthor(
    client: &dyn WorkflowLlmClient,
    contract: &AcceptanceContract,
    ids: &BTreeSet<String>,
    scope: &AuthorScope,
    judge_model: &str,
) -> Result<AcceptanceContract> {
    let mut feedback: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for id in ids {
        let frozen = entry(contract, id)
            .ok_or_else(|| anyhow!("check '{id}' is not in the acceptance contract"))?;
        let first = if frozen.judgment.verdict == JudgeDecision::Accepted {
            format!(
                "check '{id}' was named for re-authoring; replace it with a check that fails whenever the criterion is false"
            )
        } else {
            refuted_check_message(id, &frozen.judgment.reason, &frozen.judgment.counterexample)
        };
        feedback.insert(id.clone(), vec![first]);
    }
    let mut working = contract.clone();
    let mut pending = ids.clone();
    for attempt in 1..=REAUTHOR_ATTEMPTS {
        let mut authored = Vec::new();
        for id in &pending {
            let frozen = entry(contract, id).expect("named ids were checked above");
            let notes = feedback.get_mut(id).expect("feedback seeded per id");
            let reply = author_entry(client, scope, frozen, notes, attempt).await?;
            match candidate_entry(&reply, frozen)
                .and_then(|candidate| check_defects(&working, candidate))
            {
                Ok(candidate) => authored.push(candidate),
                Err(reason) => notes.push(reason),
            }
        }
        if !authored.is_empty() {
            let mut subset = contract.clone();
            let acceptance_ids: BTreeSet<_> =
                contract.acceptance.iter().map(|e| e.id.clone()).collect();
            let (acceptance, supplementary): (Vec<_>, Vec<_>) = authored
                .into_iter()
                .partition(|candidate| acceptance_ids.contains(&candidate.id));
            subset.acceptance = acceptance;
            subset.supplementary = supplementary;
            let judged = judge::judge_entries(client, subset, judge_model).await?;
            for candidate in judged.acceptance.into_iter().chain(judged.supplementary) {
                if candidate.judgment.verdict == JudgeDecision::Accepted {
                    pending.remove(&candidate.id);
                    let id = candidate.id.clone();
                    *entry_mut(&mut working, &id).expect("named id") = candidate;
                } else if let Some(notes) = feedback.get_mut(&candidate.id) {
                    notes.push(refuted_check_message(
                        &candidate.id,
                        &candidate.judgment.reason,
                        &candidate.judgment.counterexample,
                    ));
                }
            }
        }
        if pending.is_empty() {
            return Ok(working);
        }
    }
    let lines = pending
        .iter()
        .map(|id| {
            feedback
                .get(id)
                .and_then(|notes| notes.last())
                .cloned()
                .unwrap_or_default()
        })
        .collect::<Vec<_>>();
    Err(anyhow!("{}", report(&lines, REAUTHOR_ATTEMPTS)))
}

/// The host-owned parts of a re-authored entry are never the author's:
/// criterion text and gap declaration stay frozen, the judgment is the
/// judge's. A reply that is not one entry for this id is the author's defect.
fn candidate_entry(
    reply: &str,
    frozen: &AcceptanceCriterion,
) -> std::result::Result<AcceptanceCriterion, String> {
    let document =
        crate::command::workflow_freeze_candidate::candidate_document(reply.trim().as_bytes());
    let mut value: serde_json::Value = serde_json::from_slice(&document).map_err(|error| {
        format!(
            "the reply for check '{}' is not one JSON entry ({error}); return the raw JSON object alone",
            frozen.id
        )
    })?;
    if value["id"] != serde_json::json!(frozen.id)
        && let Some([only]) = value["acceptance"].as_array().map(Vec::as_slice)
        && only["id"] == serde_json::json!(frozen.id)
    {
        value = only.clone();
    }
    if value["id"] != serde_json::json!(frozen.id) {
        return Err(format!(
            "the reply is not an entry with id '{}'; return exactly that entry",
            frozen.id
        ));
    }
    let check: AcceptanceCheck =
        serde_json::from_value(value["check"].clone()).map_err(|error| {
            format!(
                "check '{}': the entry's check does not match either shape ({error})",
                frozen.id
            )
        })?;
    if check == frozen.check {
        return Err(format!(
            "check '{}': the reply repeats the check the judge did not accept; change it so it fails in the counterexample state",
            frozen.id
        ));
    }
    Ok(AcceptanceCriterion {
        id: frozen.id.clone(),
        criterion: frozen.criterion.clone(),
        check,
        gap_permitted: frozen.gap_permitted,
        judgment: JudgeVerdict {
            verdict: JudgeDecision::Refuted,
            counterexample: String::new(),
            reason: String::new(),
            host_call_id: String::new(),
            sampling: None,
        },
    })
}

/// A candidate whose check carries a host-verifiable defect never reaches
/// the judge: the defect goes back to the author as the finding it is.
fn check_defects(
    working: &AcceptanceContract,
    candidate: AcceptanceCriterion,
) -> std::result::Result<AcceptanceCriterion, String> {
    let mut probe = working.clone();
    *entry_mut(&mut probe, &candidate.id).expect("named id") = candidate.clone();
    let prefix = format!("{}.check", candidate.id);
    let defects = acceptance_policy_findings(&probe)
        .into_iter()
        .filter(|finding| finding.field == prefix)
        .map(|finding| finding.message)
        .collect::<Vec<_>>();
    if defects.is_empty() {
        Ok(candidate)
    } else {
        Err(defects.join("; "))
    }
}

fn author_prompt(
    scope: &AuthorScope,
    frozen: &AcceptanceCriterion,
    notes: &[String],
    attempt: usize,
) -> String {
    let current = serde_json::json!({
        "id": frozen.id,
        "criterion": frozen.criterion,
        "check": frozen.check,
        "gap_permitted": frozen.gap_permitted,
    });
    [
        format!(
            "Re-author exactly one acceptance entry of a frozen acceptance contract: {}. The host judge did not accept the current entry, so it can never run. Author ONLY this entry, not the whole contract.",
            frozen.id
        ),
        format!("Read the PRD at {}.", scope.prd_path.display()),
        format!(
            "The code repository is {}. It is the ONLY place to verify source paths, test names, module layout and whether a file exists: a repository path you name must be one you observed there. Never descend into dependency, build-output or earlier-run directories.",
            scope.repository_root.display()
        ),
        format!(
            "{} is the project root. It holds the PRD and the task root; read source under the repository root alone.",
            scope.project_root.display()
        ),
        "Return one JSON object with id, criterion, check, gap_permitted, judgment. The two examples below are ENTRIES showing the two check shapes; your reply is one such entry and nothing around it.".to_string(),
        ENTRY_SHAPES.to_string(),
        "A check must exercise the deliverable and fail when its criterion is false, not merely match usage text or assert that a file exists. The host judges the entry adversarially against the criterion.".to_string(),
        format!(
            "Use the exact id {}. Criterion and judgment are host-owned placeholders; gap_permitted stays {}.",
            frozen.id, frozen.gap_permitted
        ),
        format!("The entry being replaced: {current}"),
        format!(
            "Attempt {attempt} of {REAUTHOR_ATTEMPTS}. Findings to fix, oldest first:\n- {}",
            notes.join("\n- ")
        ),
        "Your entire reply must be the entry itself: the raw JSON object, starting with { and ending with }. Emit no prose, no explanation, no headings and no Markdown code fences before or after it.".to_string(),
        "Do not run commands or write files.".to_string(),
    ]
    .join("\n")
}

async fn author_entry(
    client: &dyn WorkflowLlmClient,
    scope: &AuthorScope,
    frozen: &AcceptanceCriterion,
    notes: &[String],
    attempt: usize,
) -> Result<String> {
    let prompt = author_prompt(scope, frozen, notes, attempt);
    let call = WorkflowAgentCall {
        session_id: format!(
            "{ACCEPTANCE_REAUTHOR_AGENT}-{}-{}",
            frozen.id,
            uuid::Uuid::new_v4()
        ),
        task: prompt.clone(),
        cwd: Some(scope.repository_root.clone()),
        ordinal: 0,
        attempt,
        agent: WorkflowAgentSpec {
            // A host agent registered in every project: a key only a project's
            // `.archon/agents` defines fails to launch everywhere else.
            key: ACCEPTANCE_REAUTHOR_AGENT.into(),
            display_name: "acceptance reauthor".into(),
            model: "sonnet".into(),
            phase: 0,
            critical: false,
            parallelizable: false,
            quality_threshold: 0.5,
            tool_access: WorkflowAgentToolAccess::ReadOnly,
        },
        messages: vec![serde_json::json!({ "role": "user", "content": prompt })],
        system: Vec::new(),
        tools: Vec::new(),
        allowed_tools: std::iter::once(EXACT_TOOL_POLICY_MARKER)
            .chain(AUTHOR_TOOLS)
            .map(str::to_string)
            .collect(),
        timeout_secs: Some(judge::JUDGE_TIMEOUT_SECS),
        disable_auto_background: true,
        write_roots: Vec::new(),
        provider_env: None,
    };
    let outcome = tokio::time::timeout(
        Duration::from_secs(judge::JUDGE_TIMEOUT_SECS),
        client.run_agent(call),
    )
    .await
    .map_err(|_| {
        anyhow!(
            "acceptance re-author for '{}' timed out after {}s",
            frozen.id,
            judge::JUDGE_TIMEOUT_SECS
        )
    })?
    .map_err(anyhow::Error::new)
    .with_context(|| format!("re-authoring acceptance check '{}'", frozen.id))?;
    // An incomplete reply is not an entry: it is fed back like one that does
    // not parse, and costs the attempt.
    if judge::require_complete_judge_response(&outcome).is_err() || outcome.content.is_empty() {
        return Ok(String::new());
    }
    Ok(outcome.content)
}

#[cfg(test)]
#[path = "workflow_acceptance_reauthor_test_client.rs"]
pub(crate) mod test_client;
#[cfg(test)]
#[path = "workflow_acceptance_reauthor_tests.rs"]
mod tests;

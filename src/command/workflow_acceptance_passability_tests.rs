//! Issue 275 through the staged freeze: a check's own failure on the tree
//! before any implementation is judged for whether the check can pass.

use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};

use archon_workflow::error::WorkflowResult;
use archon_workflow::llm_client_port::WorkflowAgentOutcome;
use async_trait::async_trait;
use serde_json::Value;

use super::findings::tests::outside_set;
use super::*;
use crate::command::workflow_freeze_budget::{FreezeBudget, FreezeResume};

/// Fails before any implementation because the product refuses the
/// check's own fixture: no implementation can make it pass.
pub(super) const UNPASSABLE: &str = r#"echo "gate refused dataset fixture-1: observed rows 40 below required minimum 400" >&2; exit 1"#;
/// Fails before any implementation because the output it reads is absent.
pub(super) const ABSENT: &str =
    "python3 -c 'import json; assert json.load(open(\"out.json\"))[\"valid\"]'";
/// What the scripted judge reads as a refusal of the check's own setup.
const RULE: &str = "below required minimum";

/// Accepts every check, except one whose baseline output it is shown states
/// [`RULE`]: that one cannot pass as written. Counts both kinds of batch.
#[derive(Default)]
pub(super) struct EvidenceJudge {
    /// Batches whose checks carry no baseline output.
    pub(super) plain: AtomicUsize,
    /// Batches whose checks carry their baseline output.
    pub(super) evidence: AtomicUsize,
    pub(super) prompts: Mutex<Vec<String>>,
    /// Every re-author prompt.
    pub(super) authored: Mutex<Vec<String>>,
    /// The commands the re-author writes, in turn; then [`ABSENT`].
    pub(super) replies: Mutex<std::collections::VecDeque<String>>,
    /// Appended to every reason the judge gives.
    pub(super) quote: Mutex<String>,
}

impl EvidenceJudge {
    pub(super) fn evidence_prompts(&self) -> Vec<String> {
        let prompts = self.prompts.lock().unwrap();
        (prompts.iter())
            .filter(|prompt| prompt.contains("\"baseline\""))
            .cloned()
            .collect()
    }
}

#[async_trait]
impl WorkflowLlmClient for EvidenceJudge {
    fn provider_id(&self) -> Option<String> {
        Some("scripted".into())
    }

    /// The re-author: shown its findings, it corrects the check's setup so
    /// that only the missing feature can make it fail.
    async fn run_agent(
        &self,
        call: archon_workflow::llm_client_port::WorkflowAgentCall,
    ) -> WorkflowResult<WorkflowAgentOutcome> {
        self.authored.lock().unwrap().push(call.task.clone());
        let entry: Value = (call.task.lines())
            .find_map(|line| line.strip_prefix("The entry being replaced: "))
            .map(|line| serde_json::from_str(line).expect("the replaced entry is JSON"))
            .expect("the author prompt names the entry it replaces");
        Ok(WorkflowAgentOutcome {
            content: super::reauthor::test_client::command_entry(
                &entry,
                &(self.replies.lock().unwrap().pop_front()).unwrap_or_else(|| ABSENT.into()),
            ),
            stop_reason: Some("end_turn".into()),
            ..WorkflowAgentOutcome::default()
        })
    }

    async fn send_message_with_temperature(
        &self,
        messages: Vec<Value>,
        _system: Vec<Value>,
        _tools: Vec<Value>,
        _model: &str,
        temperature: f64,
    ) -> WorkflowResult<WorkflowAgentOutcome> {
        assert_eq!(temperature, 0.0);
        let prompt = messages[0]["content"].as_str().expect("prompt").to_string();
        let checks: Vec<Value> =
            serde_json::from_str(prompt.split_once("Checks: ").expect("checks").1)
                .expect("checks are JSON");
        let shown = checks.iter().any(|check| check.get("baseline").is_some());
        let counter = if shown { &self.evidence } else { &self.plain };
        counter.fetch_add(1, SeqCst);
        self.prompts.lock().unwrap().push(prompt);
        let quote = self.quote.lock().unwrap().clone();
        let decisions = (checks.iter())
            .map(|check| {
                let stderr = check["baseline"]["stderr"].as_str().unwrap_or_default();
                let refused = stderr.contains(RULE);
                serde_json::json!({
                    "id": check["id"],
                    "verdict": if refused { "refuted" } else { "accepted" },
                    "counterexample": if refused { "observed rows below the required minimum" } else { "none is constructible" },
                    "reason": format!("{}{quote}", if refused { "its own fixture is below the minimum the product enforces; supply enough rows" } else { "the failure is the absent feature" }),
                })
            })
            .collect::<Vec<_>>();
        Ok(WorkflowAgentOutcome {
            content: serde_json::json!({ "decisions": decisions }).to_string(),
            stop_reason: Some("end_turn".into()),
            ..WorkflowAgentOutcome::default()
        })
    }

    async fn send_message(
        &self,
        messages: Vec<Value>,
        system: Vec<Value>,
        tools: Vec<Value>,
        model: &str,
    ) -> WorkflowResult<WorkflowAgentOutcome> {
        self.send_message_with_temperature(messages, system, tools, model, 0.0)
            .await
    }
}

pub(super) async fn freeze(
    project: &Path,
    tasks: &Path,
    prd: &Path,
    client: &Arc<EvidenceJudge>,
    resume: &FreezeResume,
) -> Result<PreparedAcceptanceFreeze> {
    let candidate = std::fs::read(tasks.join(ACCEPTANCE_CONTRACT_FILE)).unwrap();
    prepare_acceptance_freeze_resumable(
        project,
        tasks,
        prd,
        GateMode::Observe,
        candidate,
        client.clone(),
        resume,
    )
    .await
}

pub(super) fn saving() -> FreezeResume {
    FreezeResume::saving(FreezeBudget::unlimited(), true)
}

/// The one finding for `id` that states it cannot pass as written.
pub(super) fn cannot_pass<'a>(
    prepared: &'a PreparedAcceptanceFreeze,
    id: &str,
) -> Vec<&'a GateFinding> {
    (prepared.findings.iter())
        .filter(|finding| finding.subject == id)
        .filter(|finding| {
            finding
                .text
                .starts_with(&format!("check '{id}': cannot pass"))
        })
        .collect()
}

#[tokio::test]
async fn a_check_refused_for_its_own_fixture_goes_back_with_its_baseline_output() {
    let (project, _outside, tasks, prd) = outside_set(UNPASSABLE);
    let client = Arc::new(EvidenceJudge::default());
    let prepared = freeze(project.path(), &tasks, &prd, &client, &FreezeResume::none())
        .await
        .expect("the freeze completes");
    assert!(
        prepared.non_accepted_ids().contains("AC-X-001"),
        "a check that cannot pass is never published: {:?}",
        prepared
            .findings
            .iter()
            .map(|f| &f.text)
            .collect::<Vec<_>>()
    );
    let found = cannot_pass(&prepared, "AC-X-001");
    assert_eq!(found.len(), 1, "{:?}", prepared.findings);
    let text = &found[0].text;
    assert!(
        text.contains("observed rows 40 below required minimum 400"),
        "{text}"
    );
    assert!(text.contains("exit 1"), "{text}");
    assert!(text.contains("supply enough rows"), "{text}");
    assert_eq!(
        found[0].remediation_scope,
        archon_workflow::RemediationScope::CandidateArtifact
    );
    assert!(
        !(prepared.findings.iter()).any(|f| f.text.starts_with("check 'AC-X-001' was refuted")),
        "one finding per check, the one with its output: {:?}",
        prepared.findings
    );
    assert_eq!(client.evidence.load(SeqCst), 1);
}

#[tokio::test]
async fn a_check_failing_because_its_feature_is_absent_is_kept() {
    let (project, _outside, tasks, prd) = outside_set(ABSENT);
    let client = Arc::new(EvidenceJudge::default());
    let prepared = freeze(project.path(), &tasks, &prd, &client, &FreezeResume::none())
        .await
        .expect("the freeze completes");
    assert!(prepared.findings.is_empty(), "{:?}", prepared.findings);
    assert!(prepared.non_accepted_ids().is_empty());
    let shown = client.evidence_prompts();
    assert_eq!(shown.len(), 1, "the judge saw the baseline output once");
    assert!(
        shown[0].contains("No such file or directory"),
        "{}",
        shown[0]
    );
}

/// Issue 255 kept: a freeze stopped resumable before the evidence pass
/// resumes from every saved verdict, and an evidence-bearing verdict, once
/// saved, answers every later freeze of the same check and output.
#[tokio::test]
async fn a_retry_after_an_incomplete_freeze_reuses_the_saved_verdicts() {
    let (project, _outside, tasks, prd) = outside_set(UNPASSABLE);
    // 300 s usable: the probe runs, the evidence pass does not fit.
    let short = FreezeResume::saving(
        FreezeBudget::within(900, Arc::new(std::time::Instant::now)),
        true,
    );
    let first = Arc::new(EvidenceJudge::default());
    let initial = freeze(project.path(), &tasks, &prd, &first, &short)
        .await
        .expect("active evidence judging is never deferred by a total");
    assert_eq!(first.plain.load(SeqCst), 1);
    assert_eq!(first.evidence.load(SeqCst), 1);
    assert_eq!(cannot_pass(&initial, "AC-X-001").len(), 1);

    let second = Arc::new(EvidenceJudge::default());
    let resumed = freeze(project.path(), &tasks, &prd, &second, &saving())
        .await
        .expect("saved verdicts are reused");
    assert_eq!(second.plain.load(SeqCst), 0);
    assert_eq!(second.evidence.load(SeqCst), 0);

    let third = Arc::new(EvidenceJudge::default());
    let again = freeze(project.path(), &tasks, &prd, &third, &saving())
        .await
        .expect("the retry completes");
    assert_eq!(third.plain.load(SeqCst), 0);
    assert_eq!(
        third.evidence.load(SeqCst),
        0,
        "its saved verdict is reused"
    );
    assert_eq!(
        cannot_pass(&again, "AC-X-001")[0].text,
        cannot_pass(&resumed, "AC-X-001")[0].text
    );
}

/// A verdict the judge gave without the baseline output (the saved batch of
/// a freeze before Issue 275) never answers whether the check can pass, and
/// a verdict given on other output never answers for this output.
#[tokio::test]
async fn a_verdict_made_without_this_baseline_output_is_never_reused() {
    let (project, _outside, tasks, prd) = outside_set(r#"cat note.txt >&2; exit 1"#);
    std::fs::write(
        project.path().join("note.txt"),
        "observed rows 40 below required minimum 400\n",
    )
    .unwrap();
    let first = Arc::new(EvidenceJudge::default());
    freeze(project.path(), &tasks, &prd, &first, &saving())
        .await
        .expect("the freeze completes");
    // Only the evidence-less batch is left, as a freeze before the fix left it.
    let _ = std::fs::remove_dir_all(project.path().join(".archon/freeze-cache/passability"));
    let second = Arc::new(EvidenceJudge::default());
    let prepared = freeze(project.path(), &tasks, &prd, &second, &saving())
        .await
        .expect("the freeze completes");
    assert_eq!(second.plain.load(SeqCst), 0, "the batch verdict is reused");
    assert_eq!(
        second.evidence.load(SeqCst),
        1,
        "the evidence pass is asked"
    );
    assert_eq!(cannot_pass(&prepared, "AC-X-001").len(), 1);

    // The same check with other output on the baseline is judged afresh.
    std::fs::write(
        project.path().join("note.txt"),
        "observed rows 40 below required minimum 500\n",
    )
    .unwrap();
    let third = Arc::new(EvidenceJudge::default());
    let changed = freeze(project.path(), &tasks, &prd, &third, &saving())
        .await
        .expect("the freeze completes");
    assert_eq!(third.evidence.load(SeqCst), 1, "new output, new verdict");
    assert!(
        cannot_pass(&changed, "AC-X-001")[0]
            .text
            .contains("minimum 500")
    );
}

#[tokio::test]
async fn a_huge_baseline_output_is_bounded_with_a_marker() {
    let (project, _outside, tasks, prd) = outside_set(
        r#"i=0; while [ "$i" -lt 3000 ]; do echo "progress line $i of the fixture build" >&2; i=$((i+1)); done; echo "gate refused: observed rows 40 below required minimum 400" >&2; exit 1"#,
    );
    let client = Arc::new(EvidenceJudge::default());
    let prepared = freeze(project.path(), &tasks, &prd, &client, &FreezeResume::none())
        .await
        .expect("the freeze completes");
    let found = cannot_pass(&prepared, "AC-X-001");
    assert_eq!(found.len(), 1, "{:?}", prepared.findings);
    let text = &found[0].text;
    assert!(text.contains("minimum 400"), "the failing line is kept");
    assert!(text.contains("[output: "), "truncation is marked: {text}");
    assert!(text.len() < 6_000, "bounded: {} bytes", text.len());
    let shown = client.evidence_prompts();
    let plain = (client.prompts.lock().unwrap().iter())
        .find(|prompt| !prompt.contains("\"baseline\""))
        .map_or(0, String::len);
    assert!(
        shown[0].len() < plain + 6_000,
        "the judge prompt is bounded too: {} vs {plain}",
        shown[0].len()
    );
}

/// The verdict-preservation rule (a repair must not newly pass where its
/// original failed its own assertion: `newly_passing_findings`) never holds
/// a correction to a check refuted as unable to pass: the staged freeze's
/// probe holds no original, and the re-author holds only accepted checks
/// it was not seeded with. The corrected check, failing on the baseline
/// for the missing feature, is accepted.
#[tokio::test]
async fn a_correction_of_a_check_that_cannot_pass_is_accepted() {
    let (project, _outside, tasks, prd) = outside_set(UNPASSABLE);
    let client = Arc::new(EvidenceJudge::default());
    let refused = freeze(project.path(), &tasks, &prd, &client, &saving())
        .await
        .expect("the freeze completes");
    assert_eq!(cannot_pass(&refused, "AC-X-001").len(), 1);
    // The author's next candidate: same id, its own setup corrected.
    let path = tasks.join(ACCEPTANCE_CONTRACT_FILE);
    let mut draft: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    draft["acceptance"][0]["check"]["command"] = ABSENT.into();
    std::fs::write(&path, serde_json::to_vec(&draft).unwrap()).unwrap();
    let corrected = freeze(project.path(), &tasks, &prd, &client, &saving())
        .await
        .expect("the freeze completes");
    assert!(corrected.findings.is_empty(), "{:?}", corrected.findings);
    assert!(corrected.non_accepted_ids().is_empty());
    let shown = client.evidence_prompts();
    assert!(
        shown.last().unwrap().contains("No such file or directory"),
        "the correction's own baseline failure was judged"
    );
}

/// Through the re-authoring freeze: the author is first shown the finding
/// with the baseline output, and its correction is accepted.
#[tokio::test]
async fn the_reauthoring_freeze_shows_the_output_and_accepts_the_correction() {
    let (project, _outside, tasks, prd) = outside_set(UNPASSABLE);
    let client = Arc::new(EvidenceJudge::default());
    let scope = reauthor::AuthorScope::for_task_set(project.path(), &tasks, &prd);
    let prepared = prepare_acceptance_freeze_reauthoring(
        project.path(),
        &tasks,
        &prd,
        GateMode::Enforce,
        client.clone(),
        &scope,
    )
    .await
    .expect("the corrected check is accepted");
    let authored = client.authored.lock().unwrap().clone();
    assert_eq!(authored.len(), 1);
    assert!(
        authored[0].contains("cannot pass as written")
            && authored[0].contains("observed rows 40 below required minimum 400"),
        "{}",
        authored[0]
    );
    assert!(prepared.findings.is_empty(), "{:?}", prepared.findings);
    let contract = prepared.contract().unwrap();
    assert!(matches!(
        &contract.acceptance[0].check,
        archon_workflow::task_set_contract::AcceptanceCheck::Command { command, .. } if command == ABSENT
    ));
}

#[path = "workflow_acceptance_passability_tests_b.rs"]
mod b;

//! Issue-44: the body gate audits the candidate for the obligations it
//! claims, while its author can still repair it. A false verdict is a
//! `Body` finding on that task; a sibling's obligations are never asked; a
//! critic that cannot answer is an error, never a pass.

use super::*;

pub(super) const HOLLOW: &str =
    "Focused tests read a fixture registry; the shared store is not written.";
const STALE: &str = "An earlier attempt at this body, already replaced.";

/// The base corpus (TASK-WS-001 on disk claiming both obligations) with the
/// PRD's claims split: TASK-WS-001 keeps G-WS-001; the candidate TASK-WS-002
/// claims AC-WS-001 alone and is not on disk. Returns the cwd.
fn split_corpus() -> tempfile::TempDir {
    let temp = corpus();
    let landed = temp.path().join("tasks/PRD-WS-001/TASK-WS-001.md");
    let body = std::fs::read_to_string(&landed).unwrap().replace(
        "implements: [\"G-WS-001\", \"AC-WS-001\"]",
        "implements: [\"G-WS-001\"]",
    );
    std::fs::write(&landed, body).unwrap();
    temp
}

fn candidate_path(cwd: &Path) -> PathBuf {
    cwd.join("tasks/PRD-WS-001/TASK-WS-002.md")
}

pub(super) fn candidate_body(scope: &str) -> String {
    format!(
        "# TASK-WS-002 — Register\n\n```yaml\ntask_id: TASK-WS-002\ntitle: Register\ncomplexity: medium\nstatus: pending\ndepends_on: []\nblocks: []\nimplements: [\"AC-WS-001\"]\nrequired_env_keys: []\nrequired_tools: [cargo]\ndeliverable_contracts: []\n```\n\n## Scope\n\n{scope}\n\n## Focused Tests\n\n- `cargo test -p registry`\n"
    )
}

pub(super) fn body_reply(necessarily_true: bool) -> String {
    let verdict = if necessarily_true {
        serde_json::json!({"obligation_id": "AC-WS-001", "necessarily_true": true, "weakest_task_id": "", "reason": "the task must write the entry", "quoted_task_text": ""})
    } else {
        serde_json::json!({"obligation_id": "AC-WS-001", "necessarily_true": false, "weakest_task_id": "TASK-WS-002", "reason": "a fixture registry leaves the shared store empty", "quoted_task_text": HOLLOW})
    };
    serde_json::json!({ "verdicts": [verdict] }).to_string()
}

pub(super) async fn audit_candidate(
    cwd: &Path,
    body: &str,
    client: Result<Arc<dyn WorkflowLlmClient>>,
) -> Result<(String, Vec<GateFinding>)> {
    audit_task_file_candidate(cwd, &candidate_path(cwd), body, client, &[]).await
}

#[tokio::test]
async fn a_false_verdict_on_the_candidate_is_a_body_finding_naming_obligation_and_task() {
    let temp = split_corpus();
    let cwd = temp.path();
    let critic = FakeCritic::new(vec![Ok(body_reply(false))]);
    let (report, findings) = audit_candidate(cwd, &candidate_body(HOLLOW), Ok(critic.clone()))
        .await
        .expect("a false verdict is a finding, not an error");
    assert_eq!(findings.len(), 1, "{report}");
    let finding = &findings[0];
    assert_eq!(
        finding.text,
        format!(
            "obligation AC-WS-001 is claimed by TASK-WS-002 but none is obliged to make it true — a fixture registry leaves the shared store empty — task TASK-WS-002: \"{HOLLOW}\""
        ),
        "the body author reads the same words the set gate would print"
    );
    assert_eq!(
        finding.remediation_scope,
        archon_workflow::RemediationScope::Body,
        "routeFindings must put it into the body author's retry"
    );
    assert_eq!(
        finding.subject, "TASK-WS-002",
        "the claiming task is the subject"
    );
    assert_eq!(finding.gate_id, GateId::WorkflowLintTaskFile);
    assert!(
        finding
            .source_path
            .as_ref()
            .unwrap()
            .ends_with("TASK-WS-002.md")
    );
    assert!(report.contains("BLOCKING obligation AC-WS-001"), "{report}");
    assert_eq!(critic.calls(), 1);
    assert!(
        !candidate_path(cwd).exists(),
        "the audit read the candidate from stdin, never from a file it does not publish"
    );
}

#[tokio::test]
async fn a_true_verdict_on_the_candidate_adds_no_finding() {
    let temp = split_corpus();
    let cwd = temp.path();
    let critic = FakeCritic::new(vec![Ok(body_reply(true))]);
    let (report, findings) = audit_candidate(cwd, &candidate_body(HOLLOW), Ok(critic.clone()))
        .await
        .expect("audit");
    assert!(findings.is_empty(), "{report}");
    assert!(
        report.contains("AC-WS-001: necessarily true given TASK-WS-002"),
        "{report}"
    );
    assert_eq!(critic.calls(), 1);
}

/// Only the candidate's own claims are asked, and the candidate's text is
/// what is asked about — not a stale file at its path, and not the sibling's
/// obligation.
#[tokio::test]
async fn the_body_audit_asks_only_the_candidates_claims_over_the_candidates_text() {
    let temp = split_corpus();
    let cwd = temp.path();
    std::fs::write(candidate_path(cwd), candidate_body(STALE)).unwrap();
    let critic = FakeCritic::new(vec![Ok(body_reply(true))]);
    let (report, findings) = audit_candidate(cwd, &candidate_body(HOLLOW), Ok(critic.clone()))
        .await
        .expect("audit");
    assert!(findings.is_empty(), "{report}");
    assert_eq!(critic.calls(), 1);
    let prompt = critic.prompts.lock().unwrap()[0].clone();
    assert!(prompt.contains("\"id\":\"AC-WS-001\""));
    assert!(
        !prompt.contains("\"id\":\"G-WS-001\""),
        "the sibling's obligation is not this body's to answer for: {prompt}"
    );
    assert!(
        prompt.contains("===== BEGIN TASK TASK-WS-002 =====") && prompt.contains(HOLLOW),
        "the candidate's own text travels with the question"
    );
    assert!(
        !prompt.contains(STALE),
        "a stale file at the candidate's path is replaced, not read"
    );
    assert!(
        !prompt.contains("===== BEGIN TASK TASK-WS-001 ====="),
        "a sibling the candidate never names is not pulled into its cluster"
    );
    assert!(
        report.contains("1 claimed obligation(s) in 1 cluster(s)"),
        "{report}"
    );
}

/// The cluster is the set gate's cluster: a co-claimant on disk is read
/// alongside the candidate, so the verdict the body gate caches is the one
/// the set gate will look up.
#[tokio::test]
async fn a_co_claimant_on_disk_joins_the_candidates_cluster_and_the_set_gate_reuses_it() {
    let temp = corpus();
    let cwd = temp.path();
    let critic = FakeCritic::new(vec![Ok(body_reply(true))]);
    let (report, findings) = audit_candidate(cwd, &candidate_body(HOLLOW), Ok(critic.clone()))
        .await
        .expect("audit");
    assert!(findings.is_empty(), "{report}");
    let prompt = critic.prompts.lock().unwrap()[0].clone();
    assert!(
        prompt.contains("===== BEGIN TASK TASK-WS-001 =====")
            && prompt.contains("===== BEGIN TASK TASK-WS-002 ====="),
        "both claimants of AC-WS-001 are read together"
    );
    assert!(!prompt.contains("\"id\":\"G-WS-001\""));
    assert!(
        report.contains("AC-WS-001: necessarily true given TASK-WS-001, TASK-WS-002"),
        "{report}"
    );
    // Land the body, then run the set gate: the shared cluster is cached and
    // only G-WS-001 (claimed by TASK-WS-001 alone) is a new question.
    std::fs::write(candidate_path(cwd), candidate_body(HOLLOW)).unwrap();
    let only_g = serde_json::json!({ "verdicts": [{"obligation_id": "G-WS-001", "necessarily_true": true, "reason": "obliged"}] });
    let set_critic = FakeCritic::new(vec![Ok(only_g.to_string())]);
    let evaluation = evaluate(cwd, Ok(set_critic.clone())).await;
    assert!(
        evaluation.operational_error().is_none(),
        "{:?}",
        evaluation.operational_error()
    );
    assert_eq!(set_critic.calls(), 1);
    assert!(
        evaluation.report.contains("1 asked of") && evaluation.report.contains("1 served from"),
        "{}",
        evaluation.report
    );
    assert!(
        !set_critic.prompts.lock().unwrap()[0].contains("\"id\":\"AC-WS-001\""),
        "the body gate's verdict is served from cache"
    );
}

#[tokio::test]
async fn an_unanswering_critic_is_an_error_never_a_clean_body() {
    let temp = split_corpus();
    let cwd = temp.path();
    let error = audit_candidate(
        cwd,
        &candidate_body(HOLLOW),
        Err(anyhow!("connection refused")),
    )
    .await
    .expect_err("no client, no verdict");
    assert!(
        format!("{error:#}").contains("connection refused")
            && format!("{error:#}").contains("critic client"),
        "{error:#}"
    );
    let critic = FakeCritic::new(vec![Err("provider 503".into())]);
    let error = audit_candidate(cwd, &candidate_body(HOLLOW), Ok(critic))
        .await
        .expect_err("a failed call is not a pass");
    assert!(format!("{error:#}").contains("provider 503"), "{error:#}");
}

#[test]
fn the_decomposition_body_gate_carries_the_provider_and_the_freeze_clock() {
    let catalog = crate::command::workflow_host_command_catalog::fixed_decomposition_catalog("rev")
        .expect("catalog");
    let gate = &catalog.capabilities["land-task-body"];
    assert_eq!(
        gate.environment_profile,
        archon_workflow::EnvironmentProfileId::FreezeProvider,
        "the body gate now asks the critic, so it needs the provider"
    );
    assert!(
        gate.remediation_scopes
            .contains(&archon_workflow::RemediationScope::Body)
    );
    assert_eq!(
        gate.timeout_secs, catalog.capabilities["task-set-lint"].timeout_secs,
        "the same audit runs on the same clock"
    );
}

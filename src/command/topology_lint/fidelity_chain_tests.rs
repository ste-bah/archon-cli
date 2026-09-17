//! Issue-43: the critic reads the ownership chain, one hop. A claimant that
//! names a sibling task of the same set as owning the result puts that
//! sibling's text in front of the critic; a sibling that does not claim the
//! obligation is noted, not blocked.

use super::*;

const DEFERRAL: &str =
    "The store entry itself lands in TASK-WS-002; TASK-ZZ-009 is a different set and out of scope.";
const SIBLING: &str = "Writes the registry entry into the shared store.";

/// The base corpus plus a sibling task that claims nothing, named by the
/// claimant's text alongside an id that belongs to no task in the set.
fn chain_corpus() -> tempfile::TempDir {
    let temp = corpus();
    let tasks = temp.path().join("tasks").join("PRD-WS-001");
    let claimant = tasks.join("TASK-WS-001.md");
    let body = std::fs::read_to_string(&claimant)
        .unwrap()
        .replace(LOOPHOLE, &format!("{LOOPHOLE} {DEFERRAL}"));
    std::fs::write(&claimant, body).unwrap();
    std::fs::write(
        tasks.join("TASK-WS-002.md"),
        format!(
            "# TASK-WS-002 — Store\n\n```yaml\ntask_id: TASK-WS-002\ntitle: Store\ncomplexity: medium\nstatus: pending\ndepends_on: []\nblocks: []\nimplements: []\nrequired_env_keys: []\nrequired_tools: [cargo]\ndeliverable_contracts: []\n```\n\n## Scope\n\n{SIBLING}\n\n## Focused Tests\n\n- `cargo test -p store`\n"
        ),
    )
    .unwrap();
    temp
}

fn claimant_only_digest(cwd: &Path) -> String {
    let obligations = vec![
        ClaimedObligation {
            id: "AC-WS-001".into(),
            text: "Ingestion stores a registry entry.".into(),
        },
        ClaimedObligation {
            id: "G-WS-001".into(),
            text: "Ingest widgets into the shared store.".into(),
        },
    ];
    let text = std::fs::read_to_string(cwd.join("tasks/PRD-WS-001/TASK-WS-001.md")).unwrap();
    fidelity_cluster_digest(
        &obligations,
        &[ClaimingTask {
            task_id: "TASK-WS-001".into(),
            text,
        }],
    )
}

#[tokio::test]
async fn a_named_sibling_joins_the_cluster_and_changes_its_digest() {
    let temp = chain_corpus();
    let cwd = temp.path();
    let critic = FakeCritic::new(vec![Ok(reply(&[]))]);
    let evaluation = evaluate(cwd, Ok(critic.clone())).await;
    assert!(
        evaluation.operational_error().is_none(),
        "{:?}",
        evaluation.operational_error()
    );
    assert_eq!(critic.calls(), 1, "both obligations share one cluster");
    let prompt = critic.prompts.lock().unwrap()[0].clone();
    assert!(prompt.contains("===== BEGIN TASK TASK-WS-001 ====="));
    assert!(
        prompt.contains("===== BEGIN TASK TASK-WS-002 =====") && prompt.contains(SIBLING),
        "the named sibling's full text travels with the question"
    );
    assert!(
        !prompt.contains("BEGIN TASK TASK-ZZ-009"),
        "an id outside the set is never pulled in"
    );
    assert!(prompt.contains("or that a claiming task names as owning part of the result"));
    assert!(
        evaluation
            .report
            .contains("AC-WS-001: necessarily true given TASK-WS-001, TASK-WS-002"),
        "{}",
        evaluation.report
    );
    let cache = cwd.join(".archon/lint-cache/fidelity");
    let cached: Vec<String> = std::fs::read_dir(&cache)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(cached.len(), 1, "{cached:?}");
    assert_ne!(
        cached[0],
        format!("{}.json", claimant_only_digest(cwd)),
        "the digest covers the sibling, so the cache key changes with the chain"
    );
}

#[tokio::test]
async fn a_named_sibling_that_does_not_claim_the_obligation_is_noted_not_blocked() {
    let temp = chain_corpus();
    let cwd = temp.path();
    let evaluation = evaluate(cwd, Ok(FakeCritic::new(vec![Ok(reply(&[]))]))).await;
    for id in ["AC-WS-001", "G-WS-001"] {
        let note = format!(
            "  NOTE obligation {id} is claimed by TASK-WS-001, whose text names TASK-WS-002 as owning the result, but TASK-WS-002 does not claim {id}\n"
        );
        assert!(evaluation.report.contains(&note), "{}", evaluation.report);
    }
    assert!(
        !evaluation.report.contains("TASK-ZZ-009 as owning"),
        "{}",
        evaluation.report
    );
    assert!(
        !evaluation
            .findings
            .iter()
            .any(|finding| finding.text.starts_with("obligation ")),
        "a note is not a finding"
    );
}

/// A false verdict may name the sibling as the weakest task; the finding
/// still says who claimed the obligation and points at the sibling's file.
#[tokio::test]
async fn a_false_verdict_on_the_sibling_names_the_claimant_and_points_at_the_sibling() {
    let temp = chain_corpus();
    let cwd = temp.path();
    let verdicts = serde_json::json!({"verdicts": [
        {"obligation_id": "AC-WS-001", "necessarily_true": false, "weakest_task_id": "TASK-WS-002", "reason": "the sibling writes nothing the PRD names", "quoted_task_text": SIBLING},
        {"obligation_id": "G-WS-001", "necessarily_true": true, "reason": "obliged"}
    ]});
    let evaluation = evaluate(cwd, Ok(FakeCritic::new(vec![Ok(verdicts.to_string())]))).await;
    assert!(evaluation.operational_error().is_none());
    let finding = evaluation
        .findings
        .iter()
        .find(|finding| finding.text.starts_with("obligation AC-WS-001"))
        .expect("blocking finding");
    assert_eq!(
        finding.text,
        format!(
            "obligation AC-WS-001 is claimed by TASK-WS-001 but none is obliged to make it true — the sibling writes nothing the PRD names — task TASK-WS-002: \"{SIBLING}\""
        )
    );
    assert!(
        finding
            .source_path
            .as_ref()
            .unwrap()
            .ends_with("TASK-WS-002.md")
    );
}

#[test]
fn a_task_id_is_named_only_whole() {
    assert!(mentions_whole("lands in TASK-WS-002.", "TASK-WS-002"));
    assert!(mentions_whole("(TASK-WS-002)", "TASK-WS-002"));
    assert!(mentions_whole("TASK-WS-002", "TASK-WS-002"));
    assert!(!mentions_whole("lands in TASK-WS-0021.", "TASK-WS-002"));
    assert!(!mentions_whole("XTASK-WS-002", "TASK-WS-002"));
    assert!(!mentions_whole(
        "TASK-WS-0021 and TASK-WS-0022",
        "TASK-WS-002"
    ));
}

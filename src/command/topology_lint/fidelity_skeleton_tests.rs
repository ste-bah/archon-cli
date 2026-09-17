//! Issue-45: the critic reads the frozen skeleton. Every task in the set is
//! listed with its frozen edges, a sibling whose body is not yet written is
//! still a listed task in the per-body audit, a re-frozen skeleton changes
//! every cluster digest, and a set with no skeleton is audited and told so.

use super::body::{HOLLOW, audit_candidate, body_reply, candidate_body};
use super::*;

/// Three frozen tasks: TASK-WS-001 blocks TASK-WS-002, which TASK-WS-002
/// depends on, and TASK-WS-003 depends on TASK-WS-002 for ordering only.
/// Only TASK-WS-001 has a body on disk (the base corpus).
fn skeleton_json(third_depends_on_second: bool) -> String {
    let third_depends_on = if third_depends_on_second {
        serde_json::json!([{"task_id": "TASK-WS-002", "ordering_only": true}])
    } else {
        serde_json::json!([])
    };
    serde_json::to_string_pretty(&serde_json::json!({
        "schema_version": 1,
        "acceptance_digest": "d",
        "tasks": [
            {"task_id": "TASK-WS-001", "file_name": "TASK-WS-001.md", "depends_on": [], "blocks": ["TASK-WS-002"], "implements": ["G-WS-001", "AC-WS-001"], "deliverable_contracts": [{"kind": "artifact", "artifact_path": "out/registry.json"}]},
            {"task_id": "TASK-WS-002", "file_name": "TASK-WS-002.md", "depends_on": [{"task_id": "TASK-WS-001", "consumes": [{"artifact_path": "out/registry.json"}]}], "blocks": [], "implements": ["AC-WS-001"], "deliverable_contracts": []},
            {"task_id": "TASK-WS-003", "file_name": "TASK-WS-003.md", "depends_on": third_depends_on, "blocks": [], "implements": [], "deliverable_contracts": []}
        ]
    }))
    .unwrap()
}

fn skeleton_path(cwd: &Path) -> PathBuf {
    cwd.join("tasks/PRD-WS-001")
        .join(archon_workflow::task_set_contract::TASK_SKELETON_FILE)
}

/// The audit alone, as the waiver test runs it: the ordinary lint's freeze
/// chain check (`task_set.rs`) is not what these tests exercise.
async fn audit_set(cwd: &Path, client: &dyn WorkflowLlmClient) -> Result<FidelityAudit> {
    audit(cwd, &cwd.join("tasks/PRD-WS-001"), client, &[]).await
}

fn frozen_corpus() -> tempfile::TempDir {
    let temp = corpus();
    std::fs::write(skeleton_path(temp.path()), skeleton_json(true)).unwrap();
    temp
}

const SECOND_LINE: &str = r#"{"task_id":"TASK-WS-002","file_name":"TASK-WS-002.md","depends_on":[{"task_id":"TASK-WS-001","consumes":["out/registry.json"],"ordering_only":false}],"blocks":[],"implements":["AC-WS-001"],"deliverable_contracts":[]}"#;
const THIRD_LINE: &str = r#"{"task_id":"TASK-WS-003","file_name":"TASK-WS-003.md","depends_on":[{"task_id":"TASK-WS-002","consumes":[],"ordering_only":true}],"blocks":[],"implements":[],"deliverable_contracts":[]}"#;

/// (a) The set gate's prompt lists every skeleton task with its id,
/// depends_on and blocks, between the obligations and the task texts.
#[tokio::test]
async fn the_set_gate_prompt_lists_every_skeleton_task_with_its_frozen_edges() {
    let temp = frozen_corpus();
    let critic = FakeCritic::new(vec![Ok(reply(&[]))]);
    let audit = audit_set(temp.path(), critic.as_ref())
        .await
        .expect("audit");
    assert!(audit.findings.is_empty(), "{}", audit.report);
    let prompt = critic.prompts.lock().unwrap()[0].clone();
    assert!(
        prompt.contains(
            r#"{"task_id":"TASK-WS-001","file_name":"TASK-WS-001.md","depends_on":[],"blocks":["TASK-WS-002"],"implements":["G-WS-001","AC-WS-001"],"deliverable_contracts":[{"kind":"artifact","artifact_path":"out/registry.json"}]}"#
        ),
        "{prompt}"
    );
    assert!(prompt.contains(SECOND_LINE), "{prompt}");
    assert!(prompt.contains(THIRD_LINE), "{prompt}");
    let obligations_at = prompt.find("Obligations: [").unwrap();
    let skeleton_at = prompt.find("FROZEN SKELETON (every task").unwrap();
    let task_at = prompt.find("===== BEGIN TASK TASK-WS-001 =====").unwrap();
    assert!(obligations_at < skeleton_at && skeleton_at < task_at);
    assert!(prompt.contains("depends_on is transitive"));
    assert!(!prompt.contains("this task set has no frozen skeleton"));
}

/// (b) The per-body audit of TASK-WS-002 has no sibling body for
/// TASK-WS-003 and does not pull TASK-WS-001 into the cluster, yet both
/// siblings' skeleton lines are in the prompt.
#[tokio::test]
async fn a_candidate_audit_lists_siblings_whose_bodies_are_not_written() {
    let temp = frozen_corpus();
    let cwd = temp.path();
    let landed = cwd.join("tasks/PRD-WS-001/TASK-WS-001.md");
    let body = std::fs::read_to_string(&landed).unwrap().replace(
        "implements: [\"G-WS-001\", \"AC-WS-001\"]",
        "implements: [\"G-WS-001\"]",
    );
    std::fs::write(&landed, body).unwrap();
    let critic = FakeCritic::new(vec![Ok(body_reply(true))]);
    let (report, findings) = audit_candidate(cwd, &candidate_body(HOLLOW), Ok(critic.clone()))
        .await
        .expect("audit");
    assert!(findings.is_empty(), "{report}");
    let prompt = critic.prompts.lock().unwrap()[0].clone();
    assert!(
        !prompt.contains("===== BEGIN TASK TASK-WS-001 =====")
            && !prompt.contains("===== BEGIN TASK TASK-WS-003 ====="),
        "neither sibling body is in the cluster: {prompt}"
    );
    assert!(
        prompt.contains(r#"{"task_id":"TASK-WS-001","#) && prompt.contains(THIRD_LINE),
        "both siblings are listed from the skeleton: {prompt}"
    );
    assert!(prompt.contains(SECOND_LINE), "{prompt}");
    assert!(prompt.contains("is not yet written; it will be audited when it is written"));
}

/// (c) Re-freezing the skeleton changes the digest: the cached verdict is
/// not served and the critic is asked again over unchanged texts.
#[tokio::test]
async fn a_changed_skeleton_invalidates_the_cached_verdict() {
    let temp = frozen_corpus();
    let cwd = temp.path();
    let critic = FakeCritic::new(vec![Ok(reply(&[]))]);
    audit_set(cwd, critic.as_ref()).await.expect("audit");
    let again = audit_set(cwd, critic.as_ref()).await.expect("audit");
    assert_eq!(critic.calls(), 1, "unchanged skeleton is served from cache");
    assert!(again.report.contains("1 served from"), "{}", again.report);
    std::fs::write(skeleton_path(cwd), skeleton_json(false)).unwrap();
    let critic = FakeCritic::new(vec![Ok(reply(&[]))]);
    let refrozen = audit_set(cwd, critic.as_ref()).await.expect("audit");
    assert_eq!(critic.calls(), 1, "a re-frozen skeleton re-asks");
    assert!(
        refrozen.report.contains("1 asked of"),
        "{}",
        refrozen.report
    );
    let prompt = critic.prompts.lock().unwrap()[0].clone();
    assert!(!prompt.contains(THIRD_LINE));
    assert!(
        prompt.contains(r#"{"task_id":"TASK-WS-003","file_name":"TASK-WS-003.md","depends_on":[]"#)
    );
    let cache = cwd.join(".archon/lint-cache/fidelity");
    assert_eq!(
        std::fs::read_dir(&cache).unwrap().count(),
        2,
        "two digests, one per skeleton"
    );
}

/// (d) A set with no frozen skeleton is still audited, its prompt says so,
/// and its digest differs from the same texts with a skeleton.
#[tokio::test]
async fn a_set_without_a_skeleton_is_audited_and_the_prompt_says_so() {
    let temp = corpus();
    let cwd = temp.path();
    assert!(!skeleton_path(cwd).exists());
    let critic = FakeCritic::new(vec![Ok(reply(&[]))]);
    let evaluation = evaluate(cwd, Ok(critic.clone())).await;
    assert!(
        evaluation.operational_error().is_none(),
        "{:?}",
        evaluation.operational_error()
    );
    assert!(
        evaluation
            .report
            .contains("AC-WS-001: necessarily true given TASK-WS-001")
    );
    let prompt = critic.prompts.lock().unwrap()[0].clone();
    assert!(
        prompt.contains("FROZEN SKELETON: this task set has no frozen skeleton; only the task texts below establish inter-task ordering and ownership."),
        "{prompt}"
    );
    assert!(!prompt.contains(r#"{"task_id":"#));
    let cache = cwd.join(".archon/lint-cache/fidelity");
    let without: Vec<String> = std::fs::read_dir(&cache)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    std::fs::write(skeleton_path(cwd), skeleton_json(true)).unwrap();
    let critic = FakeCritic::new(vec![Ok(reply(&[]))]);
    audit_set(cwd, critic.as_ref()).await.expect("audit");
    assert_eq!(critic.calls(), 1, "freezing a skeleton re-asks");
    assert_eq!(
        std::fs::read_dir(&cache).unwrap().count(),
        without.len() + 1
    );
}

/// A skeleton file that exists but cannot be read is operational, never a
/// silent fall-back to the no-skeleton wording.
#[tokio::test]
async fn an_unreadable_skeleton_is_operational_never_a_pass() {
    let temp = corpus();
    let cwd = temp.path();
    std::fs::write(skeleton_path(cwd), "{not json").unwrap();
    let critic = FakeCritic::new(vec![]);
    let error = match audit_set(cwd, critic.as_ref()).await {
        Ok(audit) => panic!("an unreadable skeleton is an error, not {}", audit.report),
        Err(error) => format!("{error:#}"),
    };
    assert!(error.contains("parsing skeleton"), "{error}");
    assert_eq!(critic.calls(), 0);
}

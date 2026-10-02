//! Production write-wave seam with scripted agent replies and real Git writes:
//! accepted, malformed, empty and tolerated replies.
use archon_workflow::v2::call_data::v2_agent_request;
use archon_workflow::*;
use serde_json::json;
use std::path::Path;

#[path = "support/write_wave_seam.rs"]
mod seam;
use seam::*;

#[tokio::test]
async fn accepted_write_wave_commits_real_files_before_return() {
    let f = Fixture::new();
    let (out, _) = f.wave("write-accepted", Reply::Accepted).await;
    assert_eq!(out.status, WorkflowV2Status::Accepted, "{out:#?}");
    assert_ne!(git(&f.repo, &["rev-parse", "HEAD"]), f.base);
    assert_eq!(
        git(&f.repo, &["show", "HEAD:added.txt"]),
        "retained new file"
    );
    assert_eq!(git(&f.repo, &["show", "HEAD:owned.txt"]), "implemented");
}
#[tokio::test]
async fn malformed_reply_preserves_nonempty_patch_and_next_wave_resumes() {
    preserves_and_resumes(Reply::Malformed).await;
}
#[tokio::test]
async fn timeout_after_files_preserves_nonempty_patch_and_next_wave_resumes() {
    preserves_and_resumes(Reply::Timeout).await;
}
async fn preserves_and_resumes(reply: Reply) {
    let f = Fixture::new();
    let (out, _) = f.wave("write-first", reply).await;
    assert_ne!(out.status, WorkflowV2Status::Accepted, "{out:#?}");
    assert_eq!(git(&f.repo, &["rev-parse", "HEAD"]), f.base);
    let branch =
        f.v2.load_branch_outcome("write-first", "write-first-0")
            .unwrap()
            .unwrap();
    let data = branch.result.unwrap().data;
    let patch = Path::new(
        data["partial_work"]["patch_path"]
            .as_str()
            .expect("partial patch not captured through wave"),
    );
    let bytes = std::fs::read(patch).unwrap();
    assert!(!bytes.is_empty());
    let text = String::from_utf8_lossy(&bytes);
    assert!(text.contains("implemented") && text.contains("retained new file"));
    let (out, dispatch) = f.wave("write-resumed", Reply::Accepted).await;
    assert!(
        *dispatch.resumed.lock().unwrap(),
        "next item workspace did not apply retained patch"
    );
    // Obs-8 across waves: the fresh attempt is told what the earlier branch
    // for this task was refused, found through the saved outcome.
    let prompt = dispatch.prompts.lock().unwrap()[0].clone();
    assert!(
        prompt
            .contains("refused by the host — do not retry them:\n  - Bash `cargo build --release`"),
        "{prompt}"
    );
    assert_eq!(out.status, WorkflowV2Status::Accepted, "{out:#?}");
    assert_eq!(
        git(&f.repo, &["show", "HEAD:added.txt"]),
        "retained new file"
    );
}

#[tokio::test]
async fn missing_final_closer_does_not_discard_completed_work() {
    let f = Fixture::new();
    let (out, _) = f.wave("write-closer", Reply::MissingCloser).await;
    assert_eq!(out.status, WorkflowV2Status::Accepted, "{out:#?}");
    assert_eq!(
        git(&f.repo, &["show", "HEAD:added.txt"]),
        "retained new file"
    );
}
#[tokio::test]
async fn single_quote_escape_does_not_discard_completed_work() {
    let f = Fixture::new();
    let (out, _) = f.wave("write-quote", Reply::SingleQuoteEscape).await;
    assert_eq!(out.status, WorkflowV2Status::Accepted, "{out:#?}");
    assert_eq!(git(&f.repo, &["show", "HEAD:owned.txt"]), "implemented");
}
#[tokio::test]
async fn budget_and_resumed_patch_reach_actual_rendered_prompt() {
    let f = Fixture::new();
    let (_, first) = f.wave("write-budget", Reply::Timeout).await;
    {
        let prompts = first.prompts.lock().unwrap();
        assert!(prompts[0].contains("Time budget:"));
        assert!(prompts[0].contains("Write the deliverable files first"));
    }
    let (_, next) = f.wave("write-budget-resume", Reply::Accepted).await;
    let prompts = next.prompts.lock().unwrap();
    assert!(prompts[0].contains("has been applied to this workspace"));
    assert!(prompts[0].contains("Implement the item now."));
}

#[test]
fn syntax_tolerance_does_not_invent_missing_values_or_write_evidence() {
    let call = WorkflowV2HostCall {
        id: "write-invalid".into(),
        method: WorkflowV2HostMethod::Implementation,
        write_mode: Some(WorkflowV2WriteMode::Worktree),
        options: WorkflowV2HostOptions::default(),
    };
    let request = v2_agent_request(
        "implement",
        None,
        &WorkflowV2CallExecution {
            call,
            input: json!({}),
            depends_on: vec![],
        },
        None,
    );
    let adapter = WorkflowV2AgentAdapter::new();
    for raw in [
        r#"{"status":"accepted","summary":"cut off"#,
        r#"{"status":"accepted","data":{"count":12"#,
        r#"{"status":"accepted","data":{"ready":tru"#,
        r#"{"status":"accepted","summary":"no evidence""#,
        r#"{"status":"accepted","summary":"bad\qescape"}"#,
    ] {
        assert!(
            adapter.parse_agent_output(&request, raw).is_err(),
            "invalid report was accepted: {raw}"
        );
    }
}

#[tokio::test]
async fn empty_reply_after_writes_retains_partial_and_next_wave_resumes() {
    preserves_and_resumes(Reply::Empty).await;
}

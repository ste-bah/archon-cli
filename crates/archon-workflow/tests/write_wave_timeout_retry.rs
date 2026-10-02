//! Write-wave in-run timeout retry: a branch the host cut with work on disk
//! is re-asked once, and a second cut, a host cut or a transport drop inside
//! the retry stall the way the gap says.
use archon_workflow::*;
use std::time::Duration;

#[path = "support/write_wave_seam.rs"]
mod seam;
use seam::*;

/// A branch the host cut with work on disk is re-asked once in-run, told what
/// its worktree holds, and its accepted retry lands like a first-try accept.
#[tokio::test]
async fn timed_out_branch_with_partial_work_is_retried_in_run_and_lands() {
    let f = Fixture::new();
    let (out, dispatch) = f.wave("write-retry", Reply::TimeoutOnce).await;
    assert_eq!(out.status, WorkflowV2Status::Accepted, "{out:#?}");
    assert!(
        !out.residual_gaps
            .iter()
            .any(|gap| gap.id.starts_with("write_branch_timeout_")),
        "{out:#?}"
    );
    assert_ne!(git(&f.repo, &["rev-parse", "HEAD"]), f.base);
    assert_eq!(
        git(&f.repo, &["show", "HEAD:added.txt"]),
        "retained new file"
    );
    // What lands is the retry's worktree, its own edit included.
    assert_eq!(
        git(&f.repo, &["show", "HEAD:owned.txt"]),
        "implemented by retry"
    );
    let branch =
        f.v2.load_branch_outcome("write-retry", "write-retry-0")
            .unwrap()
            .unwrap();
    let result = branch.result.unwrap();
    assert_eq!(result.status, WorkflowV2Status::Accepted, "{result:#?}");
    assert!(
        result
            .residual_gaps
            .iter()
            .all(|gap| !gap.id.starts_with("write_branch_timeout_"))
    );
    let manifest = std::fs::read_to_string(
        f.store
            .run_dir(&f.run)
            .join("write-coordination/stages/write-retry/manifests/write-retry-0.json"),
    )
    .expect("manifest persisted for the accepted retry");
    assert!(
        manifest.contains("owned.txt") && manifest.contains("added.txt"),
        "{manifest}"
    );
    // The retry is the second and last session, told it continues earlier work.
    let prompts = dispatch.prompts.lock().unwrap();
    assert_eq!(prompts.len(), 2, "one timed-out session and one retry");
    assert!(
        !prompts[0].contains("A previous attempt at this task"),
        "{}",
        prompts[0]
    );
    let retry = &prompts[1];
    assert!(
        retry.contains("A previous attempt at this task ran out of time before finishing."),
        "{retry}"
    );
    assert!(
        retry.contains("Its uncommitted work (2 file(s)) has been applied to this workspace"),
        "{retry}"
    );
    assert!(
        retry.contains("added.txt") && retry.contains("owned.txt"),
        "{retry}"
    );
    assert!(retry.contains("The declared focused tests are believed to pass; run them once and return the result envelope."), "{retry}");
    assert!(retry.contains("this call has 30 minutes"), "{retry}");
    assert!(
        retry.contains("Implement the item now.") && retry.contains("\nLanding policy ("),
        "{retry}"
    );
    // Obs-8: the retry is told what the cut session was refused; the first
    // session, with no earlier session to remember, is not.
    assert!(
        retry.contains("The previous session had these tool calls refused by the host — do not retry them:\n  - Bash `cargo build --release` → Release builds are disabled for this write-capable workflow call."),
        "{retry}"
    );
    assert!(retry.contains("Its last 1 tool call (most recent last) were:\n  - Bash `cargo build --release` → refused:"), "{retry}");
    assert!(
        !prompts[0].contains("refused by the host"),
        "{}",
        prompts[0]
    );
    assert!(
        *dispatch.resumed.lock().unwrap(),
        "retry did not see the partial work in its worktree"
    );
    assert_eq!(
        *dispatch.timeout_overrides.lock().unwrap(),
        vec![None, Some(1_800)]
    );
    let transport = std::fs::read_to_string(f.v2.root().join("transport.jsonl")).unwrap();
    let row = transport
        .lines()
        .find(|line| line.contains("\"kind\":\"write_branch_timeout_retry\""))
        .expect("retry row recorded");
    assert!(
        row.contains("\"item_id\":\"write-retry-0\"") && row.contains("\"patch_files\":2"),
        "{row}"
    );
}

/// A second timeout stalls exactly as before: the gap is emitted, the partial
/// work is kept, and `resume` picks it up.
#[tokio::test]
async fn a_second_timeout_emits_the_gap_as_before() {
    let f = Fixture::new();
    let (out, dispatch) = f.wave("write-twice", Reply::Timeout).await;
    assert_ne!(out.status, WorkflowV2Status::Accepted, "{out:#?}");
    assert_eq!(
        dispatch.prompts.lock().unwrap().len(),
        2,
        "exactly one in-run retry"
    );
    assert_eq!(git(&f.repo, &["rev-parse", "HEAD"]), f.base);
    let branch =
        f.v2.load_branch_outcome("write-twice", "write-twice-0")
            .unwrap()
            .unwrap();
    let result = branch.result.unwrap();
    assert!(
        result
            .residual_gaps
            .iter()
            .any(|gap| gap.id == "write_branch_timeout_write-twice-0"),
        "{result:#?}"
    );
    assert_eq!(result.data["branch_runtime_timeout"], true);
    assert!(
        result.data["partial_work"]["patch_path"].is_string(),
        "{result:#?}"
    );
    assert!(
        result.evidence.iter().any(|e| e
            .summary
            .contains("in-run retry with partial work applied ended")),
        "{result:#?}"
    );
}

/// Issue-10, the live shape: the host's typed cut arrives inside the retry
/// with hours of call budget left. The loop must not read it as a transport
/// drop and start a third session — the retry is one attempt, then the stall.
#[tokio::test]
async fn a_host_cut_inside_the_retry_stalls_without_a_third_session() {
    let f = Fixture::new();
    let (out, dispatch) = f
        .wave_under(
            "write-cut",
            Reply::HostCut,
            Duration::from_secs(14_400),
            Duration::from_secs(1_800),
        )
        .await;
    assert_ne!(out.status, WorkflowV2Status::Accepted, "{out:#?}");
    assert_eq!(
        dispatch.prompts.lock().unwrap().len(),
        2,
        "first session and ONE retry"
    );
    assert_eq!(
        *dispatch.timeout_overrides.lock().unwrap(),
        vec![None, Some(1_800)]
    );
    assert_eq!(git(&f.repo, &["rev-parse", "HEAD"]), f.base);
    let branch =
        f.v2.load_branch_outcome("write-cut", "write-cut-0")
            .unwrap()
            .unwrap();
    let result = branch.result.unwrap();
    assert_eq!(result.data["branch_runtime_timeout"], true, "{result:#?}");
    assert!(
        result
            .residual_gaps
            .iter()
            .any(|gap| gap.id == "write_branch_timeout_write-cut-0"),
        "{result:#?}"
    );
    assert!(
        result.evidence.iter().any(|e| e
            .summary
            .contains("in-run retry with partial work applied ended")),
        "{result:#?}"
    );
    // The partial patch reflects the retry's edits, not only the first cut's.
    let patch =
        std::fs::read_to_string(result.data["partial_work"]["patch_path"].as_str().unwrap())
            .unwrap();
    assert!(patch.contains("implemented by retry"), "{patch}");
    let rows = std::fs::read_to_string(f.v2.root().join("transport.jsonl")).unwrap();
    assert_eq!(
        rows.matches("\"kind\":\"write_branch_timeout_retry\"")
            .count(),
        1,
        "{rows}"
    );
}

/// A genuine provider drop inside the retry is still re-asked — that is what
/// the transport budget is for — but under the retry's own wall clock, not
/// the first session's.
#[tokio::test]
async fn a_transport_drop_inside_the_retry_re_asks_within_the_retry_budget() {
    let f = Fixture::new();
    let (out, dispatch) = f
        .wave_under(
            "write-drop",
            Reply::HostCutThenDrop,
            Duration::from_secs(14_400),
            Duration::from_secs(1),
        )
        .await;
    assert_ne!(out.status, WorkflowV2Status::Accepted, "{out:#?}");
    let dispatches = dispatch.prompts.lock().unwrap().len();
    // One cut session, then at least one re-ask after a drop, and never the
    // full transport budget (1 + 6) the call budget of hours would allow.
    assert!((3..=5).contains(&dispatches), "dispatches: {dispatches}");
    let overrides = dispatch.timeout_overrides.lock().unwrap();
    assert_eq!(overrides[0], None);
    assert!(
        overrides[1..].iter().all(|o| *o == Some(1)),
        "{overrides:?}"
    );
    let branch =
        f.v2.load_branch_outcome("write-drop", "write-drop-0")
            .unwrap()
            .unwrap();
    let result = branch.result.unwrap();
    assert_eq!(result.data["branch_runtime_timeout"], true, "{result:#?}");
    assert!(
        result
            .residual_gaps
            .iter()
            .any(|gap| gap.id == "write_branch_timeout_write-drop-0"),
        "{result:#?}"
    );
}

/// Issue-213 C2: a session the runner stopped for making no progress is
/// terminal for its branch. It is not re-asked in-run (exactly one session),
/// what it wrote is kept, and its `NeedsReview` result carries the gap the
/// script remediates.
#[tokio::test]
async fn a_no_progress_stop_is_not_re_asked_and_goes_to_remediation() {
    let f = Fixture::new();
    let (out, dispatch) = f.wave("write-stuck", Reply::NoProgress).await;
    assert_ne!(out.status, WorkflowV2Status::Accepted, "{out:#?}");
    assert_eq!(
        dispatch.prompts.lock().unwrap().len(),
        1,
        "a no-progress stop must not be re-asked in-run"
    );
    assert_eq!(git(&f.repo, &["rev-parse", "HEAD"]), f.base);
    let transport =
        std::fs::read_to_string(f.v2.root().join("transport.jsonl")).unwrap_or_default();
    assert!(
        !transport.contains("\"kind\":\"write_branch_timeout_retry\""),
        "{transport}"
    );
    let result =
        f.v2.load_branch_outcome("write-stuck", "write-stuck-0")
            .unwrap()
            .unwrap()
            .result
            .unwrap();
    assert_eq!(result.status, WorkflowV2Status::NeedsReview, "{result:#?}");
    assert_eq!(result.data["branch_no_progress_stop"], true);
    assert!(
        result.summary.contains("routed for remediation"),
        "{}",
        result.summary
    );
    assert!(
        result
            .residual_gaps
            .iter()
            .any(|gap| gap.id == "write_branch_timeout_write-stuck-0"),
        "{result:#?}"
    );
    assert!(
        result.data["partial_work"]["patch_path"].is_string(),
        "the stopped session's work must be kept: {result:#?}"
    );
}

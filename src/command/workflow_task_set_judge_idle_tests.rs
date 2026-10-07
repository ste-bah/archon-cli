//! Issue 356: provider activity renews the judge's window without a total.
use super::*;
use std::time::Duration;

struct Streaming {
    gap: u64,
    silent_after: bool,
}
#[async_trait]
impl WorkflowLlmClient for Streaming {
    async fn send_message(
        &self,
        _: Vec<serde_json::Value>,
        _: Vec<serde_json::Value>,
        _: Vec<serde_json::Value>,
        _: &str,
    ) -> WorkflowResult<WorkflowAgentOutcome> {
        unreachable!()
    }
    async fn send_message_with_progress(
        &self,
        _: Vec<serde_json::Value>,
        _: Vec<serde_json::Value>,
        _: Vec<serde_json::Value>,
        _: &str,
        _: f64,
        progress: archon_shell::progress::Progress,
    ) -> WorkflowResult<WorkflowAgentOutcome> {
        for _ in 0..2 {
            tokio::time::sleep(Duration::from_secs(self.gap)).await;
            progress.record();
        }
        if self.silent_after {
            std::future::pending::<()>().await;
        }
        tokio::time::sleep(Duration::from_secs(self.gap)).await;
        Ok(WorkflowAgentOutcome {
            content: format!("{HEAD}{TAIL}"),
            stop_reason: Some("end_turn".into()),
            ..Default::default()
        })
    }
}
async fn active(gap: u64) {
    let started = tokio::time::Instant::now();
    let judged = judge_contract(
        &Streaming {
            gap,
            silent_after: false,
        },
        contract(),
        &expected(),
    )
    .await
    .unwrap();
    assert_eq!(
        judged.acceptance[0].judgment.verdict,
        JudgeDecision::Accepted
    );
    assert!(started.elapsed().as_secs() > JUDGE_TIMEOUT_SECS);
}
#[tokio::test(start_paused = true)]
async fn issue356_judge_regular_stream_survives() {
    active(3_600).await;
}
#[tokio::test(start_paused = true)]
async fn issue356_judge_slow_stream_survives() {
    active(7_000).await;
}
#[tokio::test(start_paused = true)]
async fn issue356_judge_near_boundary_stream_survives() {
    active(7_199).await;
}

#[tokio::test(start_paused = true)]
async fn issue356_judge_stalls_from_last_activity() {
    for gap in [1, 3_600, 7_199] {
        let started = tokio::time::Instant::now();
        let error = judge_contract(
            &Streaming {
                gap,
                silent_after: true,
            },
            contract(),
            &expected(),
        )
        .await
        .unwrap_err();
        assert!(
            JudgeIncomplete::caused(&error).is_some(),
            "a stall never produces a verdict"
        );
        assert_eq!(
            started.elapsed().as_secs(),
            2 * gap + JUDGE_TIMEOUT_SECS,
            "last activity must renew the full window"
        );
    }
}

struct PartialStream(std::sync::atomic::AtomicUsize);
#[async_trait]
impl WorkflowLlmClient for PartialStream {
    async fn send_message(
        &self,
        _: Vec<serde_json::Value>,
        _: Vec<serde_json::Value>,
        _: Vec<serde_json::Value>,
        _: &str,
    ) -> WorkflowResult<WorkflowAgentOutcome> {
        unreachable!()
    }
    async fn send_message_with_progress(
        &self,
        _: Vec<serde_json::Value>,
        _: Vec<serde_json::Value>,
        _: Vec<serde_json::Value>,
        _: &str,
        _: f64,
        progress: archon_shell::progress::Progress,
    ) -> WorkflowResult<WorkflowAgentOutcome> {
        if self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst) > 0 {
            std::future::pending::<()>().await;
        }
        for _ in 0..2 {
            tokio::time::sleep(Duration::from_secs(4_500)).await;
            progress.record();
        }
        Ok(WorkflowAgentOutcome {
            content: HEAD.into(),
            stop_reason: Some("max_tokens".into()),
            ..Default::default()
        })
    }
}

#[tokio::test(start_paused = true)]
async fn issue356_judge_stall_keeps_saved_reply_and_resume_continues_it() {
    use crate::command::workflow_freeze_budget::{FreezeIncomplete, FreezeProgress};
    let temp = tempfile::tempdir().unwrap();
    let progress = FreezeProgress::default();
    progress.saved(false); // A probe verdict saved before this judge call.
    let partial = PartialReply::new(temp.path(), "batch", &progress);
    let client = PartialStream(std::sync::atomic::AtomicUsize::new(0));
    let error = judge_contract_resumable(&client, contract(), &expected(), Some(&partial))
        .await
        .unwrap_err();
    assert!(JudgeIncomplete::caused(&error).is_some());
    assert_eq!(
        progress.total(),
        2,
        "the earlier probe and the saved partial reply survive the stall"
    );
    assert_eq!(partial.open_reply(), Some((HEAD.into(), 1)));
    let incomplete = FreezeIncomplete::stalled(error.to_string(), &progress);
    assert!(incomplete.report().ends_with("archon-host-progress: 2"));
    let resumed = Scripted::new(vec![Ok((TAIL, Some("end_turn")))]);
    let judged = judge_contract_resumable(&resumed, contract(), &expected(), Some(&partial))
        .await
        .unwrap();
    assert_eq!(
        judged.acceptance[0].judgment.verdict,
        JudgeDecision::Accepted
    );
    assert_eq!(resumed.calls()[0][1]["content"], HEAD);
}

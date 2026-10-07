//! A real HTTP read-idle expiration is stopped, never an ordinary lint error.
use super::*;
use crate::runtime::llm::transport_tests::{PING, client_for, openai_delta, ping, settle};
use std::sync::Arc;

async fn lint(client: Arc<dyn archon_workflow::WorkflowLlmClient>, cache: &Path) -> Result<()> {
    use crate::command::topology_lint::{
        fidelity_resume,
        fidelity_store::{StoreIdentity, VerdictStore},
    };
    let store = VerdictStore::new(cache.into(), StoreIdentity::of(client.as_ref()));
    let inputs = [(
        vec![ClaimedObligation {
            id: "REQ-X-001".into(),
            text: "required behavior".into(),
        }],
        vec![ClaimingTask {
            task_id: "TASK-X-001".into(),
            text: "required behavior".into(),
        }],
        "batch".into(),
    )];
    let resume = crate::command::workflow_freeze_budget::FreezeResume::saving(
        FreezeBudget::unlimited(),
        true,
    );
    fidelity_resume::resolve(
        client.as_ref(),
        &store,
        &inputs,
        &SkeletonSummary::absent(),
        &resume,
    )
    .await
    .map(|_| ())
}

async fn idle_after(frame: Option<&str>) {
    for local in [false, true] {
        idle_after_for(frame, local).await;
    }
}
async fn idle_after_for(frame: Option<&str>, local: bool) {
    tokio::task::LocalSet::new()
        .run_until(async move {
            let (client, mut transport) = client_for(Some(1), true, local).await;
            let cache = tempfile::tempdir().unwrap();
            let critic = tokio::task::spawn_local(async move { lint(client, cache.path()).await });
            transport.ready().await;
            if let Some(frame) = frame {
                let frame = match (local, frame == PING) {
                    (true, true) => ping(true).into(),
                    (true, false) => openai_delta("{", false),
                    (false, _) => frame.to_string(),
                };
                transport.frame(&frame).await;
            }
            tokio::time::pause();
            tokio::time::advance(Duration::from_secs(1)).await;
            settle().await;
            assert!(
                critic.is_finished(),
                "the transport must stop at its 1s idle backstop, before the 7200s critic watchdog"
            );
            let error = critic
                .await
                .unwrap()
                .expect_err("an idle stream yields no verdict");
            assert!(
                crate::command::topology_lint::fidelity_resume::LintIncomplete::caused(&error)
                    .is_some(),
                "HTTP idle expiration must be resumable: {error:#}"
            );
            tokio::time::resume();
        })
        .await;
}

#[tokio::test]
async fn issue356_transport_critic_idle_before_text() {
    idle_after(None).await;
}
#[tokio::test]
async fn issue356_transport_critic_idle_after_ping() {
    idle_after(Some(PING)).await;
}
#[tokio::test]
async fn issue356_transport_critic_idle_after_partial_text() {
    idle_after(Some("event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"{\"}}\n\n")).await;
}

#[tokio::test]
async fn issue356_transport_critic_idle_before_headers() {
    tokio::task::LocalSet::new()
        .run_until(async move {
            for local in [false, true] {
            let (client, mut transport) =
                client_for(Some(1), false, local).await;
            let cache = tempfile::tempdir().unwrap();
            let critic = tokio::task::spawn_local(async move { lint(client, cache.path()).await });
            transport.ready().await;
            tokio::time::pause();
            tokio::time::advance(Duration::from_secs(1)).await;
            settle().await;
            assert!(
                critic.is_finished(),
                "the transport must stop at its 1s idle backstop, before the 7200s critic watchdog"
            );
            let error = critic
                .await
                .unwrap()
                .expect_err("header silence yields no verdict");
            assert!(
                crate::command::topology_lint::fidelity_resume::LintIncomplete::caused(&error)
                    .is_some(),
                "HTTP header silence must pause: {error:#}"
            );
            tokio::time::resume();
            }
        })
        .await;
}

//! Real configured provider transport must outlast the judge activity window.
use super::*;
use crate::runtime::llm::transport_tests::{PING, answer, client_for, openai_delta, settle};
use std::time::Duration;

async fn active_then_stall(gap: u64, local: bool) {
    let (client, mut transport) = client_for(None, true, local).await;
    let judge =
        tokio::spawn(async move { judge_contract(client.as_ref(), contract(), &expected()).await });
    transport.ready().await;
    tokio::time::pause();
    for _ in 0..3 {
        tokio::time::advance(Duration::from_secs(gap)).await;
        settle().await;
        assert!(
            !judge.is_finished(),
            "real HTTP read backstop cut an active judge before its {JUDGE_TIMEOUT_SECS}s window"
        );
        transport
            .frame(&if local {
                openai_delta(" ", false)
            } else {
                PING.into()
            })
            .await;
    }
    transport
        .frame(&if local {
            openai_delta(ACCEPTED, true)
        } else {
            answer(ACCEPTED)
        })
        .await;
    let judged = judge.await.unwrap().unwrap();
    assert_eq!(
        judged.acceptance[0].judgment.verdict,
        JudgeDecision::Accepted
    );
    tokio::time::resume();

    // Same real path, then genuine silence; last ping starts a fresh window.
    let (client, mut transport) = client_for(None, true, local).await;
    let judge =
        tokio::spawn(async move { judge_contract(client.as_ref(), contract(), &expected()).await });
    transport.ready().await;
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(3600)).await;
    settle().await;
    transport
        .frame(&if local {
            openai_delta(" ", false)
        } else {
            PING.into()
        })
        .await;
    tokio::time::advance(Duration::from_secs(JUDGE_TIMEOUT_SECS - 1)).await;
    settle().await;
    assert!(!judge.is_finished());
    tokio::time::advance(Duration::from_secs(1)).await;
    settle().await;
    let error = judge.await.unwrap().unwrap_err();
    assert!(JudgeIncomplete::caused(&error).is_some(), "{error:#}");
    assert!(
        error.to_string().contains("no provider progress"),
        "{error:#}"
    );
    tokio::time::resume();
}
#[tokio::test]
async fn issue356_transport_judge_hour_gaps() {
    active_then_stall(3600, false).await;
}
#[tokio::test]
async fn issue356_transport_judge_slow_gaps() {
    active_then_stall(7000, false).await;
}
#[tokio::test]
async fn issue356_transport_judge_boundary_gaps() {
    active_then_stall(7199, false).await;
}

#[tokio::test]
async fn issue356_transport_local_judge_hour_gaps() {
    active_then_stall(3600, true).await;
}
#[tokio::test]
async fn issue356_transport_local_judge_slow_gaps() {
    active_then_stall(7000, true).await;
}
#[tokio::test]
async fn issue356_transport_local_judge_boundary_gaps() {
    active_then_stall(7199, true).await;
}

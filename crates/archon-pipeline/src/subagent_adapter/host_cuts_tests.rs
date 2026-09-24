use super::*;
use archon_tools::subagent_activity;
use tokio::time::Instant;

const N: Duration = Duration::from_secs(1_800);
const WALL: u64 = 14_400;

fn secs(value: u64) -> Duration {
    Duration::from_secs(value)
}

/// A session that honours cancellation the way the executor does: whatever it
/// was doing, a cancel ends it as `Cancelled`.
fn session(
    cancel: &CancellationToken,
    work: impl Future<Output = ()> + Send + 'static,
) -> SessionRun {
    let cancel = cancel.clone();
    Box::pin(async move {
        tokio::select! {
            _ = cancel.cancelled() => SubagentOutcome::Cancelled,
            () = work => SubagentOutcome::Completed("done".into()),
        }
    })
}

async fn drive_bounded(
    work: impl Future<Output = ()> + Send + 'static,
    limit: Option<Duration>,
    wall: Option<u64>,
) -> (SubagentOutcome, Option<HostCut>, Duration) {
    let cancel = CancellationToken::new();
    let bound = InactivityBound::new(limit);
    let mut run = session(&cancel, work);
    if let Some(bound) = &bound {
        run = bound.install("session", run);
    }
    let started = Instant::now();
    let (outcome, cut) = drive(run, &cancel, wall, bound.as_ref()).await;
    (outcome, cut, Instant::now() - started)
}

/// Case A: the session's last request stalled and nothing came back. It is cut
/// at the inactivity bound, hours before its wall clock.
#[tokio::test(start_paused = true)]
async fn a_silent_session_is_cut_for_inactivity_at_the_bound() {
    let (outcome, cut, elapsed) = drive_bounded(
        async {
            subagent_activity::note();
            std::future::pending::<()>().await;
        },
        Some(N),
        Some(WALL),
    )
    .await;
    assert_eq!(elapsed, N);
    assert_eq!(
        cut,
        Some(HostCut::Inactivity {
            silent: N,
            limit: N
        })
    );
    let error = inactivity_failure(&outcome, cut).expect("an inactivity cut is an error");
    assert!(
        subagent_activity::is_inactivity_timeout_text(&error.to_string()),
        "{error}"
    );
}

/// Case B: every tool call is slow (each runs most of the bound), but the
/// session keeps calling tools and reading their results. It is never cut,
/// though its total run is several bounds long.
#[tokio::test(start_paused = true)]
async fn slow_but_continuous_tool_calls_are_never_cut() {
    let (outcome, cut, elapsed) = drive_bounded(
        async {
            for _ in 0..8 {
                // The model thinks, then calls a tool that takes its time.
                tokio::time::sleep(secs(900)).await;
                subagent_activity::note();
                let round = subagent_activity::tool_round();
                tokio::time::sleep(secs(1_500)).await;
                drop(round);
            }
        },
        Some(N),
        Some(WALL * 2),
    )
    .await;
    assert_eq!(cut, None);
    assert!(matches!(outcome, SubagentOutcome::Completed(_)));
    assert_eq!(elapsed, secs(8 * 2_400));
    assert!(elapsed > N * 10);
}

/// The stated rule for a long in-flight tool call: it is activity for as long
/// as it runs, however long — tools carry their own bounds, and the wall clock
/// still bounds a tool that has none. The silence that follows its result is
/// measured from the result.
#[tokio::test(start_paused = true)]
async fn a_long_in_flight_tool_call_is_activity_and_silence_counts_from_its_result() {
    let (outcome, cut, elapsed) = drive_bounded(
        async {
            let round = subagent_activity::tool_round();
            tokio::time::sleep(N * 3).await;
            drop(round);
            std::future::pending::<()>().await;
        },
        Some(N),
        Some(WALL),
    )
    .await;
    assert_eq!(elapsed, N * 4, "cut one bound after the tool returned");
    assert!(matches!(cut, Some(HostCut::Inactivity { .. })));
    assert!(inactivity_failure(&outcome, cut).is_some());
}

/// The two bounds end a session with records that cannot be confused.
#[tokio::test(start_paused = true)]
async fn wall_clock_and_inactivity_cuts_are_told_apart() {
    // Active throughout, so only the wall clock can end it.
    let (wall_outcome, wall_cut, wall_elapsed) = drive_bounded(
        async {
            loop {
                tokio::time::sleep(secs(60)).await;
                subagent_activity::note();
            }
        },
        Some(N),
        Some(7_200),
    )
    .await;
    assert_eq!(wall_elapsed, secs(7_200));
    assert_eq!(wall_cut, Some(HostCut::WallClock));
    assert!(inactivity_failure(&wall_outcome, wall_cut).is_none());
    let wall_text =
        crate::subagent_adapter::llm_response_for_subagent_outcome(wall_outcome, true, Some(7_200))
            .expect_err("a wall-clock cut is an error")
            .to_string();

    let (idle_outcome, idle_cut, _) = drive_bounded(
        async {
            subagent_activity::note();
            std::future::pending::<()>().await;
        },
        Some(N),
        Some(7_200),
    )
    .await;
    let idle_text = inactivity_failure(&idle_outcome, idle_cut)
        .expect("an inactivity cut is an error")
        .to_string();

    assert!(
        wall_text.contains("subagent timed out after 7200s"),
        "{wall_text}"
    );
    assert!(!subagent_activity::is_inactivity_timeout_text(&wall_text));
    assert!(subagent_activity::is_inactivity_timeout_text(&idle_text));
    assert!(!idle_text.contains("timed out after"), "{idle_text}");
}

/// A session still queued for a subagent slot has not started, so only the
/// wall clock can end it.
#[tokio::test(start_paused = true)]
async fn a_session_queued_before_it_starts_is_never_cut_for_inactivity() {
    let (outcome, cut, elapsed) = drive_bounded(
        async {
            // Waiting for capacity: the runner has not reported anything yet.
            tokio::time::sleep(N * 3).await;
            subagent_activity::note();
            tokio::time::sleep(N / 2).await;
        },
        Some(N),
        Some(WALL),
    )
    .await;
    assert_eq!(cut, None);
    assert!(matches!(outcome, SubagentOutcome::Completed(_)));
    assert_eq!(elapsed, N * 3 + N / 2);
}

/// Off means off: no clock is installed, and a session silent for many bounds
/// runs until it finishes on its own.
#[tokio::test(start_paused = true)]
async fn a_disabled_bound_never_cuts() {
    assert!(InactivityBound::new(None).is_none());
    let client = crate::subagent_adapter::SubagentPipelineClient::new(
        Arc::new(crate::subagent_adapter::tests::NoopClient),
        archon_tools::tool::ToolContext::default(),
    );
    assert_eq!(client.inactivity_timeout, None);
    let client = client.with_inactivity_timeout(Some(0));
    assert_eq!(client.inactivity_timeout, None);
    assert_eq!(
        client.with_inactivity_timeout(Some(60)).inactivity_timeout,
        Some(secs(60))
    );

    let (outcome, cut, elapsed) = drive_bounded(
        async {
            assert!(subagent_activity::current().is_none(), "no clock installed");
            tokio::time::sleep(N * 10).await;
        },
        None,
        None,
    )
    .await;
    assert_eq!(cut, None);
    assert!(matches!(outcome, SubagentOutcome::Completed(_)));
    assert_eq!(elapsed, N * 10);
}

/// A session that finished in the instant the bound fired keeps its answer.
#[test]
fn a_completed_session_is_never_reported_inactive() {
    let cut = Some(HostCut::Inactivity {
        silent: N,
        limit: N,
    });
    assert!(inactivity_failure(&SubagentOutcome::Completed("x".into()), cut).is_none());
    assert!(inactivity_failure(&SubagentOutcome::Cancelled, cut).is_some());
    assert!(inactivity_failure(&SubagentOutcome::Cancelled, Some(HostCut::WallClock)).is_none());
}

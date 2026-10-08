use super::*;
use crate::subagent_activity;

fn secs(value: u64) -> Duration {
    Duration::from_secs(value)
}

async fn host_progress_scope<T>(
    agent_id: &str,
    clock: Arc<DispatchClock>,
    work: impl std::future::Future<Output = T>,
) -> T {
    let activity = subagent_activity::ActivityClock::new();
    let activity_session = subagent_activity::SessionClock {
        agent_id: agent_id.to_string(),
        clock: activity,
    };
    scope_session(
        agent_id.to_string(),
        vec![clock],
        subagent_activity::scope(activity_session, work),
    )
    .await
}

/// Issue 288: a clock is pending until its call takes a slot. Dispatch,
/// setup and a queue before admission count nothing; the run time starts at
/// admission, once (a retry admitted again changes nothing).
#[tokio::test(start_paused = true)]
async fn a_clock_starts_at_admission_not_at_dispatch() {
    let clock = DispatchClock::new();
    tokio::time::sleep(secs(7_200)).await;
    assert!(!clock.is_admitted());
    assert_eq!(clock.elapsed(), Duration::ZERO, "nothing before admission");
    clock.admit();
    tokio::time::sleep(secs(30)).await;
    clock.admit();
    assert_eq!(clock.elapsed(), secs(30));
    let waiter = {
        let clock = Arc::clone(&clock);
        tokio::spawn(async move { clock.exceeding(secs(100)).await })
    };
    tokio::time::sleep(secs(69)).await;
    assert!(!waiter.is_finished());
    tokio::time::sleep(secs(1)).await;
    tokio::task::yield_now().await;
    waiter.await.unwrap();
}

/// A wait reported before admission keeps the clock at zero, and admission
/// inside the wait starts it only when the wait ends: a queued call is timed
/// from the moment it holds its slot.
#[tokio::test(start_paused = true)]
async fn admission_inside_a_wait_starts_the_clock_when_the_wait_ends() {
    let clock = DispatchClock::new();
    let wait = clock.pause_for_slot();
    tokio::time::sleep(secs(10_000)).await;
    clock.admit();
    assert_eq!(clock.elapsed(), Duration::ZERO);
    drop(wait);
    tokio::time::sleep(secs(5)).await;
    assert_eq!(clock.elapsed(), secs(5));
    assert_eq!(clock.cut(secs(5)).await, DispatchCut::Execution(secs(5)));
}

/// A call whose executor never reports a slot or a wait is not unbounded:
/// the time outside any reported wait is cut at the limit, and the cut says
/// the admission report is missing. Reported waits do not count toward it.
#[tokio::test(start_paused = true)]
async fn a_call_never_admitted_is_cut_and_named() {
    let clock = DispatchClock::new();
    let started = Instant::now();
    let wait = clock.pause_for_slot();
    tokio::time::sleep(secs(500)).await;
    drop(wait);
    let cut = clock.cut(secs(100)).await;
    assert_eq!(cut, DispatchCut::NeverAdmitted(secs(100)));
    assert_eq!(
        Instant::now() - started,
        secs(600),
        "the reported wait did not count"
    );
    assert!(
        cut.to_string().contains("missing admission report"),
        "{cut}"
    );
    assert_eq!(clock.elapsed(), Duration::ZERO);
}

#[tokio::test(start_paused = true)]
async fn the_clock_counts_execution_and_not_the_slot_wait() {
    let clock = DispatchClock::new();
    clock.admit();
    tokio::time::sleep(secs(10)).await;
    let pause = clock.pause_for_slot();
    assert!(clock.waiting_for_slot());
    tokio::time::sleep(secs(500)).await;
    assert_eq!(clock.elapsed(), secs(10), "the wait is not counted");
    drop(pause);
    tokio::time::sleep(secs(5)).await;
    assert_eq!(clock.elapsed(), secs(15));
}

#[tokio::test(start_paused = true)]
async fn a_deadline_never_fires_during_a_slot_wait() {
    let clock = DispatchClock::new();
    clock.admit();
    let pause = clock.pause_for_slot();
    let started = Instant::now();
    let waiter = {
        let clock = Arc::clone(&clock);
        tokio::spawn(async move { clock.exceeding(secs(100)).await })
    };
    tokio::time::sleep(secs(1_000)).await;
    assert!(!waiter.is_finished(), "a queued call is never cut");
    drop(pause);
    waiter.await.unwrap();
    assert_eq!(
        Instant::now() - started,
        secs(1_100),
        "the bound starts when the slot is acquired"
    );
}

#[tokio::test(start_paused = true)]
async fn within_excludes_slot_waits_reported_by_a_session() {
    let output = within(secs(100), async {
        let call = current_call().expect("the call clock is installed");
        scope_session("agent-a", vec![call], async {
            let pause = slot_wait("agent-a").expect("clocks for this session");
            tokio::time::sleep(secs(1_000)).await;
            assert!(admitted("agent-a"));
            drop(pause);
            tokio::time::sleep(secs(60)).await;
            "ran"
        })
        .await
    })
    .await;
    assert_eq!(output, Ok("ran"));
}

#[tokio::test(start_paused = true)]
async fn within_cuts_only_after_a_full_no_progress_window() {
    let started = Instant::now();
    let output = within(secs(100), async {
        let call = current_call().expect("the call clock is installed");
        host_progress_scope("agent-a", call, async {
            assert!(admitted("agent-a"));
            tokio::time::sleep(secs(90)).await;
            progress("new tool call 1");
            tokio::time::sleep(secs(90)).await;
            progress("new tool call 2");
            tokio::time::sleep(secs(90)).await;
        })
        .await
    })
    .await;
    assert_eq!(
        output,
        Ok(()),
        "novel progress renews the window repeatedly"
    );
    assert_eq!(Instant::now() - started, secs(270));
    // Work that never reaches an executor is cut too, and named.
    let output = within(secs(100), tokio::time::sleep(secs(1_000))).await;
    assert_eq!(output, Err(DispatchCut::NeverAdmitted(secs(100))));
}

#[tokio::test(start_paused = true)]
async fn a_progress_event_resets_the_deadline_and_a_later_stall_is_bounded() {
    let clock = DispatchClock::new();
    host_progress_scope("progress", Arc::clone(&clock), async {
        clock.admit();
        tokio::time::sleep(secs(99)).await;
        progress("new assistant text");
        tokio::time::sleep(secs(99)).await;
        assert_eq!(clock.elapsed(), secs(99));
        assert_eq!(clock.last_progress().as_deref(), Some("new assistant text"));
        assert_eq!(
            clock.cut(secs(100)).await,
            DispatchCut::Execution(secs(100))
        );
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn repeated_progress_events_keep_a_long_call_alive_without_removing_its_bound() {
    let clock = DispatchClock::new();
    host_progress_scope("repeated-progress", Arc::clone(&clock), async {
        clock.admit();
        for step in 0..5 {
            tokio::time::sleep(secs(99)).await;
            progress(&format!("new tool call {step}"));
            assert_eq!(clock.elapsed(), Duration::ZERO);
        }
        tokio::time::sleep(secs(100)).await;
        assert_eq!(
            clock.cut(secs(100)).await,
            DispatchCut::Execution(secs(100))
        );
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn progress_during_a_slot_wait_renews_the_window_after_admission_resumes() {
    let clock = DispatchClock::new();
    host_progress_scope("wait-progress", Arc::clone(&clock), async {
        clock.admit();
        tokio::time::sleep(secs(80)).await;
        let pause = clock.pause_for_slot();
        tokio::time::sleep(secs(500)).await;
        progress("new tool call during the wait");
        drop(pause);
        tokio::time::sleep(secs(99)).await;
        assert_eq!(clock.elapsed(), secs(99));
        assert_eq!(
            clock.cut(secs(100)).await,
            DispatchCut::Execution(secs(100))
        );
    })
    .await;
}

/// Activity is not progress: output, a tool round and its end keep the
/// inactivity bound away but never renew the no-progress window, so a session
/// that stays busy repeating itself is cut and the cut names the last novel
/// activity it made.
#[tokio::test(start_paused = true)]
async fn activity_without_novel_progress_never_renews_the_window() {
    let output = within_named(secs(100), async {
        let call = current_call().expect("the call clock is installed");
        host_progress_scope("busy", call, async {
            assert!(admitted("busy"));
            progress("turn 1: new tool call Read a.rs");
            loop {
                tokio::time::sleep(secs(10)).await;
                subagent_activity::note();
                drop(subagent_activity::tool_round());
            }
        })
        .await
    })
    .await;
    let (cut, last) = output.expect_err("a busy loop is cut");
    assert_eq!(cut, DispatchCut::Execution(secs(100)));
    assert_eq!(last, "last novel activity: turn 1: new tool call Read a.rs");
    assert_eq!(
        last_progress_text(None),
        "last novel activity: none since the window opened"
    );
}

#[tokio::test]
async fn a_child_session_never_stops_its_parents_clocks() {
    let clock = DispatchClock::new();
    scope_session("parent", vec![Arc::clone(&clock)], async {
        assert!(
            !admitted("child"),
            "a child's admission is not the parent's"
        );
        assert!(!clock.is_admitted());
        assert!(slot_wait("child").is_none());
        assert!(!clock.waiting_for_slot());
        let pause = slot_wait("parent").expect("the parent's own wait");
        assert!(clock.waiting_for_slot());
        drop(pause);
        assert!(!clock.waiting_for_slot());
    })
    .await;
    assert!(slot_wait("parent").is_none(), "outside the scope");
}

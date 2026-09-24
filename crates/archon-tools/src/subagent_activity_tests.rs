use super::*;

const LIMIT: Duration = Duration::from_secs(600);

#[tokio::test(start_paused = true)]
async fn silence_past_the_bound_is_reported_with_its_length() {
    let clock = ActivityClock::new();
    clock.touch();
    let started = Instant::now();
    let silent = silence_exceeding(&clock, LIMIT).await;
    assert_eq!(silent, LIMIT);
    assert_eq!(Instant::now() - started, LIMIT);
}

#[tokio::test(start_paused = true)]
async fn activity_before_the_bound_moves_the_deadline() {
    let clock = ActivityClock::new();
    clock.touch();
    let started = Instant::now();
    let toucher = {
        let clock = Arc::clone(&clock);
        async move {
            for _ in 0..5 {
                tokio::time::sleep(Duration::from_secs(500)).await;
                clock.touch();
            }
        }
    };
    let (silent, ()) = tokio::join!(silence_exceeding(&clock, LIMIT), toucher);
    assert_eq!(silent, LIMIT);
    // Five touches 500s apart, then a full bound of silence after the last.
    assert_eq!(Instant::now() - started, Duration::from_secs(2500) + LIMIT);
}

#[tokio::test(start_paused = true)]
async fn an_in_flight_tool_round_is_activity_and_its_end_restarts_the_clock() {
    let clock = ActivityClock::new();
    let started = Instant::now();
    let round = clock.tool_round();
    let slow_tool = async move {
        // Three bounds' worth of one tool call: never silence.
        tokio::time::sleep(LIMIT * 3).await;
        drop(round);
    };
    let (silent, ()) = tokio::join!(silence_exceeding(&clock, LIMIT), slow_tool);
    assert_eq!(silent, LIMIT);
    assert_eq!(Instant::now() - started, LIMIT * 4);
}

/// A session queued for a subagent slot has not started, so it cannot be
/// silent: the clock runs from the runner's first activity.
#[tokio::test(start_paused = true)]
async fn a_session_that_has_not_started_is_never_silent() {
    let clock = ActivityClock::new();
    let started = Instant::now();
    assert_eq!(clock.silent_since(), None);
    let queued = {
        let clock = Arc::clone(&clock);
        async move {
            tokio::time::sleep(LIMIT * 5).await;
            clock.touch();
        }
    };
    let (silent, ()) = tokio::join!(silence_exceeding(&clock, LIMIT), queued);
    assert_eq!(silent, LIMIT);
    assert_eq!(Instant::now() - started, LIMIT * 6);
}

#[tokio::test]
async fn the_clock_crosses_a_spawn_only_when_inherited_for_its_own_session() {
    let session = SessionClock {
        agent_id: "session-a".into(),
        clock: ActivityClock::new(),
    };
    let (own, child) = scope(session, async {
        assert!(current().is_some());
        let own = tokio::spawn(inherit(current_for("session-a"), async {
            current().is_some()
        }));
        // A subagent the session spawns has its own id: nothing is inherited.
        let child = tokio::spawn(inherit(current_for("child-b"), async {
            current().is_some()
        }));
        (own.await.unwrap(), child.await.unwrap())
    })
    .await;
    assert!(own);
    assert!(!child);
    assert!(current().is_none());
    note();
    assert!(tool_round().is_none());
}

#[test]
fn the_cut_text_carries_its_marker_and_no_wall_clock_phrase() {
    let text = inactivity_error_text(Duration::from_secs(3601), LIMIT);
    assert!(is_inactivity_timeout_text(&text), "{text}");
    let lower = text.to_ascii_lowercase();
    for wall in ["timed out after", "wall-clock timeout", "deadline exceeded"] {
        assert!(!lower.contains(wall), "{text}");
    }
    assert!(text.contains("3601s"), "{text}");
}

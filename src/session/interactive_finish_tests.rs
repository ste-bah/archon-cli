use archon_tui::app::TuiEvent;

fn report() -> archon_memory::garden::GardenReport {
    archon_memory::garden::GardenReport {
        duplicates_merged: 0,
        stale_pruned: 0,
        importance_decayed: 0,
        fragments_merged: 0,
        overflow_pruned: 0,
        total_memories_before: 10,
        total_memories_after: 10,
        duration_ms: 5,
        review_pairs: Vec::new(),
        semantic_pass_unavailable: false,
    }
}

/// A pass that changed nothing says nothing.
///
/// Consolidation fires on every session start once the throttle elapses. A
/// line on every launch is noise people learn to skip, and a notice everyone
/// skips is no more visible than the log line it replaced.
#[test]
fn a_no_op_consolidation_is_silent() {
    assert!(super::auto_consolidation_summary(&report(), 0).is_none());
    // Importance decay alone is bookkeeping, not a change to what is
    // remembered, so it does not warrant interrupting a session start.
    let decayed = archon_memory::garden::GardenReport {
        importance_decayed: 12,
        ..report()
    };
    assert!(super::auto_consolidation_summary(&decayed, 0).is_none());
}

/// Anything that removed or altered a memory has to be reported.
#[test]
fn destructive_outcomes_are_reported() {
    for (label, built) in [
        (
            "duplicate",
            archon_memory::garden::GardenReport {
                duplicates_merged: 2,
                ..report()
            },
        ),
        (
            "fragment",
            archon_memory::garden::GardenReport {
                fragments_merged: 1,
                ..report()
            },
        ),
        (
            "stale",
            archon_memory::garden::GardenReport {
                stale_pruned: 3,
                ..report()
            },
        ),
        (
            "overflow",
            archon_memory::garden::GardenReport {
                overflow_pruned: 4,
                ..report()
            },
        ),
    ] {
        assert!(
            super::auto_consolidation_summary(&built, 0).is_some(),
            "{label} removals must be surfaced"
        );
    }
}

fn pending(count: usize) -> archon_memory::garden::GardenReport {
    archon_memory::garden::GardenReport {
        review_pairs: (0..count)
            .map(|i| archon_memory::garden::ReviewPair {
                a_id: format!("a{i}"),
                b_id: format!("b{i}"),
                a_content: "one".into(),
                b_content: "two".into(),
            })
            .collect(),
        ..report()
    }
}

/// Pending review pairs are surfaced even though nothing was changed: they
/// are the work `/garden` would do next, and the only prompt to run it.
#[test]
fn pending_review_pairs_are_surfaced() {
    let summary = super::auto_consolidation_summary(&pending(1), 0).expect("summary");
    assert_eq!(
        summary, "1 pair(s) awaiting review",
        "the panel entry is prefixed with \"Memory garden:\" by the splash builder,          so the summary itself must be the bare description"
    );
}

/// Pairs the adjudicator settled are counted as settled, not as still pending.
///
/// The report is a snapshot taken before adjudication ran, so its
/// `review_pairs` count is stale by the time this line is drawn. Showing it raw
/// would tell the user work is outstanding that has already been done — and the
/// panel exists precisely so the memory changes made behind their back are
/// legible.
#[test]
fn adjudicated_pairs_are_reported_as_merged_and_removed_from_the_backlog() {
    let summary = super::auto_consolidation_summary(&pending(7), 3).expect("summary");
    assert_eq!(
        summary,
        "3 pair(s) merged after review, 4 pair(s) awaiting review"
    );

    // A band cleared completely leaves no backlog to mention.
    let summary = super::auto_consolidation_summary(&pending(2), 2).expect("summary");
    assert_eq!(summary, "2 pair(s) merged after review");
}

/// A pass that never ran is surfaced, even though it changed nothing.
///
/// This is the state every Archon process but the first is in: CozoDB admits
/// one writer, so the rest read memory over TCP. Reported as an absence rather
/// than a count, because the counts it would otherwise show are zeroes it never
/// measured.
#[test]
fn an_unavailable_semantic_pass_is_surfaced() {
    let unavailable = archon_memory::garden::GardenReport {
        semantic_pass_unavailable: true,
        ..report()
    };
    let summary = super::auto_consolidation_summary(&unavailable, 0).expect("summary");
    assert_eq!(summary, "semantic pass unavailable (second instance)");
    // The splash column truncates past roughly fifty characters, and a notice
    // clipped to "semantic pass unavailable (second inst..." says less than
    // nothing.
    assert!(summary.len() <= 50, "summary must fit the splash column");
}

#[tokio::test]
async fn explicit_resume_rejects_history_atomically_when_capacity_is_insufficient() {
    let (tx, mut rx) = archon_tui::event_channel::bounded_tui_event_channel_with_capacity(1);
    let messages = vec![serde_json::json!({
        "role": "assistant",
        "content": "x".repeat(archon_tui::event_channel::MAX_COALESCED_CONTENT_BYTES + 1)
    })];

    let result = super::replay_resumed_conversation(&tx, messages).await;

    assert!(result.is_none());
    assert!(rx.try_recv().is_err(), "partial history must not be queued");
}

#[tokio::test]
async fn explicit_resume_replays_history_into_tui() {
    let (tx, mut rx) = archon_tui::event_channel::bounded_tui_event_channel();
    let messages = vec![serde_json::json!({
        "role": "assistant",
        "content": "TAIL-SENTINEL-界🙂e\u{301}"
    })];

    super::display_initial_resume_history(&tx, &messages)
        .await
        .expect("queue history");

    let mut text = String::new();
    while let Ok(event) = rx.try_recv() {
        if let TuiEvent::TextDelta(delta) = event {
            text.push_str(&delta);
        }
    }
    assert!(text.contains("Resumed session history (1 messages)"));
    assert!(text.contains("TAIL-SENTINEL-界🙂e\u{301}"));
    assert!(text.contains("End of history"));
}

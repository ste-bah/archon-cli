//! L5 (Issue 360 review): a seed orders replies and gates by the script's own
//! ordinals, never by the wall clock.
use super::*;

fn refuted(id: &str) -> Value {
    finding(
        &format!("check '{id}' was refuted by the host judge; reason: weak"),
        id,
    )
}

fn version(id: &str, command: &str) -> Value {
    let mut entry = entry(id);
    entry["check"]["command"] = json!(command);
    entry
}

/// A clock stepped back between a gate and the next round: that round's
/// replies are still the replies since the gate.
#[test]
fn a_clock_stepped_back_after_a_gate_keeps_the_replies_since() {
    let weak = version("AC-2", "weak");
    let candidate = json!({"entries": [entry("AC-1"), weak]}).to_string();
    let records = vec![
        reply(
            "acceptance-author-AC-1-4",
            "04:00:00",
            &entry("AC-1").to_string(),
        ),
        reply("acceptance-author-AC-2-4", "04:01:00", &weak.to_string()),
        gate(
            "freeze-acceptance",
            "05:00:00",
            &candidate,
            vec![refuted("AC-2")],
            false,
        ),
        // Round 2 started at 03:00 by the stepped-back clock.
        reply(
            "acceptance-author-AC-2-7",
            "03:00:00",
            &entry("AC-2").to_string(),
        ),
        reply(
            "acceptance-author-AC-3-7",
            "03:00:01",
            &entry("AC-3").to_string(),
        ),
    ];
    let derived = derive(&records, &[], &criteria(&["AC-1", "AC-2", "AC-3"])).unwrap();
    let (gates, carried_candidate, replies, invalid, carried) = entries_seed(&derived);
    assert_eq!(gates.len(), 1);
    assert_eq!(carried_candidate.as_deref(), Some(candidate.as_str()));
    assert_eq!(
        replies
            .iter()
            .map(|r| r.call_id.as_str())
            .collect::<Vec<_>>(),
        ["acceptance-author-AC-2-7", "acceptance-author-AC-3-7"],
        "the repair and the new entry, not dropped as older than the gate"
    );
    assert!(invalid.is_empty());
    assert_eq!(carried, 3);
}

/// The latest reply of an entry or an artifact is its highest ordinal.
#[test]
fn the_latest_reply_is_the_highest_ordinal_whenever_it_started() {
    let records = vec![
        reply(
            "acceptance-author-AC-1-7",
            "01:00:00",
            &version("AC-1", "seven").to_string(),
        ),
        reply(
            "acceptance-author-AC-1-4",
            "02:00:00",
            &version("AC-1", "four").to_string(),
        ),
        reply("skeleton-author-2", "01:00:00", "skeleton two"),
        reply("skeleton-author-1", "02:00:00", "skeleton one"),
    ];
    let derived = derive(&records, &[], &criteria(&["AC-1"])).unwrap();
    let (_, _, replies, _, _) = entries_seed(&derived);
    assert_eq!(replies[0].call_id, "acceptance-author-AC-1-7");
    let SubjectSeed::Artifact {
        candidate,
        candidate_call,
        ..
    } = &derived.subjects["skeleton"]
    else {
        panic!()
    };
    assert_eq!(
        (candidate.as_str(), candidate_call.as_str()),
        ("skeleton two", "skeleton-author-2")
    );
}

/// The last gate is the one of the latest round its candidate holds, even
/// when an earlier gate's clock ran ahead; a later round that repeats the
/// judged entries does not move it past the replies it holds.
#[test]
fn the_last_gate_is_the_gate_of_the_latest_round() {
    let weak = version("AC-2", "weak");
    let first = json!({"entries": [entry("AC-1"), weak]}).to_string();
    let second = json!({"entries": [entry("AC-1"), entry("AC-2")]}).to_string();
    let records = vec![
        reply(
            "acceptance-author-AC-1-4",
            "01:00:00",
            &entry("AC-1").to_string(),
        ),
        reply("acceptance-author-AC-2-4", "01:01:00", &weak.to_string()),
        // This gate's clock ran ahead.
        gate(
            "freeze-acceptance",
            "09:00:00",
            &first,
            vec![refuted("AC-2")],
            false,
        ),
        reply(
            "acceptance-author-AC-2-7",
            "07:00:00",
            &entry("AC-2").to_string(),
        ),
        gate(
            "freeze-acceptance",
            "08:00:00",
            &second,
            vec![refuted("AC-1")],
            false,
        ),
        // Round 3 repeated AC-1 byte for byte: nothing it holds is new.
        reply(
            "acceptance-author-AC-1-10",
            "08:30:00",
            &entry("AC-1").to_string(),
        ),
    ];
    let derived = derive(&records, &[], &criteria(&["AC-1", "AC-2"])).unwrap();
    let (gates, carried_candidate, replies, invalid, carried) = entries_seed(&derived);
    assert_eq!(
        gates.iter().map(|g| g.call_id.as_str()).collect::<Vec<_>>(),
        ["freeze-acceptance-09:00:00", "freeze-acceptance-08:00:00"],
        "in round order"
    );
    assert_eq!(carried_candidate.as_deref(), Some(second.as_str()));
    assert!(
        replies
            .iter()
            .all(|r| serde_json::from_str::<Value>(&r.text).unwrap() == entry("AC-1")),
        "only the judged AC-1 again, which repairs nothing: {replies:?}"
    );
    assert!(invalid.is_empty());
    assert_eq!(carried, 2);
}

//! Issue-117: a targeted gap the round's own judge disposes of as
//! `resolved` is not reopened by a NEW gap naming the same file; every
//! other disposition, or none, leaves the gate exactly as strict as before.

use super::gate_tests::slot;
use super::tests::*;
use super::*;
use crate::v2::agent_adapter::{WorkflowV2AgentAdapter, WorkflowV2AgentRequest};
use crate::v2::{WorkflowV2HostCall, WorkflowV2HostOptions, WorkflowV2Status};
use serde_json::json;

const TARGETED: (&str, &str, &str) = (
    "gap-store",
    "high",
    "the store lane reads the timeframe, crates/shared/src/store.rs:3",
);
/// A new, unrelated problem in the same file the round fixed.
const UNRELATED: (&str, &str, &str) = (
    "gap-store-test",
    "medium",
    "crates/shared/src/store.rs:120 lacks a test",
);

/// A world whose plan holds one round for [`TARGETED`], with the call that
/// recorded it and the slot already in `calls`.
fn planned() -> (World, PlannedRound, Vec<WorkflowV2HostCall>) {
    let w = world();
    let recorded = verdict(
        "verification-wave-review-verify-task-a-1-2",
        &["TASK-A"],
        &[TARGETED],
    );
    w.save(&recorded);
    let round = w.plan().rounds[0].clone();
    let calls = vec![recorded.call.clone(), slot()];
    (w, round, calls)
}

/// The round's fix, then its judge recording `gaps` and, in its branch
/// result's data, `dispositions` (as the verification wave records it).
fn judged(
    w: &World,
    round: &PlannedRound,
    gaps: &[(&str, &str, &str)],
    dispositions: Option<Value>,
) -> Vec<WorkflowV2HostCall> {
    let tasks: Vec<&str> = round.tasks.iter().map(String::as_str).collect();
    let fix = execution(round, true, &[]).call;
    let mut check = execution(round, false, &[]).call;
    check.id = "verification-wave-review-verify-residual-8".into();
    w.save(&record(
        fix.clone(),
        WorkflowV2Status::Accepted,
        &tasks,
        &[],
    ));
    std::thread::sleep(std::time::Duration::from_millis(5));
    let mut judge = record(check.clone(), WorkflowV2Status::Accepted, &tasks, gaps);
    if let Some(dispositions) = dispositions {
        judge.result.data["outcomes"][0]["result"]["data"] =
            json!({ GAP_DISPOSITIONS_KEY: dispositions });
    }
    w.save(&judge);
    vec![fix, check]
}

fn gate(w: &World, calls: &[WorkflowV2HostCall]) -> ResidualVerdict {
    residual_verdict(calls, &w.store, Some(&w.universe), Some(w.root()))
}

fn blocks_gap_store(gate: &ResidualVerdict) -> bool {
    gate.blocking.iter().any(|b| b.contains("`gap-store`"))
}

#[test]
fn a_resolved_disposition_keeps_a_new_medium_on_the_same_file_a_new_gap() {
    let (w, round, mut calls) = planned();
    calls.extend(judged(
        &w,
        &round,
        &[UNRELATED],
        Some(json!([{"gap_id": "gap-store", "status": "resolved"}])),
    ));
    let gate = gate(&w, &calls);
    assert!(gate.blocking.is_empty(), "{gate:#?}");
    assert!(
        gate.notes
            .iter()
            .any(|n| n.contains(&round.key) && n.contains("resolved") && n.contains("gap-store")),
        "the targeted gap resolved: {gate:#?}"
    );
    assert!(
        gate.notes
            .iter()
            .any(|n| n.starts_with("warning:") && n.contains("gap-store-test")),
        "the new medium is weighed at its own severity: {gate:#?}"
    );
}

#[test]
fn a_resolved_disposition_leaves_a_new_high_on_the_same_file_blocking_by_its_own_name() {
    let (w, round, mut calls) = planned();
    calls.extend(judged(
        &w,
        &round,
        &[(
            "gap-store-race",
            "high",
            "crates/shared/src/store.rs:88 races",
        )],
        Some(json!([{"gap_id": "gap-store", "status": "resolved"}])),
    ));
    let gate = gate(&w, &calls);
    assert!(!blocks_gap_store(&gate), "{gate:#?}");
    assert_eq!(gate.blocking.len(), 1, "{gate:#?}");
    assert!(gate.blocking[0].contains("gap-store-race"), "{gate:#?}");
}

#[test]
fn without_a_readable_resolved_disposition_a_weighty_gap_on_the_same_file_reopens() {
    for dispositions in [
        None,
        Some(json!([])),
        Some(json!([{"gap_id": "gap-store", "status": "fixed"}])),
        Some(json!([{"gap_id": "gap-other", "status": "resolved"}])),
        Some(json!({"gap-store": "resolved"})),
        Some(json!([{"gap_id": "gap-store", "status": "resolved"},
            {"gap_id": "gap-store", "status": "open"}])),
    ] {
        let (w, round, mut calls) = planned();
        calls.extend(judged(&w, &round, &[UNRELATED], dispositions.clone()));
        let gate = gate(&w, &calls);
        assert!(blocks_gap_store(&gate), "{dispositions:?}: {gate:#?}");
    }
}

#[test]
fn an_open_disposition_holds_the_gap_even_with_nothing_recorded_again() {
    for id in ["gap-store", "  GAP-STORE "] {
        let (w, round, mut calls) = planned();
        calls.extend(judged(
            &w,
            &round,
            &[],
            Some(json!([{"gap_id": id, "status": "open"}])),
        ));
        let gate = gate(&w, &calls);
        assert!(blocks_gap_store(&gate), "{gate:#?}");
        assert!(gate.blocking[0].contains("open"), "{gate:#?}");
    }
}

#[test]
fn a_resolved_gap_its_judge_records_again_under_its_id_stands_at_any_severity() {
    for severity in ["low", "medium", "high"] {
        let (w, round, mut calls) = planned();
        calls.extend(judged(
            &w,
            &round,
            &[("gap-store", severity, "reworded, still there")],
            Some(json!([{"gap_id": "gap-store", "status": "resolved"}])),
        ));
        let gate = gate(&w, &calls);
        assert!(blocks_gap_store(&gate), "{severity}: {gate:#?}");
        assert!(gate.blocking[0].contains("again"), "{gate:#?}");
    }
}

#[test]
fn a_disposition_from_any_record_but_the_rounds_judge_is_not_read() {
    // On a later verifier of the same task.
    let (w, round, mut calls) = planned();
    calls.extend(judged(&w, &round, &[UNRELATED], None));
    std::thread::sleep(std::time::Duration::from_millis(5));
    let mut later = verdict(
        "verification-wave-review-verify-task-a-1-30",
        &["TASK-A"],
        &[],
    );
    later.result.data["outcomes"][0]["result"]["data"] =
        json!({ GAP_DISPOSITIONS_KEY: [{"gap_id": "gap-store", "status": "resolved"}] });
    w.save(&later);
    calls.push(later.call.clone());
    assert!(blocks_gap_store(&gate(&w, &calls)));
    // On the round's own fix.
    let (w, round, mut calls) = planned();
    calls.extend(judged(&w, &round, &[UNRELATED], None));
    let mut fix = w.store.load_call_record(&calls[2].id).unwrap().unwrap();
    fix.result.data[GAP_DISPOSITIONS_KEY] = json!([{"gap_id": "gap-store", "status": "resolved"}]);
    w.store.save_call_record(&fix).unwrap();
    assert!(blocks_gap_store(&gate(&w, &calls)));
    // On the verifier that recorded the gap in the first place.
    let (w, round, mut calls) = planned();
    let mut recorder = w.store.load_call_record(&calls[0].id).unwrap().unwrap();
    recorder.result.data[GAP_DISPOSITIONS_KEY] =
        json!([{"gap_id": "gap-store", "status": "resolved"}]);
    w.store.save_call_record(&recorder).unwrap();
    calls.extend(judged(&w, &round, &[UNRELATED], None));
    assert!(blocks_gap_store(&gate(&w, &calls)));
}

#[test]
fn the_rounds_verifier_and_an_adjudicator_are_asked_for_the_disposition() {
    let (w, round, _) = planned();
    let view = round_view(&round, &w.store);
    let asked = view["disposition_instruction"].as_str().unwrap();
    for needle in [
        GAP_DISPOSITIONS_KEY,
        "\"gap-store\"",
        "\"resolved\"",
        "\"open\"",
        "residual_gaps",
    ] {
        assert!(asked.contains(needle), "{needle}: {asked}");
    }
    assert!(
        !view["claim"]
            .as_str()
            .unwrap()
            .contains(GAP_DISPOSITIONS_KEY),
        "the fix is not asked: only the round's verifier disposes"
    );
    // An adjudication's one prompt is its claim, and it asks too.
    let w = world();
    w.save(&verdict(
        "verification-wave-review-verify-task-a-1-2",
        &["TASK-A"],
        &[(
            "gap-roster",
            "high",
            "the provider lanes are declared by no task",
        )],
    ));
    let round = w.plan().rounds[0].clone();
    assert_eq!(round.kind, RoundKind::Adjudication);
    let view = round_view(&round, &w.store);
    let claim = view["claim"].as_str().unwrap();
    assert!(
        claim.contains(GAP_DISPOSITIONS_KEY) && claim.contains("\"gap-roster\""),
        "{claim}"
    );
    assert_eq!(view["dispatchable"], true, "{view}");
}

fn verifier_request() -> WorkflowV2AgentRequest {
    WorkflowV2AgentRequest {
        call: WorkflowV2HostCall {
            id: "verification-wave-review-verify-residual-8-0".into(),
            method: WorkflowV2HostMethod::Agent,
            write_mode: None,
            options: WorkflowV2HostOptions::default(),
        },
        role: "verifier".into(),
        task: "verify the residual round".into(),
        constraints: Vec::new(),
        input: Value::Null,
        repository_root: None,
        project_artifacts: Default::default(),
        target_files: Vec::new(),
        target_ownership_scopes: Vec::new(),
    }
}

/// The verifier's reply as an agent writes it, through the adapter's own
/// parse, filed as the verification wave files a branch result.
#[test]
fn a_disposition_in_a_real_verifier_reply_reaches_the_gate_top_level_or_in_data() {
    let head = r#"{"status": "accepted", "summary": "the split now reads the instrument",
        "evidence": [{"kind": "test", "summary": "store tests pass"}],
        "commands_run": [{"kind": "test", "command": "cargo test -p shared store", "status": "succeeded", "exit_code": 0, "output_summary": "ok"}],
        "task_coverage": [{"task_id": "TASK-A", "status": "accepted", "summary": "fixed", "evidence": [{"kind": "test", "summary": "pass"}]}],
        "residual_gaps": [{"id": "gap-store-test", "severity": "medium", "description": "crates/shared/src/store.rs:120 lacks a test"}]"#;
    let entry = r#"[{"gap_id": "gap-store", "status": "resolved"}]"#;
    for (reply, resolved) in [
        (
            format!("Verified.\n{head}, \"gap_dispositions\": {entry}}}"),
            true,
        ),
        (
            format!("{head}, \"data\": {{\"note\": 1, \"gap_dispositions\": {entry}}}}}"),
            true,
        ),
        (format!("{head}}}"), false),
    ] {
        let parsed = WorkflowV2AgentAdapter::new()
            .parse_agent_output(&verifier_request(), &reply)
            .unwrap_or_else(|error| panic!("{reply}: {error}"));
        let (w, round, mut calls) = planned();
        calls.extend(judged(&w, &round, &[UNRELATED], None));
        let mut judge = w.store.load_call_record(&calls[3].id).unwrap().unwrap();
        judge.result.data["outcomes"][0]["result"] = serde_json::to_value(&parsed).unwrap();
        w.store.save_call_record(&judge).unwrap();
        let gate = gate(&w, &calls);
        assert_eq!(!blocks_gap_store(&gate), resolved, "{reply}: {gate:#?}");
    }
}

/// A deep path fills the opening words every gap on its file shares: a
/// disposed gap compares them without the path, an undisposed one as before.
#[test]
fn a_new_gap_opening_with_the_same_deep_path_is_new_only_once_disposed() {
    const DEEP: &str = "crates/shared/src/providers/lanes/canonical_store.rs";
    for (disposes, blocks) in [(true, false), (false, true)] {
        let w = world();
        std::fs::create_dir_all(w.root().join("crates/shared/src/providers/lanes")).unwrap();
        std::fs::write(w.root().join(DEEP), "//\n").unwrap();
        std::fs::write(
            w.root().join("tasks/TASK-A.md"),
            format!("`{DEEP}` and the ingest lane must stay consistent\n"),
        )
        .unwrap();
        let text =
            format!("{DEEP}:3 reads the timeframe segment, not the instrument, at every call site");
        let recorded = verdict(
            "verification-wave-review-verify-task-a-1-2",
            &["TASK-A"],
            &[("gap-deep", "high", &text)],
        );
        w.save(&recorded);
        let round = w.plan().rounds[0].clone();
        let mut calls = vec![recorded.call.clone(), slot()];
        let new = format!("{DEEP}:120 lacks a test");
        calls.extend(judged(
            &w,
            &round,
            &[("gap-deep-test", "medium", &new)],
            disposes.then(|| json!([{"gap_id": "gap-deep", "status": "resolved"}])),
        ));
        let first = gate(&w, &calls);
        let reopened = first.blocking.iter().any(|b| b.contains("`gap-deep`"));
        assert_eq!(reopened, blocks, "{disposes}: {first:#?}");
        // The same words on the same path are the same gap, disposed or not.
        let again = format!(
            "{DEEP}:88 reads the timeframe segment, not the instrument, at every call site still"
        );
        let later = verdict(
            "verification-wave-review-verify-task-a-1-40",
            &["TASK-A"],
            &[("gap-renamed", "low", &again)],
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
        w.save(&later);
        calls.push(later.call.clone());
        let second = gate(&w, &calls);
        assert!(
            second.blocking.iter().any(|b| b.contains("`gap-deep`")),
            "{disposes}: {second:#?}"
        );
    }
}

#[test]
fn a_disposition_resolves_nothing_when_two_of_the_rounds_gaps_share_its_id() {
    let w = world();
    for (recorder, text) in [
        (
            "verification-wave-review-verify-task-a-1-2",
            "the store lane reads the timeframe, crates/shared/src/store.rs:3",
        ),
        (
            "verification-wave-review-verify-task-a-1-3",
            "the store lane drops the venue, crates/shared/src/store.rs:9",
        ),
    ] {
        w.save(&verdict(
            recorder,
            &["TASK-A"],
            &[("gap-store", "high", text)],
        ));
    }
    let round = w.plan().rounds[0].clone();
    assert_eq!(round.residuals.len(), 2, "{round:?}");
    let mut calls = vec![
        "verification-wave-review-verify-task-a-1-2".into(),
        "verification-wave-review-verify-task-a-1-3".into(),
    ]
    .into_iter()
    .map(|id: String| w.store.load_call_record(&id).unwrap().unwrap().call)
    .collect::<Vec<_>>();
    calls.push(slot());
    calls.extend(judged(
        &w,
        &round,
        &[UNRELATED],
        Some(json!([{"gap_id": "gap-store", "status": "resolved"}])),
    ));
    assert!(blocks_gap_store(&gate(&w, &calls)));
}

#[test]
fn only_the_rounds_latest_verifier_disposes_and_a_refusing_verifier_still_reopens() {
    // An earlier verify attempt said resolved; the latest says nothing.
    let (w, round, mut calls) = planned();
    let tasks: Vec<&str> = round.tasks.iter().map(String::as_str).collect();
    let mut first = execution(&round, false, &[]).call;
    first.id = "verification-wave-review-verify-residual-8r0".into();
    let fix = execution(&round, true, &[]).call;
    w.save(&record(
        fix.clone(),
        WorkflowV2Status::Accepted,
        &tasks,
        &[],
    ));
    std::thread::sleep(std::time::Duration::from_millis(5));
    let mut early = record(first.clone(), WorkflowV2Status::Accepted, &tasks, &[]);
    early.result.data["outcomes"][0]["result"]["data"] =
        json!({ GAP_DISPOSITIONS_KEY: [{"gap_id": "gap-store", "status": "resolved"}] });
    w.save(&early);
    std::thread::sleep(std::time::Duration::from_millis(5));
    let mut last = execution(&round, false, &[]).call;
    last.id = "verification-wave-review-verify-residual-8".into();
    w.save(&record(
        last.clone(),
        WorkflowV2Status::Accepted,
        &tasks,
        &[UNRELATED],
    ));
    calls.extend([fix, first, last]);
    assert!(blocks_gap_store(&gate(&w, &calls)));
    // Disposed resolved, then a later verifier of the round's task that did
    // NOT accept records a weighty gap on the same file: it reopens.
    let (w, round, mut calls) = planned();
    calls.extend(judged(
        &w,
        &round,
        &[],
        Some(json!([{"gap_id": "gap-store", "status": "resolved"}])),
    ));
    assert!(!blocks_gap_store(&gate(&w, &calls)));
    std::thread::sleep(std::time::Duration::from_millis(5));
    let mut refusing = verdict(
        "verification-wave-review-verify-task-a-1-50",
        &["TASK-A"],
        &[UNRELATED],
    );
    refusing.status = WorkflowV2Status::NeedsReview;
    refusing.result.status = WorkflowV2Status::NeedsReview;
    w.save(&refusing);
    calls.push(refusing.call.clone());
    assert!(blocks_gap_store(&gate(&w, &calls)));
}

/// A disposed gap whose own words, paths dropped, are too few to compare
/// is compared as any other gap: a later accepted verifier re-recording it
/// word for word under a new id, even as a low note, reopens it; a new gap
/// on the same file does not.
#[test]
fn a_short_worded_disposed_gap_recorded_again_verbatim_under_a_new_id_stands() {
    const SHORT: &str = "crates/shared/src/store.rs:3 lacks a null check";
    for (again, blocks) in [
        (("gap-9", "low", SHORT), true),
        (
            (
                "gap-9",
                "low",
                "crates/shared/src/store.rs:120 lacks a test",
            ),
            false,
        ),
    ] {
        let w = world();
        let recorded = verdict(
            "verification-wave-review-verify-task-a-1-2",
            &["TASK-A"],
            &[("gap-null", "high", SHORT)],
        );
        w.save(&recorded);
        let round = w.plan().rounds[0].clone();
        let mut calls = vec![recorded.call.clone(), slot()];
        calls.extend(judged(
            &w,
            &round,
            &[UNRELATED],
            Some(json!([{"gap_id": "gap-null", "status": "resolved"}])),
        ));
        std::thread::sleep(std::time::Duration::from_millis(5));
        let later = verdict(
            "verification-wave-review-verify-task-a-1-60",
            &["TASK-A"],
            &[again],
        );
        w.save(&later);
        calls.push(later.call.clone());
        let gate = gate(&w, &calls);
        let reopened = gate.blocking.iter().any(|b| b.contains("`gap-null`"));
        assert_eq!(reopened, blocks, "{again:?}: {gate:#?}");
    }
}

use super::*;

pub(crate) fn fixed_attempt(project: &Path, run_id: &str, subject: &str) -> u64 {
    let store = archon_workflow::WorkflowStore::project(project);
    let state: serde_json::Value = serde_json::from_slice(
        &std::fs::read(store.run_dir(run_id).join("decomposition/state.json")).unwrap(),
    )
    .unwrap();
    state["attempts"][subject]["logical_attempt"]
        .as_u64()
        .expect("persisted logical attempt")
}

/// The interrupted author attempt was re-run on resume, not skipped.
///
/// `ControlPaused` records an interruption and does not advance the logical
/// attempt; resume restarts the same one. The observable is that the call which
/// was in flight at pause reaches a terminal accepted state afterwards.
///
/// This replaces an equality check against the attempt counter, which compared
/// a snapshot taken mid-flight to the value at the end of the run. That only
/// held while a phase could never take a second attempt -- true when observe
/// mode returned on first commit, false now that findings drive re-authoring.
/// A phase legitimately advancing because a gate reported a defect is the
/// repair loop working, not a resume that skipped an attempt.
pub(crate) fn assert_interrupted_attempt_resumed(
    project: &Path,
    run_id: &str,
    subject: &str,
    paused_attempt: u64,
) {
    let store = archon_workflow::WorkflowStore::project(project);
    let call_id = format!("{subject}-author-{paused_attempt}");
    let records = archon_workflow::v2::WorkflowV2ResultStore::new(store.run_dir(run_id).join("v2"))
        .load_call_records()
        .expect("call records");
    let record = records
        .iter()
        .find(|record| record.call.id == call_id)
        .unwrap_or_else(|| {
            panic!(
                "resume must re-run the interrupted attempt {call_id}; present: {:?}",
                records.iter().map(|r| &r.call.id).collect::<Vec<_>>()
            )
        });
    assert_eq!(
        record.status,
        archon_workflow::WorkflowV2Status::Accepted,
        "interrupted attempt {call_id} must complete on resume, not be abandoned"
    );
}

/// The implementation run finished with every task done and nothing blocked.
///
/// The terminal status is deliberately NOT pinned. `AC-SYN-001` can never be
/// satisfied -- the PRD requires the observer artifact to exist and forbids any
/// task from writing it -- but run-end unmet criteria are observe-only and do
/// not move the terminal status, so whether a run ends `Completed` or
/// `NeedsReview` depends on whether a reviewer happened to leave an unresolved
/// finding. Both were observed on healthy runs (wf-56746c18 needs_review,
/// wf-e2f1ab0c completed), so asserting either one alone fails half the time
/// for no defect.
///
/// What must always hold is asserted instead: a terminal status that is not a
/// failure, no blocking gap, and every write branch accepted. A failed branch,
/// a blocking gap, or a failed run still fails the proof -- none of which the
/// original single status equality caught.
pub(crate) fn assert_implementation_finished_clean(
    project: &Path,
    run_id: &str,
    terminal: archon_workflow::RunStatus,
) {
    assert!(
        matches!(
            terminal,
            archon_workflow::RunStatus::Completed | archon_workflow::RunStatus::NeedsReview
        ),
        "implementation run ended {terminal:?}; only completed or needs_review are healthy"
    );
    let store = archon_workflow::WorkflowStore::project(project);
    let events = parse_json_lines(&store.events_path(run_id)).unwrap();
    // Unresolved gaps, not historical ones. A stage can be gapped and then
    // succeed on retry -- observed on wf-d29889f3, where author-workflow-script
    // gapped at seq 3 on a malformed envelope and ended `accepted`. Failing on
    // the record rather than the outcome makes the proof a coin flip on model
    // formatting, which is what it must not be.
    // A call is recovered by a LATER accepted call-level completion: a
    // sibling branch's acceptance carries the same call_id and must not count,
    // and an acceptance that precedes the failure recovered nothing.
    let recovered_after = |call: &str, seq: u64| {
        events.iter().any(|event| {
            event["kind"] == "stage_completed"
                && event["detail"]["status"] == "accepted"
                && event["detail"]["event"] == "call_finished"
                && event["detail"]["call_id"] == call
                && event["seq"].as_u64().is_some_and(|later| later > seq)
        })
    };
    let unresolved: Vec<_> = events
        .iter()
        .filter(|event| event["kind"] == "blocking_gap_detected")
        .filter(|event| {
            let seq = event["seq"].as_u64().unwrap_or(0);
            event["detail"]["call_id"]
                .as_str()
                .is_none_or(|call| !recovered_after(call, seq))
        })
        .collect();
    assert!(
        unresolved.is_empty(),
        "a blocking gap was never resolved: {unresolved:?}"
    );
    let terminal_detail = events
        .iter()
        .rev()
        .find(|event| event["detail"]["branch_status_counts"].is_object())
        .map(|event| event["detail"].clone())
        .expect("terminal learning event carries branch status counts");
    let branches = terminal_detail["branch_status_counts"]
        .as_object()
        .expect("branch status counts");
    // `noop` is a healthy outcome: a typed no-op carrying task_coverage
    // evidence is exactly what `usable()` accepts, and a remediation round with
    // nothing to change reports one. A blocked branch is not. A failed branch
    // is judged by the run, not the count: the engine's answer to a failed
    // branch is to re-run its call, and a branch whose call was accepted
    // afterwards is recovered work, which is what this proof exists to show.
    let blocked = branches
        .get("blocked")
        .and_then(|count| count.as_u64())
        .unwrap_or(0);
    assert_eq!(blocked, 0, "no write branch may block: {branches:?}");
    // Read-only fanout branches announce a failure as a `stage_failed` event
    // carrying a branch_id; write branches record theirs only in the terminal
    // branch counts. So: every announced failure must be recovered by a later
    // accepted re-run of its call, and the terminal `failed` count must be
    // fully explained by those announcements. Anything left over is a write
    // branch that failed and was never re-run, which the proof refuses.
    let announced: Vec<_> = events
        .iter()
        .filter(|event| event["kind"] == "stage_failed" && event["detail"]["branch_id"].is_string())
        .collect();
    let unrecovered: Vec<_> = announced
        .iter()
        .filter(|event| {
            let seq = event["seq"].as_u64().unwrap_or(0);
            event["detail"]["call_id"]
                .as_str()
                .is_none_or(|call| !recovered_after(call, seq))
        })
        .map(|event| event["detail"].clone())
        .collect();
    assert!(
        unrecovered.is_empty(),
        "a failed branch was never recovered by an accepted re-run of its call: {unrecovered:?}"
    );
    let failed = branches
        .get("failed")
        .and_then(|count| count.as_u64())
        .unwrap_or(0);
    assert!(
        failed <= announced.len() as u64,
        "a write branch failed and was never re-run: counts {branches:?}, announced read-only failures {}",
        announced.len()
    );
}

/// The decomposition ends `NeedsReview` for the fixture's reason and no other,
/// and the repair loop demonstrably ran.
///
/// The proof previously asserted nothing about the decomposition's own terminal
/// state -- it only waited on the implementation run -- so a `needs_review`
/// caused by a real defect passed through unnoticed.
///
/// Three clauses, because a shape allowlist alone is not enough:
///
/// 1. The terminal status is `NeedsReview`. `AC-SYN-001` is mandated
///    unsatisfiable by the fixture, so the decomposition cannot end clean.
/// 2. The FINAL skeleton publication carries exactly one finding and no owner
///    finding. "PRD obligation has no skeleton owner" is repairable and belongs
///    to the mandated chain only on the first attempt; accepting it
///    unconditionally would pass a regressed repair loop that never cleared it.
///    Observed: attempt 1 publishes `findings=2` including the owner finding,
///    attempt 2 publishes `findings=1`.
/// 3. Each inherited echo names the exact predecessor counts -- acceptance
///    freeze 2, skeleton freeze 1. `[inherited_predecessor]` is a scope, not a
///    root cause, so an inherited finding from some new unrelated root would
///    otherwise sail through on the tag alone.
pub(crate) fn assert_decomposition_needs_review_for_the_fixture_only(
    project: &Path,
    run_id: &str,
    log_path: &Path,
) {
    let store = archon_workflow::WorkflowStore::project(project);
    let run = store.load_state(run_id).expect("decomposition run");
    assert_eq!(
        run.status,
        archon_workflow::RunStatus::NeedsReview,
        "the fixture mandates one unsatisfiable criterion, so the decomposition cannot end clean"
    );
    let text = std::fs::read_to_string(log_path).expect("decomposition log");

    const OWNER_FINDING: &str = "has no skeleton owner";
    let mut unexpected = Vec::new();
    for line in text.lines() {
        let Some(finding) = line
            .split(" text=")
            .nth(1)
            .filter(|_| line.contains(" finding="))
        else {
            continue;
        };
        let known = finding.contains("AC-SYN-001")
            || finding.contains(OWNER_FINDING)
            // Pinned counts, not the bare scope tag.
            || finding.contains("acceptance freeze carries 2 policy finding(s)")
            || finding.contains("acceptance freeze was minted in Observe mode with 2 policy finding(s)")
            || finding.contains("task skeleton freeze carries 1 policy finding(s)");
        if !known {
            unexpected.push(finding.to_string());
        }
    }
    assert!(
        unexpected.is_empty(),
        "decomposition needs_review outside the fixture's mandated chain: {unexpected:#?}"
    );

    // The repair actually cleared the owner finding.
    let publications: Vec<&str> = text
        .lines()
        .filter(|line| {
            line.contains("phase=skeleton")
                && !line.contains(" finding=")
                && line.contains("disposition=accepted_with_shadow_findings")
        })
        .collect();
    let last = publications
        .last()
        .expect("skeleton must publish at least once");
    assert!(
        last.contains("findings=1"),
        "the final skeleton publication must carry only the inherited finding, got: {last}"
    );
    let last_event = last
        .split_whitespace()
        .find_map(|field| field.strip_prefix("event_id="))
        .expect("publication line carries an event id");
    let owner_after_repair = text.lines().any(|line| {
        line.contains(&format!("event_id={last_event} ")) && line.contains(OWNER_FINDING)
    });
    assert!(
        !owner_after_repair,
        "the repair loop must clear the owner finding; it survived into the final skeleton publication"
    );
}

pub(crate) fn assert_acceptance_reused(project: &Path, run_id: &str) {
    let store = archon_workflow::WorkflowStore::project(project);
    let events = parse_json_lines(&store.events_path(run_id)).unwrap();
    assert!(events.iter().any(|event| {
        event["detail"]["phase"] == "acceptance" && event["detail"]["reused"] == true
    }));
}

pub(crate) fn assert_fixed_provider_route(project: &Path, run_id: &str) {
    let store = archon_workflow::WorkflowStore::project(project);
    let route: serde_json::Value = serde_json::from_slice(
        &std::fs::read(
            store
                .run_dir(run_id)
                .join("decomposition/provider-route.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(route["origin"], "trusted_config");
    assert!(route["endpoint_digest"].as_str().is_some());
}

pub(crate) fn assert_decomposition_events(project: &Path, run_id: &str) {
    let store = archon_workflow::WorkflowStore::project(project);
    let events = parse_json_lines(&store.events_path(run_id)).unwrap();
    for expected in [
        "author_attempt_started",
        "author_attempt_completed",
        "host_command_started",
        "host_command_completed",
        "decomposition_completed",
    ] {
        assert!(
            events.iter().any(|event| event["kind"] == expected),
            "missing {expected}: {events:#?}"
        );
    }
    let raw = std::fs::read_to_string(store.events_path(run_id)).unwrap();
    assert!(!raw.contains("candidate"));
    assert!(!raw.contains("ANTHROPIC"));
}

pub(crate) fn assert_frozen_floor(
    project: &Path,
    run_id: &str,
    expected: &archon_workflow::task_universe::WorkflowV2DeliverableContract,
) {
    let store = archon_workflow::WorkflowStore::project(project);
    let fixed: serde_json::Value = serde_json::from_slice(
        &std::fs::read(store.run_dir(run_id).join("decomposition/state.json")).unwrap(),
    )
    .unwrap();
    let task_root = PathBuf::from(fixed["identity"]["task_root_identity"].as_str().unwrap());
    let contract: archon_workflow::task_set_contract::AcceptanceContract =
        serde_json::from_slice(&std::fs::read(task_root.join("acceptance-contract.json")).unwrap())
            .unwrap();
    let criterion = contract
        .acceptance
        .iter()
        .find(|criterion| criterion.id == "AC-SYN-001")
        .unwrap();
    match &criterion.check {
        archon_workflow::task_set_contract::AcceptanceCheck::Floor { contract } => {
            assert_eq!(contract, expected);
            assert!(contract.typed_verifier_command.is_none());
        }
        archon_workflow::task_set_contract::AcceptanceCheck::Command { .. } => {
            panic!("synthetic proof refuses command-bearing frozen acceptance")
        }
    }
}

pub(crate) fn assert_synthetic_outputs(project: &Path) {
    assert_eq!(
        std::fs::read_to_string(project.join("src/alpha.txt"))
            .unwrap()
            .trim(),
        "alpha ready"
    );
    let beta: serde_json::Value =
        serde_json::from_slice(&std::fs::read(project.join("src/beta.json")).unwrap()).unwrap();
    assert_eq!(beta["ready"], true);
    assert_eq!(beta.as_object().map(serde_json::Map::len), Some(1));
    assert!(
        !project
            .join(".archon/proof/synthetic-observer-target.json")
            .exists()
    );
}

/// The sequence number of the run's terminal event.
///
/// The terminal marker is `detail.event == "terminal_status"`, which every
/// finalizer path stamps. It is NOT the event `kind`: the finalizer encodes the
/// outcome there (`stage_completed`, `stage_stalled`, `stage_failed`), and
/// `stage_completed` is also what every finished branch emits — so selecting
/// by kind either matches nothing (`completed`, the previous filter, which no
/// completing run emits) or matches a branch event that precedes the observer
/// on every run and proves nothing.
pub(crate) fn terminal_event_seq(events: &[serde_json::Value]) -> u64 {
    events
        .iter()
        .filter(|event| event["detail"]["event"] == "terminal_status")
        .filter_map(|event| event["seq"].as_u64())
        .max()
        .expect("terminal event carrying detail.event == \"terminal_status\"")
}

pub(crate) fn assert_observer_after_terminal(project: &Path, run_id: &str) {
    let store = archon_workflow::WorkflowStore::project(project);
    let events = parse_json_lines(&store.events_path(run_id)).unwrap();
    let terminal_seq = terminal_event_seq(&events);
    let observer_seq = events
        .iter()
        .filter(|event| {
            matches!(
                event["kind"].as_str(),
                Some("run_end_acceptance_observer_started")
                    | Some("run_end_acceptance_shadow_observed")
            )
        })
        .filter_map(|event| event["seq"].as_u64())
        .min()
        .expect("observer event");
    assert!(terminal_seq < observer_seq);
    let finalization: archon_workflow::FinalizationRecordV1 = serde_json::from_slice(
        &std::fs::read(store.run_dir(run_id).join("v2/finalization.json")).unwrap(),
    )
    .unwrap();
    assert!(finalization.terminal_state_committed);
    assert!(finalization.terminal_event_committed);
    match finalization.observer_state {
        Some(archon_workflow::RunEndObserverStateV1::Completed { outcome }) => {
            assert_eq!(
                outcome.authority,
                archon_workflow::ObserverAuthority::ObserveOnly
            );
            assert!(outcome.evaluated_floor_count >= 1);
            assert!(outcome.policy_finding_count >= 1);
        }
        other => panic!("observer did not complete with shadow findings: {other:?}"),
    }
    let records = store
        .run_dir(run_id)
        .join("observer/run-end-acceptance.jsonl");
    let text = std::fs::read_to_string(records).unwrap();
    assert!(text.contains("AC-SYN-001"));
    assert!(text.contains("observe_only"));
}

/// Driven by the events run 22 actually recorded, so the assertion is proven
/// against the shape the engine emits rather than the shape it was assumed to
/// emit. Run 22 was the first run ever to reach `assert_observer_after_terminal`
/// -- every earlier run died before it -- and it panicked on a `kind` no run
/// emits, after the property it exists to prove had held.
#[cfg(test)]
mod recorded_run_tests {
    use super::*;

    const FIXTURE: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/decomposition-synthetic/recorded-run-22"
    );

    fn recorded_events() -> Vec<serde_json::Value> {
        parse_json_lines(&Path::new(FIXTURE).join("events.jsonl")).unwrap()
    }

    /// Lay the recorded run out under a scratch project exactly where the
    /// store expects it.
    fn recorded_project(run_id: &str) -> tempfile::TempDir {
        let temp = tempfile::tempdir().unwrap();
        let store = archon_workflow::WorkflowStore::project(temp.path());
        let run_dir = store.run_dir(run_id);
        std::fs::create_dir_all(run_dir.join("v2")).unwrap();
        std::fs::create_dir_all(run_dir.join("observer")).unwrap();
        let fixture = Path::new(FIXTURE);
        std::fs::copy(fixture.join("events.jsonl"), store.events_path(run_id)).unwrap();
        std::fs::copy(
            fixture.join("finalization.json"),
            run_dir.join("v2/finalization.json"),
        )
        .unwrap();
        std::fs::copy(
            fixture.join("run-end-acceptance.jsonl"),
            run_dir.join("observer/run-end-acceptance.jsonl"),
        )
        .unwrap();
        temp
    }

    /// A needs_review run's terminal event has kind `stage_stalled`; a branch
    /// that finished earlier has kind `stage_completed`. Only the marker
    /// selects the right one.
    #[test]
    fn terminal_event_is_selected_by_its_marker_not_its_kind() {
        let events = recorded_events();
        let kinds: Vec<_> = events
            .iter()
            .map(|event| event["kind"].as_str().unwrap().to_string())
            .collect();
        assert!(
            kinds.contains(&"stage_completed".to_string()),
            "fixture must carry a finished branch event: {kinds:?}"
        );
        assert!(
            !kinds.contains(&"completed".to_string()),
            "no run emits kind `completed`; the fixture must not either: {kinds:?}"
        );

        assert_eq!(terminal_event_seq(&events), 77);
    }

    /// The whole post-terminal check, against the recorded run: ordering,
    /// finalization, ObserveOnly authority, evaluated floor, and the observer
    /// record naming the criterion.
    #[test]
    fn recorded_run_passes_the_observer_assertion() {
        let run_id = "wf-bb44a82c-3fce-4acc-9407-910ff5457358";
        let project = recorded_project(run_id);
        assert_observer_after_terminal(project.path(), run_id);
    }
}

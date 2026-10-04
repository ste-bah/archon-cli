//! The acceptance loop follows progress, never a round count (A2).

use super::super::{
    ACCEPTANCE_MAX_ROUNDS, ACCEPTANCE_ROUND_RECORD_SCHEMA_VERSION, AcceptanceCheckRecordV1,
    AcceptanceCheckStatus, write_round_record,
};
use super::*;

fn check(id: &str, status: AcceptanceCheckStatus, stderr: &str) -> AcceptanceCheckRecordV1 {
    AcceptanceCheckRecordV1 {
        check_id: id.into(),
        criterion: format!("criterion {id}"),
        kind: "command".into(),
        status,
        exit_code: Some(i32::from(status != AcceptanceCheckStatus::Passed)),
        operational_error: None,
        owning_tasks: vec!["TASK-A".into()],
        stdout_tail: String::new(),
        stderr_tail: stderr.into(),
        regressed_by: None,
        contract_defect: false,
        routing: None,
        regression_search: None,
        blocked: None,
    }
}

fn round(n: u32, checks: Vec<AcceptanceCheckRecordV1>) -> AcceptanceRoundRecordV1 {
    AcceptanceRoundRecordV1 {
        schema_version: ACCEPTANCE_ROUND_RECORD_SCHEMA_VERSION,
        run_id: "wf-progress".into(),
        call_id: format!("acceptance-contract-run-{n}"),
        round: n,
        attempt: 1,
        max_rounds: ACCEPTANCE_MAX_ROUNDS,
        contract_present: true,
        requested_check_ids: Vec::new(),
        execution: None,
        checks,
        operational_errors: Vec::new(),
        contract_repairs: Vec::new(),
        final_round: false,
    }
}

fn failed(id: &str, why: &str) -> AcceptanceCheckRecordV1 {
    check(id, AcceptanceCheckStatus::Failed, why)
}

#[test]
fn the_loop_runs_past_the_old_round_count_while_the_failing_set_shrinks() {
    let mut rounds = vec![failing(0, 1)];
    rounds.extend(
        (1..=5)
            .map(|n| {
                let checks = (n..=5)
                    .map(|id| failed(&format!("AC-{id}"), "assertion failed"))
                    .collect();
                round(n, checks)
            })
            .collect::<Vec<AcceptanceRoundRecordV1>>(),
    );
    for n in 1..rounds.len() {
        let decision = decide(&rounds[..n], &rounds[n]);
        assert!(!decision.final_round, "round {} shrank the set", n + 1);
        assert!(!decision.escalate);
        assert_eq!(decision.stalled_rounds, 0);
        assert_eq!(decision.pause, None);
    }
}

#[test]
fn a_stalled_round_escalates_and_a_second_one_pauses_the_run() {
    let same = || round(1, vec![failed("AC-1", "assertion failed: took 12ms")]);
    let first = same();
    let mut second = same();
    // Digits and scratch paths are not new evidence.
    second.checks[0].stderr_tail = "assertion failed: took 97ms".into();
    let decision = decide(std::slice::from_ref(&first), &second);
    assert!(!decision.final_round);
    assert!(decision.escalate);
    assert_eq!(decision.stalled_rounds, 1);
    let third = same();
    let decision = decide(&[first.clone(), second.clone()], &third);
    // Issue 262: a stall pauses with evidence; it never ends the loop.
    assert!(!decision.final_round);
    assert_eq!(decision.pause, Some(PAUSE_NO_PROGRESS));
    assert_eq!(decision.stalled_rounds, 2);
    assert!(third.blocks_completion());
}

#[test]
fn only_a_new_failing_set_is_progress_never_changed_output_text() {
    let a = round(1, vec![failed("AC-1", "missing field x")]);
    // The same check failing with other words is no progress (Batch O
    // review: output text that differs every round funded the loop for ever).
    let b = round(2, vec![failed("AC-1", "missing field y")]);
    assert!(!made_progress(std::slice::from_ref(&a), &b));
    // A different failing check is a state never seen: progress.
    let c = round(3, vec![failed("AC-2", "missing field x")]);
    assert!(made_progress(&[a.clone(), b.clone()], &c));
    // Back to the first state: an oscillation, not progress.
    let d = round(4, vec![failed("AC-1", "missing field x")]);
    assert!(!made_progress(&[a, b, c], &d));
}

fn failing(n: u32, count: u32) -> AcceptanceRoundRecordV1 {
    round(
        n,
        (0..count)
            .map(|id| failed(&format!("AC-{id}"), "x"))
            .collect(),
    )
}

/// Issue 262: no round count ends or pauses the loop. Every round here
/// reaches a failing set never seen, so round 64 and past never pause.
#[test]
fn a_loop_that_keeps_reaching_new_failing_sets_never_pauses() {
    let mut history = vec![failing(0, 1)];
    history.extend((1..=74).map(|n| failing(n, 200 - n)));
    for n in 1..history.len() {
        let decision = decide(&history[..n], &history[n]);
        assert_eq!(decision.pause, None, "round {} reached a new set", n + 1);
        assert!(!decision.final_round);
        assert_eq!(decision.stalled_rounds, 0);
    }
}

/// Round 3 (decision A): progress is a failing set this run never reached,
/// never beating the all-time minimum. After a regression from 1 failing
/// check to 100, every round that repairs one more reaches a new set: no
/// pause, at round 65 or ever.
#[test]
fn recovery_after_a_regression_is_progress_on_every_new_failing_set() {
    let mut history = vec![round(1, vec![failed("AC-ONLY", "x")])];
    for (n, count) in (2..).zip((37..=100).rev()) {
        let next = failing(n, count);
        let decision = decide(&history, &next);
        assert_eq!(decision.pause, None, "round {n} repaired a check");
        assert_eq!(decision.stalled_rounds, 0, "round {n}");
        history.push(next);
    }
}

/// A failing set reached before counts toward the stall, whatever its size;
/// a set never reached resets the count.
#[test]
fn a_revisited_failing_set_counts_toward_the_stall_and_a_new_one_resets_it() {
    let set = |n: u32, ids: &[&str]| round(n, ids.iter().map(|id| failed(id, "x")).collect());
    let a = set(1, &["AC-1"]);
    let b = set(2, &["AC-2"]);
    let again_a = set(3, &["AC-1"]);
    let c = set(4, &["AC-3"]);
    let history = vec![a.clone(), b.clone()];
    let decision = decide(&history, &again_a);
    assert_eq!((decision.stalled_rounds, decision.escalate), (1, true));
    let history = vec![a.clone(), b.clone(), again_a.clone()];
    assert_eq!(decide(&history, &c).stalled_rounds, 0, "a new set resets");
    let history = vec![a.clone(), b.clone(), again_a.clone(), c.clone()];
    assert_eq!(decide(&history, &set(5, &["AC-1"])).stalled_rounds, 1);
    let history = vec![a, b, again_a, c, set(5, &["AC-1"])];
    let decision = decide(&history, &set(6, &["AC-2"]));
    assert_eq!(decision.stalled_rounds, 2);
    assert_eq!(decision.pause, Some(PAUSE_NO_PROGRESS));
}

#[test]
fn a_clean_round_is_final_and_host_repairable_errors_keep_the_loop_going() {
    let clean = round(1, vec![check("AC-1", AcceptanceCheckStatus::Passed, "")]);
    assert!(decide(&[], &clean).final_round);
    // An erroring check is the host's to repair: never final on round 1.
    let mut errored = check("AC-1", AcceptanceCheckStatus::Error, "");
    errored.owning_tasks.clear();
    errored.operational_error = Some("native observation guardian failed".into());
    let errored = round(1, vec![errored]);
    let decision = decide(&[], &errored);
    assert!(!decision.final_round);
    // So is a round-level error.
    let mut site = round(1, Vec::new());
    site.operational_errors.push("scratch site failed".into());
    assert!(!decide(&[], &site).final_round);
}

#[test]
fn nothing_passes_without_a_contract_or_with_no_check_run() {
    let mut absent = round(1, Vec::new());
    absent.contract_present = false;
    assert!(absent.blocks_completion());
    let empty = round(1, Vec::new());
    assert!(empty.blocks_completion());
}

#[test]
fn earlier_rounds_read_the_latest_attempt_of_each_round() {
    let dir = tempfile::tempdir().unwrap();
    let mut one = round(1, vec![failed("AC-1", "x")]);
    write_round_record(dir.path(), &one).unwrap();
    one.attempt = 2;
    one.checks.push(failed("AC-2", "y"));
    write_round_record(dir.path(), &one).unwrap();
    write_round_record(dir.path(), &round(2, Vec::new())).unwrap();
    let earlier = earlier_rounds(dir.path(), 3);
    assert_eq!(earlier.len(), 2);
    assert_eq!(earlier[0].attempt, 2);
    assert_eq!(earlier[0].checks.len(), 2);
    assert!(earlier_rounds(dir.path(), 1).is_empty());
}

/// Round 3 (decision A): the states reached and the revisit count are
/// persisted, so a resumed round keeps them; a run with no ledger yet gets
/// the one its round records leave.
#[test]
fn the_ledger_is_persisted_and_rebuilt_from_records_when_absent() {
    let dir = tempfile::tempdir().unwrap();
    let a = round(1, vec![failed("AC-1", "x")]);
    let b = round(2, vec![failed("AC-2", "x")]);
    write_round_record(dir.path(), &a).unwrap();
    write_round_record(dir.path(), &b).unwrap();
    let rebuilt = ProgressLedger::load(dir.path(), 3).unwrap();
    assert_eq!(
        rebuilt,
        ProgressLedger::from_history(&[a.clone(), b.clone()])
    );
    let mut ledger = rebuilt;
    assert_eq!(ledger.observe(&round(3, vec![failed("AC-1", "y")])), 1);
    write_round_record(dir.path(), &round(3, vec![failed("AC-1", "y")])).unwrap();
    ledger.save(dir.path()).unwrap();
    let resumed = ProgressLedger::load(dir.path(), 3).unwrap();
    assert_eq!(resumed.revisits, 1);
    assert_eq!(resumed.seen.len(), 2);
}

#[test]
fn rebuild_keeps_all_attempts_including_the_resumed_round_and_records_win() {
    let dir = tempfile::tempdir().unwrap();
    let a = round(1, vec![failed("A", "x")]);
    let mut b = round(1, vec![failed("B", "x")]);
    b.attempt = 2;
    let c = round(2, vec![failed("A", "x")]);
    for record in [&a, &b, &c] {
        write_round_record(dir.path(), record).unwrap();
    }
    let expected = ProgressLedger::from_history(&[a.clone(), b, c.clone()]);
    assert_eq!(ProgressLedger::load(dir.path(), 2).unwrap(), expected);
    ProgressLedger::from_history(&[a]).save(dir.path()).unwrap();
    let mut ledger = ProgressLedger::load(dir.path(), 2).unwrap();
    assert_eq!(
        ledger, expected,
        "a readable stale ledger cannot hide records"
    );
    assert_eq!(decide_with(&mut ledger, &c).pause, Some(PAUSE_NO_PROGRESS));
}

/// Round 5: a resume restarts the loop at round 1, so attempts are read in
/// the order they were recorded, never by (round, attempt). R1{X}, R2{X},
/// then after a resume R1#2{Y}, R2#2{Y}: one revisit in a row, not two.
#[test]
fn the_ledger_is_rebuilt_in_recording_order_across_a_resume() {
    let dir = tempfile::tempdir().unwrap();
    let x = |n: u32, attempt: u32| {
        let mut record = round(n, vec![failed("AC-X", "x")]);
        record.attempt = attempt;
        record
    };
    let y = |n: u32, attempt: u32| {
        let mut record = round(n, vec![failed("AC-Y", "y")]);
        record.attempt = attempt;
        record
    };
    for record in [x(1, 1), x(2, 1), y(1, 2), y(2, 2)] {
        write_round_record(dir.path(), &record).unwrap();
    }
    assert_eq!(ProgressLedger::load(dir.path(), 3).unwrap().revisits, 1);
}

/// Round 6: a record whose order entry is lost is placed by its own write
/// time, never first: R1{X}, R2{X}, R1#2{Y} (entry lost), R2#2{Y} rebuilds
/// as X, X, Y, Y -- one revisit in a row.
#[test]
fn an_unlogged_record_is_placed_by_its_own_write_time() {
    let dir = tempfile::tempdir().unwrap();
    let set = |n: u32, attempt: u32, id: &str| {
        let mut record = round(n, vec![failed(id, "x")]);
        record.attempt = attempt;
        record
    };
    for record in [
        set(1, 1, "AC-X"),
        set(2, 1, "AC-X"),
        set(1, 2, "AC-Y"),
        set(2, 2, "AC-Y"),
    ] {
        std::thread::sleep(std::time::Duration::from_millis(5));
        write_round_record(dir.path(), &record).unwrap();
    }
    let log = dir
        .path()
        .join(ACCEPTANCE_RECORDS_DIR)
        .join("recording-order.log");
    let kept: Vec<String> = std::fs::read_to_string(&log)
        .unwrap()
        .lines()
        .filter(|line| !names_entry(line, 1, 2))
        .map(str::to_string)
        .collect();
    std::fs::write(&log, kept.join("\n") + "\n").unwrap();
    assert_eq!(ProgressLedger::load(dir.path(), 3).unwrap().revisits, 1);
}

/// Whether a recording-order line is the entry of (`round`, `attempt`), in
/// any format a writer of the log used: `round attempt`, `round attempt
/// nanos`, or a sequenced `seq=<n> round attempt`.
fn names_entry(line: &str, round: u32, attempt: u32) -> bool {
    let fields: Vec<&str> = (line.split_whitespace())
        .filter(|field| !field.starts_with("seq="))
        .collect();
    fields.get(..2) == Some(&[round.to_string().as_str(), attempt.to_string().as_str()][..])
}

pub(super) fn set(n: u32, attempt: u32, id: &str) -> AcceptanceRoundRecordV1 {
    let mut record = round(n, vec![failed(id, "x")]);
    record.attempt = attempt;
    record
}

pub(super) fn order_log(run_dir: &Path) -> std::path::PathBuf {
    run_dir
        .join(ACCEPTANCE_RECORDS_DIR)
        .join("recording-order.log")
}

/// Gives the record of (`round`, `attempt`) the file time `secs` after the
/// epoch.
fn set_file_time(run_dir: &Path, round: u32, attempt: u32, secs: u64) {
    let path = round_dir(run_dir, round).join(attempt_file_name(attempt));
    let file = std::fs::File::options().write(true).open(path).unwrap();
    let at = std::time::UNIX_EPOCH + std::time::Duration::from_secs(secs);
    file.set_modified(at).unwrap();
}

/// R1{X}, R2{X}, then after a resume R1#2{Y}, written by the real writer
/// with file times `secs`; `log`, when given, replaces the order log.
pub(super) fn x_x_y(secs: [u64; 3], log: Option<&str>) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let records = [set(1, 1, "AC-X"), set(2, 1, "AC-X"), set(1, 2, "AC-Y")];
    for (record, secs) in records.iter().zip(secs) {
        write_round_record(dir.path(), record).unwrap();
        set_file_time(dir.path(), record.round, record.attempt, secs);
    }
    if let Some(log) = log {
        std::fs::write(order_log(dir.path()), log).unwrap();
    }
    dir
}

/// X, X, Y ends on a new state (no revisit), so the next Y is the first
/// revisit: it escalates and never pauses.
pub(super) fn assert_replayed_x_x_y(run_dir: &Path) {
    let mut ledger = ProgressLedger::load(run_dir, 2).unwrap();
    assert_eq!(ledger.revisits, 0, "X, X, Y ends on a state never reached");
    let decision = decide_with(&mut ledger, &set(2, 2, "AC-Y"));
    assert_eq!(
        (decision.stalled_rounds, decision.escalate, decision.pause),
        (1, true, None),
        "the next Y is the first revisit"
    );
}

/// Round 7: the log's own order is authoritative. A clock stepped back
/// between the writes (later entries carry earlier times, and so do the
/// record files) never reorders X, X, Y as Y, X, X.
#[test]
fn a_clock_stepped_back_never_reorders_logged_records() {
    let log = "1 1 3000000000000\n2 1 2000000000000\n1 2 1000000000000\n";
    assert_replayed_x_x_y(x_x_y([3000, 2000, 1000], Some(log)).path());
}

/// Round 7: records the writer itself logged keep the log's order whatever
/// their file times say.
#[test]
fn logged_records_keep_the_log_order_over_backward_file_times() {
    assert_replayed_x_x_y(x_x_y([3000, 2000, 1000], None).path());
}

/// Round 7: equal times (a coarse clock, or a 2 s FAT write time) are no
/// tie to break by (round, attempt): the log's order decides.
#[test]
fn equal_timestamps_replay_in_log_order() {
    let log = "1 1 5000000000000\n2 1 5000000000000\n1 2 5000000000000\n";
    assert_replayed_x_x_y(x_x_y([5000; 3], Some(log)).path());
}

/// Round 7: a log written in the earlier two-column format (`round
/// attempt`, no time) is read in its own order, never ignored.
#[test]
fn legacy_two_column_entries_replay_in_log_order() {
    let log = "1 1\n2 1\n1 2\n";
    assert_replayed_x_x_y(x_x_y([3000, 2000, 1000], Some(log)).path());
}

/// Round 7: a run whose log began in an earlier format and goes on in this
/// one keeps one order: the new entries follow the old ones. A torn last
/// line never swallows the entry appended after it.
#[test]
fn a_legacy_or_torn_log_continued_by_this_writer_keeps_its_order() {
    for legacy in ["1 1\n2 1 7000\n", "1 1\n2 1"] {
        let dir = tempfile::tempdir().unwrap();
        for (record, secs) in [(set(1, 1, "AC-X"), 3000), (set(2, 1, "AC-X"), 2000)] {
            write_round_record(dir.path(), &record).unwrap();
            set_file_time(dir.path(), record.round, record.attempt, secs);
        }
        std::fs::write(order_log(dir.path()), legacy).unwrap();
        write_round_record(dir.path(), &set(1, 2, "AC-Y")).unwrap();
        set_file_time(dir.path(), 1, 2, 1000);
        assert_replayed_x_x_y(dir.path());
    }
}

/// Round 7, healed in round 8: a record replay cannot parse is never
/// skipped silently. It is quarantined and reported by name (here with no
/// ledger copy, so its state is unknown and the caller pauses).
#[test]
fn an_unparsable_record_is_quarantined_and_reported_never_skipped() {
    for broken in [
        "{\"schema_version\": 1, \"round\":",
        "{\"schema_version\": 1, \"run_id\": \"r\", \"call_id\": \"c\", \"round\": \"one\", \"attempt\": 1, \"max_rounds\": 3, \"contract_present\": true, \"final_round\": false}",
    ] {
        let dir = x_x_y([1000, 2000, 3000], None);
        let path = round_dir(dir.path(), 2).join(attempt_file_name(1));
        std::fs::write(&path, broken).unwrap();
        let healed = ProgressLedger::load_healing(dir.path()).unwrap();
        let [reported] = &healed.unknown()[..] else {
            panic!("reported once: {:?}", healed.quarantined);
        };
        assert_eq!((reported.round, reported.attempt), (2, 1));
        assert!(
            reported.original.ends_with("round-02/attempt-01.json"),
            "{reported:?}"
        );
        assert!(!path.exists());
    }
}

/// Round 7: a record lands whole or not at all. The write leaves nothing
/// but the record, and a staging file a crash left behind is no record:
/// replay, the next attempt number and the latest record all ignore it.
#[test]
fn a_record_lands_whole_and_a_crashed_staging_file_is_no_record() {
    let dir = tempfile::tempdir().unwrap();
    write_round_record(dir.path(), &set(1, 1, "AC-X")).unwrap();
    let round_one = round_dir(dir.path(), 1);
    let names = |dir: &Path| -> Vec<String> {
        let mut names: Vec<String> = (std::fs::read_dir(dir).unwrap())
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    };
    assert_eq!(names(&round_one), vec![attempt_file_name(1)]);
    std::fs::write(
        round_one.join(".attempt-02.json.crashed.tmp"),
        "{\"round\":",
    )
    .unwrap();
    assert_eq!(next_attempt(dir.path(), 1), 2);
    let ledger = ProgressLedger::load(dir.path(), 2).unwrap();
    assert_eq!(ledger.seen.len(), 1);
    let (latest, _) = super::super::latest_round_record(dir.path())
        .unwrap()
        .unwrap();
    assert_eq!((latest.round, latest.attempt), (1, 1));
}

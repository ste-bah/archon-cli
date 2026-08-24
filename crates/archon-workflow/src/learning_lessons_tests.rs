//! The curated stream's invariants: prose that can never carry a path, a
//! rendered block that can never grow without bound, and rules that both fire
//! and stay silent on the shapes a real run produces.
//!
//! Every rule here was checked against a live run's call records before it was
//! written. The previous rule set was not, and was a constant rather than a
//! signal: it keyed on `StageState::artifacts`, which has no production writer.

use super::*;
use crate::spec::WorkflowSpec;
use crate::v2::host_api::{WorkflowV2HostCall, WorkflowV2HostMethod, WorkflowV2WriteMode};
use crate::v2::result::{
    WorkflowV2CommandKind, WorkflowV2CommandRecord, WorkflowV2CommandStatus, WorkflowV2Evidence,
    WorkflowV2EvidenceKind, WorkflowV2FileRecord, WorkflowV2ResidualGap, WorkflowV2Result,
    WorkflowV2TaskCoverage, WorkflowV2TaskCoverageStatus,
};
use crate::v2::result_store::{WorkflowV2SourceTaskGraph, WorkflowV2SourceTaskItem};

fn run(status: RunStatus) -> WorkflowRun {
    let spec = WorkflowSpec::from_yaml(
        "schema: archon.workflow.v1\nname: lesson-test\ntask: Lesson test\nstages:\n  - id: a\n    kind: agent\n",
    )
    .unwrap();
    let mut run = WorkflowRun::new(spec, std::path::Path::new("/tmp/lessons-test"));
    run.status = status;
    run
}

/// A call record shaped like the ones a live run writes.
///
/// `write` mirrors the host's own `write_mode`: `true` is an `implement()`
/// call, `false` is an `agent()`/`parallel()` verification.
fn call(id: &str, write: bool, result: WorkflowV2Result) -> WorkflowV2CallRecord {
    WorkflowV2CallRecord::new(
        "wf-test",
        WorkflowV2HostCall {
            id: id.to_string(),
            method: if write {
                WorkflowV2HostMethod::Fanout
            } else {
                WorkflowV2HostMethod::Parallel
            },
            write_mode: write.then_some(WorkflowV2WriteMode::Worktree),
            options: Default::default(),
        },
        1,
        format!("hash-{id}"),
        result,
        Vec::new(),
    )
}

fn result(status: WorkflowV2Status) -> WorkflowV2Result {
    WorkflowV2Result {
        status,
        summary: "summary".to_string(),
        ..WorkflowV2Result::default()
    }
}

fn file(path: &str) -> WorkflowV2FileRecord {
    WorkflowV2FileRecord::new(path)
}

fn evidence() -> WorkflowV2Evidence {
    WorkflowV2Evidence::new(WorkflowV2EvidenceKind::Test, "a test passed")
}

fn command() -> WorkflowV2CommandRecord {
    WorkflowV2CommandRecord {
        kind: WorkflowV2CommandKind::Test,
        command: "run the focused test".to_string(),
        status: WorkflowV2CommandStatus::Succeeded,
        exit_code: Some(0),
        output_summary: "1 passed".to_string(),
    }
}

fn coverage(task: &str) -> WorkflowV2TaskCoverage {
    WorkflowV2TaskCoverage {
        task_id: task.to_string(),
        status: WorkflowV2TaskCoverageStatus::Accepted,
        summary: "covered".to_string(),
        evidence: vec![evidence()],
    }
}

fn gap(id: &str) -> WorkflowV2ResidualGap {
    WorkflowV2ResidualGap {
        id: id.to_string(),
        description: "a finding nobody resolved".to_string(),
        severity: None,
    }
}

/// Attach a source task graph, which is how a call names the tasks it concerns.
fn with_tasks(
    mut record: WorkflowV2CallRecord,
    universe: &[&str],
    ids: &[&str],
) -> WorkflowV2CallRecord {
    record.source_task_graph = Some(WorkflowV2SourceTaskGraph {
        schema_version: "workflow-v2-source-task-graph-v1".to_string(),
        canonical_task_universe: universe.iter().map(|id| id.to_string()).collect(),
        items: vec![WorkflowV2SourceTaskItem {
            item_id: format!("{}-item", record.call.id),
            canonical_task_ids: ids.iter().map(|id| id.to_string()).collect(),
            dependency_ids: Vec::new(),
            target_files: Vec::new(),
            declared_target_files: Vec::new(),
            target_file_expansions: Vec::new(),
            acceptance_criteria: Vec::new(),
            focused_verification: Vec::new(),
            expected_evidence: Vec::new(),
            artifact_requirements: Vec::new(),
            required_tools: Vec::new(),
        }],
        completed_ids: Vec::new(),
    });
    record
}

#[test]
fn rendered_prose_carries_no_path_or_identifier() {
    assert!(lessons_are_path_free());
    for rule in LessonRule::ALL {
        let text = format!("{} {}", rule.headline(), rule.guidance());
        assert!(!text.contains('/'), "{rule:?} prose has a path");
        assert!(!text.contains(".rs"), "{rule:?} prose names a source file");
        // Numbers belong in `LessonEvidence`, never in prose: lessons merge
        // across runs and a baked-in count would go stale on the first merge.
        assert!(
            !text.chars().any(|c| c.is_ascii_digit()),
            "{rule:?} bakes a count into its prose"
        );
    }
}

#[test]
fn a_write_call_accepted_having_done_nothing_is_the_silent_failure() {
    let records = vec![call("implement", true, result(WorkflowV2Status::Accepted))];
    let lessons = distil_lessons(&run(RunStatus::Completed), &records);
    let silent = lessons
        .iter()
        .find(|lesson| lesson.rule == LessonRule::SilentImplementation)
        .expect("accepted write call with no files and no commands must distil a lesson");
    assert_eq!(silent.evidence.occurrences, 1);
    assert_eq!(silent.evidence.calls, 1);
    assert_eq!(silent.scope(), "implementation");
}

#[test]
fn a_write_call_that_verified_rather_than_edited_is_not_silent() {
    // The live shape this exists to protect: a remediation that inspected the
    // baseline, ran thirteen commands, and correctly changed nothing. Calling
    // that a silent failure would teach the next run to make pointless edits.
    let mut accepted = result(WorkflowV2Status::Accepted);
    accepted.commands_run.push(command());
    accepted.task_coverage.push(coverage("TASK-A"));
    accepted.evidence.push(evidence());
    let lessons = distil_lessons(
        &run(RunStatus::Completed),
        &[call("remediate", true, accepted)],
    );
    assert!(
        !lessons
            .iter()
            .any(|lesson| lesson.rule == LessonRule::SilentImplementation),
        "an evidenced no-op must not be filed as a silent failure: {lessons:?}"
    );
}

#[test]
fn a_verification_that_wrote_nothing_is_never_a_silent_implementation() {
    // Verifications are not write-capable and legitimately change nothing.
    let lessons = distil_lessons(
        &run(RunStatus::Completed),
        &[call("verify", false, result(WorkflowV2Status::Accepted))],
    );
    assert!(
        !lessons
            .iter()
            .any(|lesson| lesson.rule == LessonRule::SilentImplementation),
        "a non-write call must never fire the write rule: {lessons:?}"
    );
}

#[test]
fn a_noop_without_proof_is_reported_and_one_with_proof_is_not() {
    let bare = call("bare", true, result(WorkflowV2Status::Noop));
    let lessons = distil_lessons(&run(RunStatus::Completed), &[bare]);
    assert_eq!(
        lessons
            .iter()
            .filter(|lesson| lesson.rule == LessonRule::UnprovenNoop)
            .count(),
        1
    );

    let mut proven = result(WorkflowV2Status::Noop);
    proven.task_coverage.push(coverage("TASK-A"));
    proven.evidence.push(evidence());
    let lessons = distil_lessons(&run(RunStatus::Completed), &[call("proven", true, proven)]);
    assert!(
        !lessons
            .iter()
            .any(|lesson| lesson.rule == LessonRule::UnprovenNoop),
        "a no-op carrying coverage and evidence is exactly what the brief asks for"
    );
}

#[test]
fn accepting_work_that_still_carries_findings_is_reported() {
    let mut accepted = result(WorkflowV2Status::Accepted);
    accepted.files_changed.push(file("src/a"));
    accepted.residual_gaps.push(gap("unresolved_finding"));
    let lessons = distil_lessons(
        &run(RunStatus::Completed),
        &[call("implement", true, accepted)],
    );
    let carried = lessons
        .iter()
        .find(|lesson| lesson.rule == LessonRule::AcceptedWithGaps)
        .expect("acceptance over an unresolved gap must distil a lesson");
    assert_eq!(carried.evidence.occurrences, 1);
    assert_eq!(carried.scope(), "acceptance");
}

#[test]
fn churn_is_counted_per_task_not_per_call() {
    // One task through six calls and another through two: one task churned.
    let universe = ["TASK-A", "TASK-B"];
    let mut records: Vec<WorkflowV2CallRecord> = (0..6)
        .map(|i| {
            with_tasks(
                call(
                    &format!("a{i}"),
                    i % 2 == 0,
                    result(WorkflowV2Status::Accepted),
                ),
                &universe,
                &["TASK-A"],
            )
        })
        .collect();
    records.extend((0..2).map(|i| {
        with_tasks(
            call(
                &format!("b{i}"),
                i % 2 == 0,
                result(WorkflowV2Status::Accepted),
            ),
            &universe,
            &["TASK-B"],
        )
    }));

    let lessons = distil_lessons(&run(RunStatus::Completed), &records);
    let churn = lessons
        .iter()
        .find(|lesson| lesson.rule == LessonRule::RepeatedTaskCycles)
        .expect("a task past the clean implement-and-verify pair must distil a lesson");
    assert_eq!(
        churn.evidence.occurrences, 1,
        "one task churned, not six calls"
    );
    assert_eq!(churn.evidence.calls, 8);
}

#[test]
fn recovering_from_one_bad_verdict_is_not_churn() {
    // implement, verify (negative), remediate, verify (accepted) — the recovery
    // loop working. A threshold that flags this teaches the next run that
    // remediation itself is the problem.
    let universe = ["TASK-A"];
    let records: Vec<WorkflowV2CallRecord> = (0..4)
        .map(|i| {
            with_tasks(
                call(
                    &format!("c{i}"),
                    i % 2 == 0,
                    result(WorkflowV2Status::Accepted),
                ),
                &universe,
                &["TASK-A"],
            )
        })
        .collect();
    let lessons = distil_lessons(&run(RunStatus::Completed), &records);
    assert!(
        !lessons
            .iter()
            .any(|lesson| lesson.rule == LessonRule::RepeatedTaskCycles),
        "one recovery cycle is the system working: {lessons:?}"
    );
}

#[test]
fn one_implement_and_one_verify_is_the_clean_shape_and_teaches_nothing() {
    let universe = ["TASK-A"];
    let mut implemented = result(WorkflowV2Status::Accepted);
    implemented.files_changed.push(file("src/a"));
    implemented.commands_run.push(command());
    let records = vec![
        with_tasks(call("implement", true, implemented), &universe, &["TASK-A"]),
        with_tasks(
            call("verify", false, result(WorkflowV2Status::Accepted)),
            &universe,
            &["TASK-A"],
        ),
    ];
    let lessons = distil_lessons(&run(RunStatus::Completed), &records);
    assert!(
        lessons.is_empty(),
        "a clean run should teach nothing: {lessons:?}"
    );
}

#[test]
fn a_run_that_never_completed_reports_the_tasks_it_never_reached() {
    let universe = ["TASK-A", "TASK-B", "TASK-C"];
    let records = vec![with_tasks(
        call("implement", true, {
            let mut r = result(WorkflowV2Status::Accepted);
            r.files_changed.push(file("src/a"));
            r
        }),
        &universe,
        &["TASK-A"],
    )];
    let lessons = distil_lessons(&run(RunStatus::NeedsReview), &records);
    let unfinished = lessons
        .iter()
        .find(|lesson| lesson.rule == LessonRule::UnfinishedRun)
        .expect("declared tasks that never reached an accepted call must distil a lesson");
    assert_eq!(
        unfinished.evidence.occurrences, 2,
        "B and C were never reached"
    );
    assert_eq!(unfinished.outcome, "needs_review");
    assert_eq!(unfinished.scope(), "plan");
}

#[test]
fn a_completed_run_is_never_told_it_left_tasks_unreached() {
    let universe = ["TASK-A", "TASK-B"];
    let records = vec![with_tasks(
        call("implement", true, {
            let mut r = result(WorkflowV2Status::Accepted);
            r.files_changed.push(file("src/a"));
            r
        }),
        &universe,
        &["TASK-A"],
    )];
    let lessons = distil_lessons(&run(RunStatus::Completed), &records);
    assert!(
        !lessons
            .iter()
            .any(|lesson| lesson.rule == LessonRule::UnfinishedRun),
        "a completed run makes no claim about unreached tasks"
    );
}

#[test]
fn a_run_with_no_calls_distils_nothing() {
    assert!(distil_lessons(&run(RunStatus::NeedsReview), &[]).is_empty());
}

fn lesson(rule: LessonRule, runs: usize, occurrences: usize) -> CuratedLesson {
    CuratedLesson {
        rule,
        evidence: LessonEvidence {
            runs,
            occurrences,
            calls: occurrences * 4,
        },
        run_id: "wf-test".to_string(),
        outcome: "needs_review".to_string(),
        ts: Utc::now(),
    }
}

#[test]
fn a_single_run_is_an_anecdote_and_is_not_rendered() {
    let block = render_lessons_block(&[lesson(LessonRule::SilentImplementation, 1, 9)]);
    assert!(
        block.is_empty(),
        "one run must not teach every later run: {block}"
    );
}

#[test]
fn the_rendered_block_is_capped_in_count_and_bytes() {
    // More lessons than the cap allows, so both bounds are actually exercised
    // rather than trivially satisfied by the rule count of the day.
    let lessons: Vec<CuratedLesson> = LessonRule::ALL
        .iter()
        .cycle()
        .take(MAX_RENDERED_LESSONS * 4)
        .map(|rule| lesson(*rule, 9, 9))
        .collect();
    let block = render_lessons_block(&lessons);
    assert!(block.len() <= MAX_LESSONS_BYTES, "block ran long");
    assert_eq!(
        block.lines().filter(|line| line.starts_with("- ")).count(),
        MAX_RENDERED_LESSONS,
        "the count cap must bind, not merely be satisfied"
    );
    assert!(
        !block.contains('/'),
        "rendered block leaked a path: {block}"
    );
}

#[test]
fn lessons_merge_across_runs_instead_of_accumulating() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::new(temp.path());
    for (run_id, occurrences) in [("wf-a", 2usize), ("wf-b", 3), ("wf-live", 7)] {
        write_lessons(
            &store,
            run_id,
            &[lesson(LessonRule::SilentImplementation, 1, occurrences)],
        )
        .unwrap();
    }

    // The in-flight run is excluded: a run must not be taught by its own
    // partial record.
    let merged = collect_curated_lessons(&store, Some("wf-live"));
    assert_eq!(merged.len(), 1, "one rule must collapse to one lesson");
    assert_eq!(merged[0].evidence.runs, 2);
    assert_eq!(merged[0].evidence.occurrences, 5);

    let block = curated_lessons_block(&store, Some("wf-live"));
    assert!(block.contains("seen in 2 runs, 5 of 20 calls"), "{block}");
    assert!(!block.contains("wf-"), "run ids must never be rendered");
}

#[test]
fn a_defect_that_stopped_happening_ages_out_of_the_window() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::new(temp.path());
    for index in 0..MAX_SOURCE_RUNS + 3 {
        write_lessons(
            &store,
            &format!("wf-{index:03}"),
            &[lesson(LessonRule::SilentImplementation, 1, 1)],
        )
        .unwrap();
    }

    let merged = collect_curated_lessons(&store, None);
    assert_eq!(merged.len(), 1);
    // Nothing deletes a lesson; the oldest runs simply fall outside the
    // window, which is how a fixed defect stops being taught.
    assert_eq!(
        merged[0].evidence.runs, MAX_SOURCE_RUNS,
        "the window must bound how many runs can vote"
    );
}

#[test]
fn an_unparseable_or_future_lesson_line_costs_one_line_not_the_stream() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join(LEARNING_LESSONS_FILE);
    let good = serde_json::to_string(&lesson(LessonRule::UnprovenNoop, 1, 1)).unwrap();
    std::fs::write(
        &path,
        format!("{{\"rule\":\"rule_from_the_future\"}}\nnot json at all\n{good}\n"),
    )
    .unwrap();
    let read = read_lessons(&path);
    assert_eq!(read.len(), 1);
    assert_eq!(read[0].rule, LessonRule::UnprovenNoop);
}

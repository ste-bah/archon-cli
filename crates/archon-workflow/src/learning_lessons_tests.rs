//! The curated stream's two invariants: prose that can never carry a path, and
//! a rendered block that can never grow without bound.

use super::*;
use crate::learning::learning_records;
use crate::run::{ArtifactRef, StageStatus};
use crate::spec::WorkflowSpec;

fn spec(stages: &str) -> WorkflowSpec {
    WorkflowSpec::from_yaml(&format!(
        "schema: archon.workflow.v1\nname: lesson-test\ntask: Lesson test\nstages:\n{stages}"
    ))
    .unwrap()
}

fn run_with(stages: &str) -> WorkflowRun {
    WorkflowRun::new(spec(stages), std::path::Path::new("/tmp/lessons-test"))
}

/// An implementation stage must declare targets and work units to validate;
/// none of that reaches a lesson, which is the point.
fn implementation(id: &str) -> String {
    format!(
        "  - id: {id}\n    kind: implementation\n    expected_target_files: [src/{id}]\n    task_ids: [T-{id}]\n"
    )
}

fn gate(id: &str, after: &str) -> String {
    format!("  - id: {id}\n    kind: quality_gate\n    depends_on: [{after}]\n")
}

/// One implementation stage and one quality gate — the ordinary shape.
fn implement_and_gate() -> WorkflowRun {
    run_with(&format!(
        "{}{}",
        implementation("build"),
        gate("check", "build")
    ))
}

fn accept(run: &mut WorkflowRun, id: &str) {
    run.stage_mut(id).unwrap().status = StageStatus::Accepted;
}

fn accept_with_artifact(run: &mut WorkflowRun, id: &str) {
    accept(run, id);
    run.stage_mut(id).unwrap().artifacts.push(ArtifactRef {
        id: format!("{id}-artifact"),
        path: std::path::PathBuf::from("artifact.md"),
        content_hash: "hash".to_string(),
        producing_stage: id.to_string(),
        source_input_hash: "input".to_string(),
        accepted: true,
    });
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
fn an_accepted_implementation_with_no_artifact_is_the_silent_failure() {
    let mut run = implement_and_gate();
    accept(&mut run, "build");
    accept(&mut run, "check");
    let lessons = distil_lessons(&run, &learning_records(&run));

    let silent = lessons
        .iter()
        .find(|lesson| lesson.rule == LessonRule::SilentImplementation)
        .expect("accepted implementation with no artifact must distil a lesson");
    assert_eq!(silent.evidence.runs, 1);
    assert_eq!(silent.evidence.occurrences, 1);
    assert_eq!(silent.evidence.stages, 2);
    assert_eq!(silent.scope(), "implementation");
}

#[test]
fn one_gate_per_implementation_does_not_teach_that_verification_is_a_problem() {
    let mut run = implement_and_gate();
    accept_with_artifact(&mut run, "build");
    accept_with_artifact(&mut run, "check");
    let lessons = distil_lessons(&run, &learning_records(&run));
    assert!(
        !lessons
            .iter()
            .any(|lesson| lesson.rule == LessonRule::VerificationDominance),
        "the intended shape must not fire the dominance rule: {lessons:?}"
    );
}

#[test]
fn judging_stages_outnumbering_producing_stages_fires_dominance() {
    let stages = format!(
        "{}{}{}{}",
        implementation("build"),
        gate("g1", "build"),
        gate("g2", "build"),
        gate("g3", "build")
    );
    let mut run = run_with(&stages);
    for id in ["build", "g1", "g2", "g3"] {
        accept_with_artifact(&mut run, id);
    }
    let lessons = distil_lessons(&run, &learning_records(&run));
    let dominance = lessons
        .iter()
        .find(|lesson| lesson.rule == LessonRule::VerificationDominance)
        .expect("three gates to one implementation must fire dominance");
    assert_eq!(dominance.evidence.occurrences, 3);
    assert_eq!(dominance.scope(), "verification");
}

#[test]
fn a_run_that_never_completed_reports_its_unverified_stages() {
    let mut run = implement_and_gate();
    run.status = RunStatus::NeedsReview;
    accept_with_artifact(&mut run, "build");
    // `check` stays Pending, which maps to Unverified.
    let lessons = distil_lessons(&run, &learning_records(&run));
    let unfinished = lessons
        .iter()
        .find(|lesson| lesson.rule == LessonRule::UnfinishedRun)
        .expect("a non-completed run with a pending stage must distil a lesson");
    assert_eq!(unfinished.evidence.occurrences, 1);
    assert_eq!(unfinished.outcome, "needs_review");
    assert_eq!(unfinished.scope(), "run");
}

#[test]
fn a_completed_run_with_durable_output_distils_nothing_to_warn_about() {
    let mut run = implement_and_gate();
    run.status = RunStatus::Completed;
    accept_with_artifact(&mut run, "build");
    accept_with_artifact(&mut run, "check");
    let lessons = distil_lessons(&run, &learning_records(&run));
    assert!(
        lessons.is_empty(),
        "a clean run should teach nothing: {lessons:?}"
    );
}

#[test]
fn retried_stages_are_counted_however_they_ended() {
    let mut run = implement_and_gate();
    accept_with_artifact(&mut run, "build");
    run.stage_mut("build").unwrap().attempt = 3;
    accept_with_artifact(&mut run, "check");
    let lessons = distil_lessons(&run, &learning_records(&run));
    let churn = lessons
        .iter()
        .find(|lesson| lesson.rule == LessonRule::RetryChurn)
        .expect("a stage past its first attempt must distil a lesson");
    assert_eq!(churn.evidence.occurrences, 1);
}

#[test]
fn the_provider_tier_and_not_the_stage_name_decides_what_counts_as_writing() {
    // In the v3 dialect a verification is an ordinary `agent` call, so the
    // stage kind cannot separate it from an investigation and the stage id is
    // project-specific. The tier the host assigns from the call method can.
    let stages = "  - id: alpha\n    kind: agent\n    provider_tier: coder\n  - id: beta\n    kind: agent\n    provider_tier: researcher\n  - id: gamma\n    kind: agent\n    provider_tier: researcher\n";
    let mut run = run_with(stages);
    for id in ["alpha", "beta", "gamma"] {
        accept(&mut run, id);
    }
    let lessons = distil_lessons(&run, &learning_records(&run));

    // `alpha` is the only writing call and it wrote nothing.
    let silent = lessons
        .iter()
        .find(|lesson| lesson.rule == LessonRule::SilentImplementation)
        .expect("a coder-tier stage with no artifact must distil a lesson");
    assert_eq!(
        silent.evidence.occurrences, 1,
        "the two researcher calls must not be mistaken for implementations"
    );

    let dominance = lessons
        .iter()
        .find(|lesson| lesson.rule == LessonRule::VerificationDominance)
        .expect("two inspecting calls to one writing call must fire dominance");
    assert_eq!(dominance.evidence.occurrences, 2);
}

fn lesson(rule: LessonRule, runs: usize, occurrences: usize) -> CuratedLesson {
    CuratedLesson {
        rule,
        evidence: LessonEvidence {
            runs,
            occurrences,
            stages: occurrences * 4,
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
    assert!(block.contains("seen in 2 runs, 5 of 20 stages"), "{block}");
    assert!(!block.contains("wf-"), "run ids must never be rendered");
}

#[test]
fn a_defect_that_stopped_happening_ages_out_of_the_window() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::new(temp.path());
    let sources = MAX_SOURCE_RUNS + 3;
    for index in 0..sources {
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
    let good = serde_json::to_string(&lesson(LessonRule::RetryChurn, 1, 1)).unwrap();
    std::fs::write(
        &path,
        format!("{{\"rule\":\"rule_from_the_future\"}}\nnot json at all\n{good}\n"),
    )
    .unwrap();
    let read = read_lessons(&path);
    assert_eq!(read.len(), 1);
    assert_eq!(read[0].rule, LessonRule::RetryChurn);
}

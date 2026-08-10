//! Reinforcement is gated on attribution (R2 slice, item 5).
//!
//! Split out of the parent test module to stay under the file-size gate. These
//! are the tests for the write this slice REMOVED: a correction only moves a
//! rule score once something has been shown to have caused it.

use super::*;

fn rule_score(memory: &Arc<dyn MemoryTrait>, rule_id: &str) -> f64 {
    memory.get_memory(rule_id).expect("get rule").importance
}

/// The gate, in the direction that must still work: an attributed correction
/// reinforces its rule.
///
/// Removing a write is only correct if the write still happens when it is
/// justified. Without this test the safe-looking outcome would be a system that
/// never reinforces anything.
#[tokio::test]
async fn an_accepted_attribution_reinforces_the_rule() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut agent = super::super::super::tests::test_agent();
    agent.config.session_id = "reinforce-session".into();
    agent.turn_number = 2;
    agent.state.messages = transcript("error: build failed", true);
    agent.set_cognitive_store(cognitive_store(temp.path()));
    let memory = graph();

    agent
        .detect_and_record_correction("no, that broke the build", &memory)
        .await;

    let store = cognitive_store(temp.path());
    let event = wait_for_attribution(&store).await;
    assert_eq!(identity(&event, "accepted"), "true");

    // FactualError: 50.0 base + 1.5 * 5.0.
    assert!(
        (rule_score(&memory, "rule:correction:factual-error:v2") - 57.5).abs() < f64::EPSILON,
        "an accepted attribution must reinforce, got {}",
        rule_score(&memory, "rule:correction:factual-error:v2")
    );
}

/// The gate in the direction it exists for: a correction nothing can explain
/// leaves every score where it found it.
///
/// The row is still written -- the unattributed cohort is the comparator the
/// promotion metric is computed against -- so "recorded" and "reinforced" come
/// apart here exactly as item 5 requires.
#[tokio::test]
async fn an_unattributed_correction_reinforces_nothing_but_is_still_recorded() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut agent = super::super::super::tests::test_agent();
    agent.config.session_id = "no-window-session".into();
    agent.turn_number = 1;
    agent.state.messages = vec![json!({"role": "user", "content": "no, that is wrong"})];
    agent.set_cognitive_store(cognitive_store(temp.path()));
    let memory = graph();

    agent
        .detect_and_record_correction("no, that is wrong", &memory)
        .await;

    let store = cognitive_store(temp.path());
    let event = wait_for_attribution(&store).await;
    assert_eq!(identity(&event, "attribution_cohort"), "unattributed");
    assert!(
        (rule_score(&memory, "rule:correction:factual-error:v2") - 50.0).abs() < f64::EPSILON,
        "an unattributed correction moved a score to {}",
        rule_score(&memory, "rule:correction:factual-error:v2")
    );
    // Recorded, linked, and recallable -- just not reinforcing.
    let stored = memory
        .search_memories(&archon_memory::types::SearchFilter {
            memory_type: Some(archon_memory::types::MemoryType::Correction),
            ..Default::default()
        })
        .expect("search");
    assert_eq!(stored.len(), 1);
}

/// No cognitive store means no attribution, and no attribution means no
/// reinforcement. Fail closed: the roadmap forbids a state mutation failing
/// open, and reinforcing unmeasured is exactly that.
#[tokio::test]
async fn a_correction_with_no_attribution_substrate_reinforces_nothing() {
    let mut agent = super::super::super::tests::test_agent();
    agent.config.session_id = "no-store-session".into();
    agent.turn_number = 2;
    agent.state.messages = transcript("error: build failed", true);
    let memory = graph();

    agent
        .detect_and_record_correction("no, that broke the build", &memory)
        .await;

    assert!(
        (rule_score(&memory, "rule:correction:factual-error:v2") - 50.0).abs() < f64::EPSILON,
        "reinforcement happened without any attribution to justify it"
    );
}

/// The lesson edge: an accepted attribution stores a lesson derived from the
/// correction, and the metric row names it.
#[tokio::test]
async fn an_accepted_attribution_stores_a_lesson_the_row_points_at() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut agent = super::super::super::tests::test_agent();
    agent.config.session_id = "lesson-session".into();
    agent.turn_number = 2;
    agent.state.messages = transcript("error: build failed", true);
    agent.set_cognitive_store(cognitive_store(temp.path()));

    agent
        .detect_and_record_correction("no, that broke the build", &graph())
        .await;

    let store = cognitive_store(temp.path());
    let event = wait_for_attribution(&store).await;
    let lesson_id = identity(&event, "lesson_id");
    assert!(
        !lesson_id.starts_with("no_lesson:"),
        "an accepted attribution must derive a lesson"
    );

    let lesson = archon_cognitive::attribution::lesson::read_causal_lesson(store.db(), lesson_id)
        .expect("read lesson")
        .expect("the lesson the row names exists");
    assert_eq!(lesson.correction_id, identity(&event, "correction_id"));
    assert_eq!(lesson.cause_action_class, "tool_run");
    assert_eq!(lesson.cause_label, "RunShell");
    assert!(
        lesson
            .evidence_refs
            .iter()
            .any(|reference| reference == "tool_result:is_error"),
        "the lesson carries the evidence its attribution rested on: {:?}",
        lesson.evidence_refs
    );
    assert!(
        !lesson.lesson.contains("broke the build"),
        "the user's words must not reach the lesson body: {}",
        lesson.lesson
    );
}

/// A refusal derives no lesson at all.
#[tokio::test]
async fn a_refused_attribution_derives_no_lesson() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut agent = super::super::super::tests::test_agent();
    agent.config.session_id = "no-lesson-session".into();
    agent.turn_number = 1;
    agent.state.messages = vec![json!({"role": "user", "content": "no, that is wrong"})];
    agent.set_cognitive_store(cognitive_store(temp.path()));

    agent
        .detect_and_record_correction("no, that is wrong", &graph())
        .await;

    let store = cognitive_store(temp.path());
    let event = wait_for_attribution(&store).await;
    assert!(identity(&event, "lesson_id").starts_with("no_lesson:"));
    assert!(
        archon_cognitive::attribution::lesson::causal_lessons(store.db())
            .expect("list lessons")
            .is_empty()
    );
}

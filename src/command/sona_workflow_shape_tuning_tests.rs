use super::*;

use archon_pipeline::learning::sona::MIN_OBSERVATIONS;

use archon_workflow::v2::decomposed_prd_plan::decomposed_prd_plan_calls;

fn consenting() -> LearningConfig {
    let mut learning = LearningConfig::default();
    learning.sona.enabled = true;
    learning.sona.pipeline_recording = true;
    learning
}

/// Seed `count` contention observations onto the width route, through the same
/// store and relation the runtime writes.
///
/// One batch rather than `count` single writes, for the reason given on the
/// sibling helper in `sona_workflow_tuning_tests.rs`: the guarded-write round
/// trip is charged once per script, and it is an order of magnitude dearer on
/// Windows than on Linux.
fn seed(project_root: &Path, class: &str, pressure: f64, count: u32) {
    let db =
        crate::command::topology_fold::open_store(&learning_store_path(project_root), "learning")
            .expect("learning store");
    archon_pipeline::learning::schema::initialize_learning_schemas(&db).expect("schemas");
    let knob = TunableShapeKnob::ImplementationWaveFanoutWidth;
    let route = SonaParameterTuner::route(class, knob.key());
    let observations: Vec<_> = (0..count)
        .map(|index| archon_pipeline::learning::sona::Trajectory {
            trajectory_id: format!("shape-seed-{index}"),
            route: route.clone(),
            agent_key: "generated-workflow-shape-tuner".to_string(),
            session_id: "seed".to_string(),
            patterns: Vec::new(),
            context: Vec::new(),
            embedding: Vec::new(),
            quality: pressure,
            reward: pressure,
            feedback_score: 1.0,
            weights_path: String::new(),
            created_at: u64::from(index) + 1,
            updated_at: u64::from(index) + 1,
        })
        .collect();
    trajectory_store::store_trajectory_batch(&db, &observations).expect("store observations");
}

fn tune(project_root: &Path, class: &str, learning: &LearningConfig) -> GeneratedShape {
    tune_generated_shape(
        project_root,
        class,
        learning,
        &decomposed_prd_plan_calls(),
        None,
    )
}

/// SONA off means the operator's concurrency, untouched, with nothing to report.
#[test]
fn a_project_without_sona_consent_gets_its_configured_concurrency() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut learning = LearningConfig::default();
    learning.sona.enabled = true;
    learning.sona.pipeline_recording = false;

    let shape = tune(temp.path(), "bug-hunt", &learning);

    assert_eq!(shape.implementation_wave_width, None);
    assert!(shape.decisions.is_empty());
    assert!(!shape.noteworthy());
    assert!(shape.report("bug-hunt").is_empty());
}

/// The first run on a project has no store. It must not create one, and it must
/// not guess.
#[test]
fn a_first_run_with_no_learning_store_gets_its_configured_concurrency() {
    let temp = tempfile::tempdir().expect("tempdir");

    let shape = tune(temp.path(), "greenfield", &consenting());

    assert_eq!(shape.implementation_wave_width, None);
    assert!(shape.decisions.is_empty());
    assert!(
        !learning_store_path(temp.path()).exists(),
        "reading must not create the store it reads"
    );
}

/// Constraint 1, end to end: evidence below the threshold reaches the bounds
/// layer as "no weight" and the run gets the configured concurrency.
#[test]
fn evidence_below_the_threshold_leaves_the_width_at_the_configured_cap() {
    let temp = tempfile::tempdir().expect("tempdir");
    seed(temp.path(), "bug-hunt", 1.0, MIN_OBSERVATIONS - 1);

    let shape = tune(temp.path(), "bug-hunt", &consenting());

    assert_eq!(
        shape.implementation_wave_width, None,
        "four contended runs are not enough to narrow anything"
    );
    let decision = shape.decisions.first().expect("one knob is reported");
    assert_eq!(decision.source, ShapeSource::InsufficientEvidence);
    assert_eq!(decision.observations, MIN_OBSERVATIONS - 1);
    assert!(shape.report("bug-hunt").is_empty());
}

/// Sustained contention narrows the fan-out, and the run says so.
#[test]
fn sustained_contention_narrows_the_fan_out_and_is_reported() {
    let temp = tempfile::tempdir().expect("tempdir");
    seed(temp.path(), "bug-hunt", 1.0, 40);

    let shape = tune(temp.path(), "bug-hunt", &consenting());
    let decision = shape.decisions.first().expect("one knob is reported");

    assert!(
        decision.weight > 0.0,
        "contention pressure must produce a positive weight, got {}",
        decision.weight
    );
    assert!(
        decision.applied <= decision.baseline,
        "the knob must never widen: {} -> {}",
        decision.baseline,
        decision.applied
    );
    if decision.applied < decision.baseline {
        assert_eq!(
            shape.implementation_wave_width,
            Some(u8::try_from(decision.applied).expect("small width"))
        );
        let report = shape.report("bug-hunt");
        assert!(
            report.contains("implementation_wave_fanout_width"),
            "{report}"
        );
        assert!(report.contains("observation(s)"), "{report}");
    }
}

/// The ratchet. No number of contention-free runs may widen a fan-out, because
/// no run ever tested a width above the operator's cap.
#[test]
fn no_amount_of_clean_evidence_widens_the_fan_out() {
    let temp = tempfile::tempdir().expect("tempdir");
    // Pressure 0.0 is the strongest possible "no contention" signal — stronger
    // than anything `observe_run` can actually record, which floors at 0.5.
    seed(temp.path(), "greenfield", 0.0, 60);

    let shape = tune(temp.path(), "greenfield", &consenting());
    let decision = shape.decisions.first().expect("one knob is reported");

    assert_eq!(
        shape.implementation_wave_width, None,
        "a contention-free history must leave the configured cap in place"
    );
    assert_eq!(decision.applied, decision.baseline);
}

/// Two classes must not read each other's evidence: a greenfield build and a
/// bug hunt contend differently, and averaging them describes neither.
#[test]
fn evidence_is_keyed_per_task_class() {
    let temp = tempfile::tempdir().expect("tempdir");
    seed(temp.path(), "bug-hunt", 1.0, 40);

    let other = tune(temp.path(), "greenfield", &consenting());

    assert_eq!(other.implementation_wave_width, None);
    let decision = other.decisions.first().expect("one knob is reported");
    assert_eq!(decision.observations, 0);
    assert_eq!(decision.source, ShapeSource::InsufficientEvidence);
}

/// Constraint 5 through the full read path: a learned narrowing that would
/// otherwise apply is withdrawn when the plan's review diamond is broken, and
/// the run keeps the operator's cap.
#[test]
fn a_learned_width_is_withdrawn_when_the_plan_lost_its_review_diamond() {
    let temp = tempfile::tempdir().expect("tempdir");
    seed(temp.path(), "bug-hunt", 1.0, 40);
    let mut plan = decomposed_prd_plan_calls();
    plan.iter_mut()
        .find(|call| call.id == "adversarial-review")
        .expect("the plan declares adversarial-review")
        .method = archon_workflow::WorkflowV2HostMethod::Reduce;

    let shape = tune_generated_shape(temp.path(), "bug-hunt", &consenting(), &plan, None);
    let decision = shape.decisions.first().expect("one knob is reported");

    assert_eq!(shape.implementation_wave_width, None);
    assert_eq!(decision.source, ShapeSource::RefusedByDependencyGraph);
    assert!(decision.refusal.is_some());
    let report = shape.report("bug-hunt");
    assert!(
        report.contains("refused before the run"),
        "a withdrawn proposal must still be visible in the run's own output: {report}"
    );
}

/// A route with no rows at all reports zero observations rather than erroring,
/// which is what makes the read path total.
#[test]
fn an_empty_store_reads_as_no_evidence() {
    let temp = tempfile::tempdir().expect("tempdir");
    seed(temp.path(), "bug-hunt", 0.5, 0);

    let shape = tune(temp.path(), "bug-hunt", &consenting());

    assert_eq!(shape.implementation_wave_width, None);
    assert_eq!(
        shape.decisions.first().expect("reported").observations,
        0,
        "an empty route is zero evidence, not missing evidence"
    );
}

#[test]
fn the_resolved_cap_is_at_least_one() {
    assert!(resolved_subagent_cap().is_some_and(|cap| cap >= 1));
}

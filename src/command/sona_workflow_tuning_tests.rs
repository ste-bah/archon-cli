use super::*;
use archon_pipeline::learning::sona::MIN_OBSERVATIONS;

fn consenting() -> LearningConfig {
    let mut learning = LearningConfig::default();
    learning.sona.enabled = true;
    learning.sona.pipeline_recording = true;
    learning
}

/// Seed `count` observations at one pressure onto one route, through the same
/// store and relation the runtime writes.
fn seed(
    project_root: &Path,
    class: &str,
    parameter: TunableGeneratedParameter,
    pressure: f64,
    count: u32,
) {
    let db =
        crate::command::topology_fold::open_store(&learning_store_path(project_root), "learning")
            .expect("learning store");
    archon_pipeline::learning::schema::initialize_learning_schemas(&db).expect("schemas");
    let route = SonaParameterTuner::route(class, parameter.key());
    for index in 0..count {
        let trajectory = archon_pipeline::learning::sona::Trajectory {
            trajectory_id: format!("tuning-seed-{}-{index}", parameter.key()),
            route: route.clone(),
            agent_key: "generated-workflow-tuner".to_string(),
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
        };
        trajectory_store::store_trajectory(&db, &trajectory).expect("store observation");
    }
}

fn decision_for(
    tuning: &GeneratedTuning,
    parameter: TunableGeneratedParameter,
) -> GeneratedTuningDecision {
    *tuning
        .decisions
        .iter()
        .find(|decision| decision.parameter == parameter)
        .expect("every parameter is reported once tuning ran")
}

/// SONA off means the operator's config, untouched, with nothing to report.
#[test]
fn a_project_without_sona_consent_gets_its_configured_limits() {
    let temp = tempfile::tempdir().expect("tempdir");
    let baseline = GeneratedWorkflowConfig::default();
    let mut learning = LearningConfig::default();
    learning.sona.enabled = true;
    learning.sona.pipeline_recording = false;

    let tuning = tune_generated_config(temp.path(), "bug-hunt", &learning, &baseline);

    assert_eq!(tuning.config.max_repair_iterations, 3);
    assert!(tuning.decisions.is_empty());
    assert!(!tuning.moved());
    assert!(tuning.report("bug-hunt").is_empty());
}

/// The first run on a project has no store. It must not create one, and it must
/// not guess.
#[test]
fn a_first_run_with_no_learning_store_gets_its_configured_limits() {
    let temp = tempfile::tempdir().expect("tempdir");

    let tuning = tune_generated_config(
        temp.path(),
        "greenfield",
        &consenting(),
        &GeneratedWorkflowConfig::default(),
    );

    assert!(tuning.decisions.is_empty());
    assert!(
        !learning_store_path(temp.path()).exists(),
        "reading must not create the store it reads"
    );
}

/// The fail-closed rule end to end: evidence below the threshold reaches the
/// config layer as "no weight" and the run gets the configured value.
#[test]
fn evidence_below_the_threshold_leaves_every_limit_at_its_configured_value() {
    let temp = tempfile::tempdir().expect("tempdir");
    seed(
        temp.path(),
        "bug-hunt",
        TunableGeneratedParameter::MaxRepairIterations,
        1.0,
        MIN_OBSERVATIONS - 1,
    );

    let tuning = tune_generated_config(
        temp.path(),
        "bug-hunt",
        &consenting(),
        &GeneratedWorkflowConfig::default(),
    );

    assert_eq!(tuning.config.max_repair_iterations, 3);
    let decision = decision_for(&tuning, TunableGeneratedParameter::MaxRepairIterations);
    assert_eq!(decision.source, TuningSource::InsufficientEvidence);
    assert_eq!(decision.observations, MIN_OBSERVATIONS - 1);
    assert!(!tuning.moved());
}

/// Enough consistent evidence of an exhausted repair budget lifts it, and the
/// run says so in words a user can act on.
#[test]
fn sustained_repair_exhaustion_lifts_the_budget_and_reports_why() {
    let temp = tempfile::tempdir().expect("tempdir");
    seed(
        temp.path(),
        "bug-hunt",
        TunableGeneratedParameter::MaxRepairIterations,
        1.0,
        40,
    );

    let tuning = tune_generated_config(
        temp.path(),
        "bug-hunt",
        &consenting(),
        &GeneratedWorkflowConfig::default(),
    );

    assert!(
        tuning.config.max_repair_iterations > 3,
        "40 exhausted budgets must lift the cap, got {}",
        tuning.config.max_repair_iterations
    );
    assert!(tuning.moved());
    let report = tuning.report("bug-hunt");
    assert!(report.contains("max_repair_iterations"), "{report}");
    assert!(report.contains("bug-hunt"), "{report}");
    assert!(report.contains("observation(s)"), "{report}");
}

/// The ratchet again, from the read side: any amount of clean-run evidence
/// leaves a verification timeout exactly where the operator put it.
#[test]
fn no_amount_of_neutral_evidence_shortens_a_verification_timeout() {
    let temp = tempfile::tempdir().expect("tempdir");
    seed(
        temp.path(),
        "review",
        TunableGeneratedParameter::VerificationBranchTimeoutSecs,
        0.5,
        200,
    );

    let tuning = tune_generated_config(
        temp.path(),
        "review",
        &consenting(),
        &GeneratedWorkflowConfig::default(),
    );

    assert_eq!(tuning.config.verification_branch_timeout_secs, 14_400);
}

/// Weights are keyed per task class, so evidence from one class must not move
/// another class's limits.
#[test]
fn evidence_recorded_for_one_class_does_not_move_another() {
    let temp = tempfile::tempdir().expect("tempdir");
    seed(
        temp.path(),
        "bug-hunt",
        TunableGeneratedParameter::MaxRepairIterations,
        1.0,
        40,
    );

    let other = tune_generated_config(
        temp.path(),
        "greenfield",
        &consenting(),
        &GeneratedWorkflowConfig::default(),
    );

    assert_eq!(other.config.max_repair_iterations, 3);
    assert_eq!(
        decision_for(&other, TunableGeneratedParameter::MaxRepairIterations).observations,
        0
    );
}

/// Whatever the store contains, the tuned config must still be one an operator
/// could have written.
#[test]
fn a_store_full_of_saturated_evidence_still_produces_a_validating_config() {
    let temp = tempfile::tempdir().expect("tempdir");
    for parameter in TunableGeneratedParameter::ALL {
        seed(temp.path(), "migration", parameter, 1.0, 400);
    }

    let tuning = tune_generated_config(
        temp.path(),
        "migration",
        &consenting(),
        &GeneratedWorkflowConfig::default(),
    );

    let mut config = archon_core::config::ArchonConfig::default();
    config.workflow.generated = tuning.config.clone();
    archon_core::config::validate(&config).expect("a learned config must still validate");
    assert!(tuning.config.verification_branch_timeout_secs >= tuning.config.host_call_timeout_secs);
}

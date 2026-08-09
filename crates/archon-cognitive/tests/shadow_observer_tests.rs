use archon_cognitive::{
    CandidateActionKind, CognitiveConfig, CognitiveSurface, LiveTurnOutcome, MetricEventStore,
    SHADOW_DEGRADED_MARKER, ShadowTurnInput, ShadowTurnObserver, WorldModelState,
    observed_action_from_tools, surprise_of,
};
use archon_policy::CognitivePolicy;
use cozo::{DbInstance, ScriptMutability};

fn config() -> CognitiveConfig {
    CognitiveConfig {
        enabled: true,
        record_decisions: true,
        record_reflections: true,
        use_self_model: true,
        max_candidates: 5,
        ..CognitiveConfig::default()
    }
}

fn policy() -> CognitivePolicy {
    CognitivePolicy {
        enabled: true,
        max_autonomous_risk: "Medium".into(),
        ..CognitivePolicy::default()
    }
}

fn turn(text: &str, dir: &std::path::Path) -> ShadowTurnInput {
    ShadowTurnInput {
        user_text: text.into(),
        session_id: "session-1".into(),
        turn_number: 1,
        surface: CognitiveSurface::Tui,
        working_dir: dir.to_path_buf(),
        world_model_state: WorldModelState::default(),
        model_id: "test-model".into(),
    }
}

fn live(action: Option<CandidateActionKind>) -> LiveTurnOutcome {
    LiveTurnOutcome {
        live_action_id: "session-1:1".into(),
        observed_action: action,
        completed: true,
        tool_failures: 0,
        user_corrected: false,
    }
}

fn count(db: &DbInstance, relation: &str, key: &str) -> usize {
    db.run_script(
        format!("?[id] := *{relation}{{{key}: id}}").as_str(),
        Default::default(),
        ScriptMutability::Immutable,
    )
    .unwrap()
    .rows
    .len()
}

#[test]
fn observation_records_a_plan_without_touching_the_live_decision_relation() {
    let db = DbInstance::new("mem", "", "").unwrap();
    let dir = tempfile::tempdir().unwrap();
    let observer = ShadowTurnObserver::new(&db, dir.path(), config(), Some(policy()));

    let observation = observer
        .observe(turn("fix the failing rust test", dir.path()))
        .unwrap()
        .expect("shadow plan");

    assert!(observation.selected_action.is_some());
    assert!(observation.candidate_rank >= 1);
    assert!(
        observation
            .degraded
            .contains(&SHADOW_DEGRADED_MARKER.to_string())
    );
    assert_eq!(
        count(&db, "cognitive_shadow_decisions", "shadow_decision_id"),
        1
    );
    // The live surfaces must keep meaning what they meant: a plan nobody
    // executed is not an executive decision and not a reflection.
    assert_eq!(count(&db, "cognitive_decisions", "decision_id"), 0);
    assert_eq!(count(&db, "cognitive_reflections", "reflection_id"), 0);
    // Nor a second situation row for a turn the live path already classified.
    assert_eq!(count(&db, "cognitive_situations", "situation_id"), 0);
    assert!(dir.path().join("cognitive-shadow-decisions.jsonl").exists());
    assert!(!dir.path().join("cognitive-reflections.jsonl").exists());
}

#[test]
fn disabled_config_plans_nothing() {
    let db = DbInstance::new("mem", "", "").unwrap();
    let dir = tempfile::tempdir().unwrap();
    let observer =
        ShadowTurnObserver::new(&db, dir.path(), CognitiveConfig::default(), Some(policy()));

    assert!(
        observer
            .observe(turn("fix the failing rust test", dir.path()))
            .unwrap()
            .is_none()
    );
}

#[test]
fn joining_an_agreeing_turn_emits_a_comparison_metric() {
    let db = DbInstance::new("mem", "", "").unwrap();
    let dir = tempfile::tempdir().unwrap();
    let observer = ShadowTurnObserver::new(&db, dir.path(), config(), Some(policy()));
    let observation = observer
        .observe(turn("fix the failing rust test", dir.path()))
        .unwrap()
        .expect("shadow plan");

    let comparison = observer
        .join(
            "session-1",
            1,
            &live(observation.selected_action),
            "test-model",
        )
        .unwrap()
        .expect("comparison");

    assert_eq!(comparison.agreed, Some(true));
    assert_eq!(comparison.surprise, Some(0.0));
    assert!(comparison.metric_recorded);

    let snapshot = MetricEventStore::new(&db, dir.path())
        .unwrap()
        .latest_snapshot()
        .unwrap();
    let agreement = snapshot
        .pooled("shadow_action_agreement_rate")
        .expect("agreement metric");
    assert_eq!(agreement.value, Some(1.0));
    assert_eq!(
        snapshot.pooled("shadow_surprise_mean").unwrap().value,
        Some(0.0)
    );
}

/// A turn that took a different action, did not complete, and failed tools is
/// the case the reflection trigger keys off.
#[test]
fn joining_a_diverging_turn_reports_surprise() {
    let db = DbInstance::new("mem", "", "").unwrap();
    let dir = tempfile::tempdir().unwrap();
    let observer = ShadowTurnObserver::new(&db, dir.path(), config(), Some(policy()));
    observer
        .observe(turn("fix the failing rust test", dir.path()))
        .unwrap()
        .expect("shadow plan");

    let comparison = observer
        .join(
            "session-1",
            1,
            &LiveTurnOutcome {
                live_action_id: "session-1:1".into(),
                observed_action: Some(CandidateActionKind::DeferOrDecline),
                completed: false,
                tool_failures: 9,
                user_corrected: true,
            },
            "test-model",
        )
        .unwrap()
        .expect("comparison");

    assert_eq!(comparison.agreed, Some(false));
    assert_eq!(comparison.surprise, Some(1.0));
}

/// Joining twice must not add a second observation to the population.
#[test]
fn a_joined_observation_is_not_joined_again() {
    let db = DbInstance::new("mem", "", "").unwrap();
    let dir = tempfile::tempdir().unwrap();
    let observer = ShadowTurnObserver::new(&db, dir.path(), config(), Some(policy()));
    let observation = observer
        .observe(turn("fix the failing rust test", dir.path()))
        .unwrap()
        .expect("shadow plan");
    observer
        .join(
            "session-1",
            1,
            &live(observation.selected_action),
            "test-model",
        )
        .unwrap()
        .expect("first join");

    assert!(
        observer
            .join(
                "session-1",
                1,
                &live(observation.selected_action),
                "test-model"
            )
            .unwrap()
            .is_none()
    );
    assert_eq!(
        MetricEventStore::new(&db, dir.path())
            .unwrap()
            .event_count(),
        1
    );
}

/// An unobservable live action is recorded as unknown, never as disagreement:
/// counting it as a miss would bias the agreement rate downward for free.
#[test]
fn an_unobservable_live_action_is_not_counted_as_disagreement() {
    let db = DbInstance::new("mem", "", "").unwrap();
    let dir = tempfile::tempdir().unwrap();
    let observer = ShadowTurnObserver::new(&db, dir.path(), config(), Some(policy()));
    observer
        .observe(turn("fix the failing rust test", dir.path()))
        .unwrap()
        .expect("shadow plan");

    let comparison = observer
        .join("session-1", 1, &live(None), "test-model")
        .unwrap()
        .expect("comparison");

    assert_eq!(comparison.agreed, None);
    assert_eq!(comparison.surprise, None);
    assert!(!comparison.metric_recorded);
    assert_eq!(
        MetricEventStore::new(&db, dir.path())
            .unwrap()
            .event_count(),
        0
    );
}

#[test]
fn joining_a_turn_with_no_shadow_plan_is_not_an_error() {
    let db = DbInstance::new("mem", "", "").unwrap();
    let dir = tempfile::tempdir().unwrap();
    let observer = ShadowTurnObserver::new(&db, dir.path(), config(), Some(policy()));

    assert!(
        observer
            .join("session-1", 7, &live(None), "test-model")
            .unwrap()
            .is_none()
    );
}

#[test]
fn tool_names_map_to_the_most_expensive_action_class() {
    assert_eq!(
        observed_action_from_tools(&[]),
        Some(CandidateActionKind::AnswerDirectly)
    );
    assert_eq!(
        observed_action_from_tools(&["Read".into(), "Bash".into()]),
        Some(CandidateActionKind::RunSafeShellProbe)
    );
    // An unmapped tool means the turn is unclassified, not "answered directly".
    assert_eq!(observed_action_from_tools(&["MysteryTool".into()]), None);
}

#[test]
fn surprise_is_bounded_and_zero_only_on_a_clean_agreeing_turn() {
    let agreeing = live(Some(CandidateActionKind::RunTests));
    assert_eq!(
        surprise_of(Some(CandidateActionKind::RunTests), &agreeing),
        Some(0.0)
    );
    assert_eq!(surprise_of(None, &agreeing), Some(0.5));
    assert_eq!(
        surprise_of(Some(CandidateActionKind::RunTests), &live(None)),
        None
    );
}

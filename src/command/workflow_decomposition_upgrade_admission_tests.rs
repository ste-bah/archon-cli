//! Issue 358 round 2: catalog schema admission from the build's own schema,
//! the transition record apart from the event log, and the dead-owner
//! recovery of a run whose state cannot be read.
use super::*;
use crate::command::workflow_decompose_transitions::{RuntimeTransitions, TRANSITIONS_PATH};

/// Rewrites a paused run's launch record as if `catalog` and `source` had
/// launched it, every digest and anchor consistent with them.
fn rebind_launch(
    store: &WorkflowStore,
    run_id: &str,
    state: &mut FixedDecompositionStateV1,
    catalog: &CommandCapabilityCatalog,
    source: &str,
) {
    state.identity.script_digest = workflow_scaffold_hash(source);
    state.identity.catalog_digest = catalog.digest.clone();
    store
        .write_run_json(run_id, FIXED_DECOMPOSITION_STATE_PATH, &*state)
        .unwrap();
    store
        .write_run_json(run_id, FIXED_CATALOG_PATH, catalog)
        .unwrap();
    let arguments: serde_json::Value = read_json(&store.run_dir(run_id).join(FIXED_ARGUMENTS_PATH));
    let route: crate::command::workflow_provider_route::TrustedProviderRouteSnapshot =
        read_json(&store.run_dir(run_id).join(FIXED_PROVIDER_ROUTE_PATH));
    let mut metadata: serde_json::Value =
        read_json(&store.run_dir(run_id).join(FIXED_GENERATED_METADATA_PATH));
    metadata["fixed_identity"] = serde_json::to_value(&state.identity).unwrap();
    metadata["scaffold_hash"] = serde_json::json!(state.identity.script_digest);
    store
        .write_run_json(run_id, FIXED_GENERATED_METADATA_PATH, &metadata)
        .unwrap();
    let mut run = store.load_state(run_id).unwrap();
    run.spec.permissions.insert(
        FIXED_LAUNCH_DIGEST_PERMISSION.into(),
        serde_json::json!(
            fixed_launch_digest(
                &state.identity,
                &arguments,
                catalog,
                &route,
                &serde_json::from_value(metadata["check_environment_policy"].clone()).unwrap(),
            )
            .unwrap()
        ),
    );
    store.save_state(&run).unwrap();
    WorkflowBundle::create_for_run(store, &run, source, WorkflowBundleOrigin::GeneratedHarness)
        .unwrap();
}

fn barrier(store: &WorkflowStore, run_id: &str) -> UpgradeBarrier {
    UpgradeBarrier {
        run_id: run_id.to_string(),
        store: store.clone(),
        expected: read_json::<FixedDecompositionStateV1>(
            &store.run_dir(run_id).join(FIXED_DECOMPOSITION_STATE_PATH),
        )
        .identity,
        builds: AtomicUsize::new(0),
    }
}

struct Lines(Arc<Mutex<Vec<String>>>);

#[async_trait::async_trait]
impl archon_workflow::WorkflowUiSink for Lines {
    async fn emit(
        &self,
        event: archon_workflow::WorkflowUiEvent,
    ) -> archon_workflow::WorkflowUiResult {
        if let archon_workflow::WorkflowUiEvent::Text(text) = event {
            self.0.lock().unwrap().push(text);
        }
        Ok(())
    }
}

/// Resumes as the build `revision`; the error and the printed lines.
async fn resume_as(
    project: &Path,
    store: &WorkflowStore,
    run_id: &str,
    revision: &str,
) -> (String, Vec<String>) {
    let lines = Arc::new(Mutex::new(Vec::new()));
    let error = resume_fixed_decomposition_at_binary_revision(
        project,
        run_id,
        true,
        &launch_config(project),
        &empty_env(),
        &barrier(store, run_id),
        Arc::new(Lines(Arc::clone(&lines))),
        None,
        None,
        revision,
    )
    .await
    .unwrap_err();
    let printed = lines.lock().unwrap().clone();
    (format!("{error:#}"), printed)
}

#[tokio::test]
async fn upgrade_358_catalog_schema_admission_follows_the_build_schema() {
    let current = fixed_decomposition_catalog("any").unwrap().schema_version;
    for (schema, admitted) in [(current, true), (1, true), (0, false), (current + 1, false)] {
        let project = fixture_project();
        let (store, run_id, _) =
            super::super::workflow_decomposition_drift_tests::launch_and_pause(project.path())
                .await;
        let mut state: FixedDecompositionStateV1 =
            read_json(&store.run_dir(&run_id).join(FIXED_DECOMPOSITION_STATE_PATH));
        let mut catalog =
            fixed_decomposition_catalog(&state.identity.starting_binary_revision).unwrap();
        catalog.schema_version = schema;
        catalog.recompute_digest().unwrap();
        rebind_launch(&store, &run_id, &mut state, &catalog, FIXED_SCRIPT_SOURCE);
        let resume = barrier(&store, &run_id);
        let message = format!(
            "{:#}",
            resume_fixed_decomposition_with_factory(
                project.path(),
                &run_id,
                true,
                &launch_config(project.path()),
                &empty_env(),
                &resume,
            )
            .await
            .unwrap_err()
        );
        if admitted {
            assert!(message.contains("barrier observed"), "{schema}: {message}");
            assert_eq!(resume.builds.load(Ordering::SeqCst), 1);
        } else {
            assert!(
                message.contains("command-catalog.schema_version")
                    && message.contains("paused")
                    && message.contains(&format!("found {schema}")),
                "{schema}: {message}"
            );
            assert_eq!(resume.builds.load(Ordering::SeqCst), 0);
        }
    }
}

fn transitions(store: &WorkflowStore, run_id: &str) -> RuntimeTransitions {
    read_json(&store.run_dir(run_id).join(TRANSITIONS_PATH))
}

/// The parseable events that show a transition, by their seq.
fn transition_events(store: &WorkflowStore, run_id: &str) -> Vec<serde_json::Value> {
    std::fs::read_to_string(store.events_path(run_id))
        .unwrap()
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter(|event| event["detail"]["transition_index"].is_u64())
        .collect()
}

fn log_of(store: &WorkflowStore, run_id: &str) -> String {
    let state: FixedDecompositionStateV1 =
        read_json(&store.run_dir(run_id).join(FIXED_DECOMPOSITION_STATE_PATH));
    std::fs::read_to_string(state.log_path).unwrap()
}

async fn torn_event_line(tail: &str) {
    let project = fixture_project();
    let (store, run_id, _) =
        super::super::workflow_decomposition_drift_tests::launch_and_pause(project.path()).await;
    let events = store.events_path(&run_id);
    let mut raw = std::fs::read(&events).unwrap_or_default();
    raw.extend_from_slice(tail.as_bytes());
    std::fs::write(&events, raw).unwrap();
    let (message, printed) = resume_as(project.path(), &store, &run_id, "upgraded-rev").await;
    assert!(message.contains("barrier observed"), "{message}");
    let record = transitions(&store, &run_id);
    assert_eq!(record.transitions.len(), 1);
    let seq = record.transitions[0].event_id.expect("event recorded");
    let shown = transition_events(&store, &run_id);
    assert_eq!(shown.len(), 1, "{shown:?}");
    assert_eq!(shown[0]["seq"], seq);
    assert!(
        log_of(&store, &run_id).contains(&format!(
            "event_id={seq} transition=binary_revision_drift persisted="
        )),
        "{}",
        log_of(&store, &run_id)
    );
    assert!(
        printed
            .iter()
            .any(|line| line.starts_with("Binary revision drift:")),
        "{printed:?}"
    );
}

#[tokio::test]
async fn upgrade_358_torn_last_event_line_does_not_block_resume() {
    torn_event_line("{\"seq\": 9999, \"kind\": \"stage_sta").await;
}
#[tokio::test]
async fn upgrade_358_torn_middle_event_line_does_not_block_resume() {
    torn_event_line("{\"seq\": 9999, \"kind\"\n{\"seq\":1,\"detail\":{\"event\":\"other\"}}\n")
        .await;
}

#[tokio::test]
async fn upgrade_358_crash_before_the_event_is_completed_once() {
    let project = fixture_project();
    let (store, run_id, _) =
        super::super::workflow_decomposition_drift_tests::launch_and_pause(project.path()).await;
    let launch = barrier(&store, &run_id).expected;
    let mut current = launch.clone();
    current.starting_binary_revision = "upgraded-rev".into();
    // The record was written; the crash came before its event.
    let mut record = RuntimeTransitions::default();
    record.transitions.push(
        crate::command::workflow_decompose_transitions::RuntimeTransition::new(launch, current),
    );
    store
        .write_run_json(&run_id, TRANSITIONS_PATH, &record)
        .unwrap();
    for attempt in 0..2 {
        let (message, printed) = resume_as(project.path(), &store, &run_id, "upgraded-rev").await;
        assert!(message.contains("barrier observed"), "{message}");
        // The completing resume shows the transition it completed, once.
        assert_eq!(
            printed
                .iter()
                .any(|line| line.starts_with("Binary revision drift:")),
            attempt == 0,
            "{printed:?}"
        );
    }
    let record = transitions(&store, &run_id);
    assert_eq!(record.transitions.len(), 1);
    let seq = record.transitions[0].event_id.expect("event completed");
    assert_eq!(transition_events(&store, &run_id).len(), 1);
    let key = format!("event_id={seq} transition=binary_revision_drift ");
    assert_eq!(log_of(&store, &run_id).matches(&key).count(), 1);
}

#[tokio::test]
async fn upgrade_358_crash_before_the_log_line_is_completed_once() {
    let project = fixture_project();
    let (store, run_id, launch_log) =
        super::super::workflow_decomposition_drift_tests::launch_and_pause(project.path()).await;
    let (message, _) = resume_as(project.path(), &store, &run_id, "upgraded-rev").await;
    assert!(message.contains("barrier observed"), "{message}");
    let seq = transitions(&store, &run_id).transitions[0]
        .event_id
        .unwrap();
    // The event was written; the crash came before the record knew its seq
    // and before the log line.
    let mut record = transitions(&store, &run_id);
    record.transitions[0].event_id = None;
    store
        .write_run_json(&run_id, TRANSITIONS_PATH, &record)
        .unwrap();
    let state: FixedDecompositionStateV1 =
        read_json(&store.run_dir(&run_id).join(FIXED_DECOMPOSITION_STATE_PATH));
    let log = std::fs::read_to_string(&state.log_path).unwrap();
    let kept: Vec<&str> = log
        .lines()
        .filter(|line| !line.starts_with(&format!("event_id={seq} ")))
        .collect();
    std::fs::write(&state.log_path, format!("{}\n", kept.join("\n"))).unwrap();
    assert!(log_of(&store, &run_id).starts_with(launch_log.trim_end()));
    let (message, printed) = resume_as(project.path(), &store, &run_id, "upgraded-rev").await;
    assert!(message.contains("barrier observed"), "{message}");
    assert!(
        !printed
            .iter()
            .any(|line| line.starts_with("Binary revision drift")
                || line.starts_with("Decomposition runtime upgrade")),
        "no new transition: {printed:?}"
    );
    assert_eq!(transition_events(&store, &run_id).len(), 1);
    assert_eq!(
        transitions(&store, &run_id).transitions[0].event_id,
        Some(seq)
    );
    let key = format!("event_id={seq} transition=binary_revision_drift ");
    assert_eq!(log_of(&store, &run_id).matches(&key).count(), 1);
}

#[tokio::test]
async fn upgrade_358_harness_upgrade_is_shown_with_its_digests() {
    let project = fixture_project();
    let (store, run_id, _) =
        super::super::workflow_decomposition_drift_tests::launch_and_pause(project.path()).await;
    let mut state: FixedDecompositionStateV1 =
        read_json(&store.run_dir(&run_id).join(FIXED_DECOMPOSITION_STATE_PATH));
    let catalog = fixed_decomposition_catalog(&state.identity.starting_binary_revision).unwrap();
    let source = format!("{FIXED_SCRIPT_SOURCE}\n// earlier harness\n");
    rebind_launch(&store, &run_id, &mut state, &catalog, &source);
    let revision = state.identity.starting_binary_revision.clone();
    let (message, printed) = resume_as(project.path(), &store, &run_id, &revision).await;
    assert!(message.contains("barrier observed"), "{message}");
    let (old, new) = (
        state.identity.script_digest.clone(),
        workflow_scaffold_hash(FIXED_SCRIPT_SOURCE),
    );
    assert!(
        printed
            .iter()
            .any(|line| line.starts_with("Decomposition runtime upgrade:")
                && line.contains(&format!("script {old}->{new}"))),
        "{printed:?}"
    );
    assert!(
        !printed
            .iter()
            .any(|line| line.starts_with("Binary revision drift")),
        "{printed:?}"
    );
    assert!(
        log_of(&store, &run_id).contains(&format!(
            "transition=decomposition_runtime_upgrade persisted={revision} current={revision}"
        )) && log_of(&store, &run_id).contains(&format!("script={old}->{new}")),
        "{}",
        log_of(&store, &run_id)
    );
    let status = crate::command::workflow_decompose_transitions::status_lines(&store, &run_id);
    assert!(
        status.contains("runtime_transitions: 1\n")
            && status.contains(&format!("script_digest={new}")),
        "{status}"
    );
}

#[tokio::test]
async fn upgrade_358_unreadable_state_recovers_the_dead_owner() {
    for (field, value) in [
        ("schema_version", serde_json::json!(99)),
        ("identity", serde_json::json!({})),
    ] {
        let project = fixture_project();
        let (store, run_id, _) =
            super::super::workflow_decomposition_drift_tests::launch_and_pause(project.path())
                .await;
        let path = store.run_dir(&run_id).join(FIXED_DECOMPOSITION_STATE_PATH);
        let mut state: serde_json::Value = read_json(&path);
        state[field] = value;
        store
            .write_run_json(&run_id, FIXED_DECOMPOSITION_STATE_PATH, &state)
            .unwrap();
        let mut run = store.load_state(&run_id).unwrap();
        run.status = RunStatus::Running;
        store.save_state(&run).unwrap();
        let generation = run.generation;
        let recoveries = || {
            std::fs::read_to_string(store.events_path(&run_id))
                .unwrap_or_default()
                .lines()
                .filter(|line| line.contains("\"stale_owner_recovered\""))
                .count()
        };
        for attempt in 0..2 {
            let factory = BarrierFactory::resume(
                project
                    .path()
                    .canonicalize()
                    .map(archon_shell::paths::plain)
                    .unwrap(),
            );
            let lines = Arc::new(Mutex::new(Vec::new()));
            let message = format!(
                "{:#}",
                resume_fixed_decomposition_with_factory_and_sink(
                    project.path(),
                    &run_id,
                    true,
                    &launch_config(project.path()),
                    &empty_env(),
                    &factory,
                    Arc::new(Lines(Arc::clone(&lines))),
                    None,
                    None,
                )
                .await
                .unwrap_err()
            );
            assert!(
                message.contains(field) && message.contains("paused"),
                "{message}"
            );
            assert_eq!(factory.builds.load(Ordering::SeqCst), 0);
            let after = store.load_state(&run_id).unwrap();
            assert_eq!(after.status, RunStatus::Paused);
            // Fenced once, when the run left Running; never again.
            assert_eq!(
                after.generation,
                generation + 1,
                "{field} attempt {attempt}"
            );
            assert_eq!(recoveries(), 1, "{field} attempt {attempt}");
            let printed = lines.lock().unwrap().clone();
            assert_eq!(
                printed
                    .iter()
                    .any(|line| line.starts_with("Stale owner recovered")),
                attempt == 0,
                "{printed:?}"
            );
        }
    }
}

#[path = "workflow_decomposition_upgrade_visible_tests.rs"]
mod visible;

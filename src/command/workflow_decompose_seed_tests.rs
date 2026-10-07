//! Issue 360: the seed record is derived once per upgrade, visible once, and
//! read back unchanged by every later resume on that runtime.
use super::*;
use crate::command::workflow_decompose_transitions::RuntimeTransition;
use derive::tests::{entry, finding, gate, reply};
use serde_json::json;

fn identity(script: &str, revision: &str) -> archon_workflow::FixedRunIdentityV1 {
    archon_workflow::FixedRunIdentityV1 {
        template_version: "fixed-decomposition-v1".into(),
        starting_binary_revision: revision.into(),
        script_digest: script.into(),
        catalog_digest: "catalog".into(),
        project_root_identity: "/project".into(),
        prd_identity: "/project/PRD.md".into(),
        task_root_identity: "/project/tasks".into(),
    }
}

/// Records the transitions `steps` (script digest, revision) after the launch.
pub(crate) fn record_transitions(store: &WorkflowStore, run_id: &str, steps: &[(&str, &str)]) {
    let mut record = RuntimeTransitions::default();
    let mut previous = identity("launch-script", "launch-rev");
    for (script, revision) in steps {
        let next = identity(script, revision);
        record
            .transitions
            .push(RuntimeTransition::new(previous, next.clone()));
        previous = next;
    }
    store
        .write_run_json(run_id, transitions::TRANSITIONS_PATH, &record)
        .unwrap();
}

fn save(store: &WorkflowStore, run_id: &str, record: &archon_workflow::WorkflowV2CallRecord) {
    let mut record = record.clone();
    record.run_id = run_id.into();
    WorkflowV2ResultStore::new(store.run_dir(run_id).join("v2"))
        .save_call_record(&record)
        .unwrap();
}

fn fixture() -> (tempfile::TempDir, WorkflowStore, String, std::path::PathBuf) {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowStore::new(temp.path().join("workflows"));
    let run = store
        .create_run(archon_workflow::WorkflowSpec {
            schema: archon_workflow::spec::WORKFLOW_SCHEMA.to_string(),
            name: "seed-test".into(),
            task: "test".into(),
            target_repository_root: None,
            max_parallelism: 1,
            max_agents: 1,
            stages: Vec::new(),
            permissions: Default::default(),
            learning_hooks: Vec::new(),
        })
        .unwrap();
    let log = temp.path().join("tasks/.decompose.log");
    let candidate = json!({"entries": [entry("AC-1"), entry("AC-2")]}).to_string();
    save(
        &store,
        &run.id,
        &reply(
            "acceptance-author-AC-1-4",
            "01:00:00",
            &entry("AC-1").to_string(),
        ),
    );
    save(
        &store,
        &run.id,
        &gate(
            "freeze-acceptance",
            "02:00:00",
            &candidate,
            vec![finding(
                "check 'AC-2' was refuted by the host judge; reason: weak",
                "AC-2",
            )],
            false,
        ),
    );
    (temp, store, run.id, log)
}

fn criteria() -> BTreeSet<String> {
    ["AC-1".to_string(), "AC-2".to_string()].into()
}

fn seed_events(store: &WorkflowStore, run_id: &str) -> Vec<serde_json::Value> {
    std::fs::read_to_string(store.events_path(run_id))
        .unwrap_or_default()
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter(|event| event["detail"]["event"] == SEED_EVENT)
        .collect()
}

#[test]
fn no_upgrade_and_a_revision_drift_alone_start_no_seed() {
    let (_temp, store, run_id, log) = fixture();
    assert_eq!(
        current_seed(&store, &run_id, &log, &criteria()).unwrap(),
        None
    );
    record_transitions(&store, &run_id, &[("launch-script", "next-rev")]);
    assert_eq!(
        current_seed(&store, &run_id, &log, &criteria()).unwrap(),
        None
    );
    assert_eq!(status_lines(&store, &run_id), "");
    let args = json!({"prdPath": "p"});
    assert_eq!(
        seeded_arguments(&args, None),
        args,
        "the launch-bound arguments, unchanged"
    );
}

#[test]
fn an_upgrade_derives_one_seed_visible_once_and_read_back_unchanged() {
    let (_temp, store, run_id, log) = fixture();
    record_transitions(&store, &run_id, &[("new-script", "next-rev")]);
    let seed = current_seed(&store, &run_id, &log, &criteria())
        .unwrap()
        .unwrap();
    assert_eq!(seed.transition_index, 0);
    assert!(seed.event_id.is_some());
    assert!(matches!(
        &seed.subjects["acceptance"],
        SubjectSeed::Entries { carried: 2, .. }
    ));
    // Work the seeded run records later never changes the seed it started from.
    save(
        &store,
        &run_id,
        &reply(
            "acceptance-author-AC-2-28",
            "03:00:00",
            &entry("AC-2").to_string(),
        ),
    );
    let again = current_seed(&store, &run_id, &log, &criteria())
        .unwrap()
        .unwrap();
    assert_eq!(again, seed);
    assert_eq!(seed_events(&store, &run_id).len(), 1);
    let log_text = std::fs::read_to_string(&log).unwrap();
    assert_eq!(log_text.matches(SEED_EVENT).count(), 1, "{log_text}");
    let status = status_lines(&store, &run_id);
    assert!(status.contains("phase_seed: transition=0"), "{status}");
    assert!(status.contains("acceptance: carried_entries=2"), "{status}");
    let args = seeded_arguments(&json!({"prdPath": "p"}), Some(&seed));
    assert_eq!(args["phaseSeed"]["author_ordinals"]["acceptance"], 4);
    assert_eq!(args["prdPath"], "p");
}

#[test]
fn a_crash_between_the_seed_record_and_its_event_writes_the_event_once() {
    let (_temp, store, run_id, log) = fixture();
    record_transitions(&store, &run_id, &[("new-script", "next-rev")]);
    // Crash after the record, before its event.
    let derived = derive_seed(
        &store,
        &run_id,
        (0, identity("new-script", "next-rev")),
        &criteria(),
    )
    .unwrap();
    write_seed(&store, &run_id, &derived).unwrap();
    let seed = current_seed(&store, &run_id, &log, &criteria())
        .unwrap()
        .unwrap();
    assert_eq!(
        seed.subjects, derived.subjects,
        "the recorded seed, not a new derivation"
    );
    // Crash after the event, before the record holds its id.
    let mut torn = seed.clone();
    torn.event_id = None;
    write_seed(&store, &run_id, &torn).unwrap();
    let again = current_seed(&store, &run_id, &log, &criteria())
        .unwrap()
        .unwrap();
    assert_eq!(again.event_id, seed.event_id);
    assert_eq!(seed_events(&store, &run_id).len(), 1);
}

#[test]
fn a_second_upgrade_derives_a_new_seed_from_the_seeded_round_and_keeps_the_first() {
    let (_temp, store, run_id, log) = fixture();
    record_transitions(&store, &run_id, &[("new-script", "next-rev")]);
    let first = current_seed(&store, &run_id, &log, &criteria())
        .unwrap()
        .unwrap();
    // The seeded round re-authored the refuted entry; then the run paused.
    let mut repaired = entry("AC-2");
    repaired["check"]["command"] = json!("stronger");
    save(
        &store,
        &run_id,
        &reply(
            "acceptance-author-AC-2-28",
            "03:00:00",
            &repaired.to_string(),
        ),
    );
    record_transitions(
        &store,
        &run_id,
        &[("new-script", "next-rev"), ("newer-script", "next-rev")],
    );
    let second = current_seed(&store, &run_id, &log, &criteria())
        .unwrap()
        .unwrap();
    assert_eq!(second.transition_index, 1);
    assert_eq!(
        second.author_ordinals["acceptance"], 28,
        "the seeded round's ordinals continue"
    );
    let SubjectSeed::Entries { replies, .. } = &second.subjects["acceptance"] else {
        panic!()
    };
    assert_eq!(replies[0].call_id, "acceptance-author-AC-2-28");
    assert_eq!(
        read_seed(&store, &run_id, 0, &identity("new-script", "next-rev"))
            .unwrap()
            .unwrap(),
        first,
        "the first seed stays as evidence"
    );
    assert_eq!(seed_events(&store, &run_id).len(), 2);
    // A revision drift after the second upgrade keeps its seed.
    record_transitions(
        &store,
        &run_id,
        &[
            ("new-script", "next-rev"),
            ("newer-script", "next-rev"),
            ("newer-script", "third-rev"),
        ],
    );
    assert_eq!(
        current_seed(&store, &run_id, &log, &criteria())
            .unwrap()
            .unwrap()
            .transition_index,
        1
    );
}

#[test]
fn an_unreadable_seed_record_pauses_the_resume() {
    let (_temp, store, run_id, log) = fixture();
    record_transitions(&store, &run_id, &[("new-script", "next-rev")]);
    current_seed(&store, &run_id, &log, &criteria()).unwrap();
    let path = store
        .run_dir(&run_id)
        .join(seed_path(0, &identity("new-script", "next-rev")));
    let mut value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    value["schema_version"] = json!(99);
    std::fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
    let error = current_seed(&store, &run_id, &log, &criteria()).unwrap_err();
    assert!(format!("{error:#}").contains("paused"), "{error:#}");
    assert!(status_lines(&store, &run_id).contains("unreadable"));
}

/// L1: the seeds directory's own entry is flushed into `decomposition/` with
/// every seed write, so a power loss never loses the seed that started a
/// seeded round (its next derivation would see the seeded calls).
#[test]
fn every_seed_write_connects_the_seeds_directory_durably() {
    let take = crate::command::workflow_task_set::take_synced_dirs;
    let (_temp, store, run_id, log) = fixture();
    let run_dir = store.run_dir(&run_id);
    let (decomposition, seeds) = (run_dir.join("decomposition"), run_dir.join(SEEDS_DIR));
    let flushed = |synced: Vec<std::path::PathBuf>| {
        assert!(synced.contains(&decomposition), "{synced:?}");
        assert!(synced.contains(&seeds), "{synced:?}");
    };
    // The first seed creates the directory.
    record_transitions(&store, &run_id, &[("new-script", "next-rev")]);
    take();
    current_seed(&store, &run_id, &log, &criteria()).unwrap();
    flushed(take());
    // A crash left the directory created but its entry unflushed.
    std::fs::remove_dir_all(&seeds).unwrap();
    std::fs::create_dir_all(&seeds).unwrap();
    take();
    current_seed(&store, &run_id, &log, &criteria()).unwrap();
    flushed(take());
    // A second upgrade writes into the existing directory.
    record_transitions(
        &store,
        &run_id,
        &[("new-script", "next-rev"), ("newer-script", "next-rev")],
    );
    take();
    current_seed(&store, &run_id, &log, &criteria()).unwrap();
    flushed(take());
}

/// L3: a seed names the runtime it was derived for and is read back for that
/// runtime only.
#[test]
fn a_seed_is_read_back_only_for_the_runtime_it_was_derived_for() {
    let (_temp, store, run_id, log) = fixture();
    record_transitions(&store, &run_id, &[("new-script", "next-rev")]);
    let first = current_seed(&store, &run_id, &log, &criteria())
        .unwrap()
        .unwrap();
    assert_eq!(first.runtime, identity("new-script", "next-rev"));
    let mut repaired = entry("AC-2");
    repaired["check"]["command"] = json!("stronger");
    save(
        &store,
        &run_id,
        &reply(
            "acceptance-author-AC-2-28",
            "03:00:00",
            &repaired.to_string(),
        ),
    );
    // The transition record was lost and rebuilt on another runtime at the
    // same index: that runtime's own seed, derived from every record.
    record_transitions(&store, &run_id, &[("other-script", "other-rev")]);
    let other = current_seed(&store, &run_id, &log, &criteria())
        .unwrap()
        .unwrap();
    assert_eq!(other.transition_index, 0);
    assert_eq!(other.runtime.script_digest, "other-script");
    assert_eq!(
        other.author_ordinals["acceptance"], 28,
        "never the other runtime's older candidate and ordinals"
    );
    let SubjectSeed::Entries { replies, .. } = &other.subjects["acceptance"] else {
        panic!()
    };
    assert_eq!(replies[0].call_id, "acceptance-author-AC-2-28");
    assert_eq!(
        read_seed(&store, &run_id, 0, &first.runtime)
            .unwrap()
            .unwrap(),
        first,
        "the first runtime's seed stays as evidence"
    );
    let events = seed_events(&store, &run_id);
    assert_eq!(events.len(), 2, "its own event");
    assert_eq!(
        events[1]["detail"]["runtime"]["script_digest"],
        "other-script"
    );
    assert_ne!(events[0]["detail"]["record"], events[1]["detail"]["record"]);
    // Rebuilt on the first runtime (another binary revision): its seed and
    // its event, read back.
    record_transitions(&store, &run_id, &[("new-script", "later-rev")]);
    let back = current_seed(&store, &run_id, &log, &criteria())
        .unwrap()
        .unwrap();
    assert_eq!(back, first);
    assert_eq!(seed_events(&store, &run_id).len(), 2);
    // A record whose content names another runtime than its file pauses.
    let path = store.run_dir(&run_id).join(seed_path(0, &first.runtime));
    let mut value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    value["runtime"]["catalog_digest"] = json!("edited-catalog");
    std::fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
    let error = format!(
        "{:#}",
        current_seed(&store, &run_id, &log, &criteria()).unwrap_err()
    );
    assert!(
        error.contains("paused") && error.contains("never used for another runtime"),
        "{error}"
    );
}

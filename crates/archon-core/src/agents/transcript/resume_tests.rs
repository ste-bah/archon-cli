//! #241: a resume sends the spawn's confinement again, and refuses an agent
//! whose metadata does not record it.

use super::*;
use crate::agents::transcript::{AgentMetadata, InheritedConfinement, RecordedIsolation};
use archon_tools::isolation::IsolationTier;
use tempfile::TempDir;

fn store_with_transcript(agent_id: &str) -> (AgentTranscriptStore, TempDir) {
    let tmp = TempDir::new().unwrap();
    let store = AgentTranscriptStore::with_base_dir(tmp.path().to_path_buf());
    store.record_message(
        agent_id,
        &serde_json::json!({"role": "user", "content": "hi"}),
    );
    store.record_message(
        agent_id,
        &serde_json::json!({"role": "assistant", "content": "hello"}),
    );
    (store, tmp)
}

fn inherited() -> InheritedConfinement {
    InheritedConfinement {
        workflow: false,
        sealed_repositories: Vec::new(),
        denied_directory_names: Vec::new(),
        parent_subagent: None,
    }
}

fn confinement(isolation: RecordedIsolation, tier: IsolationTier) -> SpawnConfinement {
    SpawnConfinement {
        isolation,
        tier,
        cwd: "/work/space".into(),
        read_roots: vec!["/project/spec.md".into()],
        write_roots: vec!["/project/out".into()],
        allowed_tools: vec!["Read".into(), "Write".into()],
        model: Some("model-a".into()),
        max_turns: 7,
        timeout_secs: 90,
        inherited: inherited(),
    }
}

fn boundary() -> SpawnConfinement {
    confinement(RecordedIsolation::WorkspaceBoundary, IsolationTier::Shared)
}

fn metadata(confinement: Option<SpawnConfinement>) -> AgentMetadata {
    AgentMetadata {
        agent_type: "explore".into(),
        worktree_path: None,
        description: None,
        filename: None,
        confinement,
    }
}

fn plan(store: &AgentTranscriptStore, agent_id: &str) -> Result<ResumePlan, String> {
    plan_resume(store, agent_id, "next step").expect("a transcript exists")
}

#[test]
fn a_boundary_agent_is_resumed_with_its_exact_confinement_and_limits() {
    let (store, _tmp) = store_with_transcript("a1");
    store.write_metadata("a1", &metadata(Some(boundary())));

    let plan = plan(&store, "a1").expect("resumable");
    let request = plan.request;
    assert_eq!(request.isolation.as_deref(), Some("workspace-boundary"));
    assert_eq!(request.cwd.as_deref(), Some("/work/space"));
    assert_eq!(request.read_roots, vec!["/project/spec.md".to_string()]);
    assert_eq!(request.write_roots, vec!["/project/out".to_string()]);
    assert_eq!(request.allowed_tools, vec!["Read", "Write"]);
    assert_eq!(request.model.as_deref(), Some("model-a"));
    assert_eq!((request.max_turns, request.timeout_secs), (7, 90));
    assert_eq!(request.subagent_type.as_deref(), Some("explore"));
    assert_eq!(request.prompt, "next step");
    assert_eq!(plan.messages.len(), 2);
    assert_eq!(plan.confinement, boundary());
}

#[test]
fn a_worktree_rung_reached_by_policy_travels_as_the_record_not_the_request() {
    let (store, _tmp) = store_with_transcript("a2");
    // Reached by the automatic policy: the request named no isolation. The
    // request asks for nothing; the executor pins the rung from the record.
    let record = confinement(RecordedIsolation::Unset, IsolationTier::Worktree);
    store.write_metadata("a2", &metadata(Some(record.clone())));
    let (request, pending) = plan(&store, "a2").unwrap().into_pending();
    assert_eq!(request.isolation, None);
    assert_eq!(pending.confinement, record);
    assert_eq!(pending.messages.len(), 2);
}

#[test]
fn a_boundary_on_a_worktree_rung_keeps_both() {
    let (store, _tmp) = store_with_transcript("a4");
    let record = confinement(
        RecordedIsolation::WorkspaceBoundary,
        IsolationTier::Worktree,
    );
    store.write_metadata("a4", &metadata(Some(record.clone())));
    let plan = plan(&store, "a4").unwrap();
    assert_eq!(
        plan.request.isolation.as_deref(),
        Some("workspace-boundary")
    );
    assert_eq!(plan.confinement.tier, IsolationTier::Worktree);
}

#[test]
fn a_shared_rung_sends_what_the_spawn_asked_for() {
    let (store, _tmp) = store_with_transcript("a3");
    let record = confinement(RecordedIsolation::Unset, IsolationTier::Shared);
    store.write_metadata("a3", &metadata(Some(record)));
    let request = plan(&store, "a3").unwrap().request;
    assert_eq!(request.isolation, None);
    assert_eq!(request.cwd.as_deref(), Some("/work/space"));
}

#[test]
fn metadata_written_before_the_record_parses_and_is_refused() {
    let (store, _tmp) = store_with_transcript("old-1");
    std::fs::write(
        store.metadata_path("old-1"),
        r#"{"agent_type": "explore", "description": "old"}"#,
    )
    .unwrap();
    let meta = store.read_metadata("old-1").expect("old metadata parses");
    assert!(meta.confinement.is_none());

    let refusal = plan(&store, "old-1").expect_err("unknown confinement is refused");
    assert!(refusal.contains("'old-1'"), "{refusal}");
    assert!(
        refusal.contains("isolation, tier, cwd, read_roots, write_roots"),
        "{refusal}"
    );
    assert!(refusal.contains("cannot tell"), "{refusal}");
    assert!(
        refusal.contains(&store.transcript_path("old-1").display().to_string()),
        "the refusal must say where the history is: {refusal}"
    );
}

#[test]
fn old_metadata_of_a_worktree_agent_is_refused_as_confined() {
    let (store, _tmp) = store_with_transcript("old-2");
    let mut meta = metadata(None);
    meta.worktree_path = Some("/wt/agent".into());
    store.write_metadata("old-2", &meta);
    let refusal = plan(&store, "old-2").unwrap_err();
    assert!(refusal.contains("worktree /wt/agent"), "{refusal}");
    assert!(refusal.contains("so it was confined"), "{refusal}");
}

#[test]
fn missing_metadata_is_refused() {
    let (store, _tmp) = store_with_transcript("old-3");
    let refusal = plan(&store, "old-3").unwrap_err();
    assert!(refusal.contains("missing or unreadable"), "{refusal}");
}

#[test]
fn an_unknown_recorded_isolation_is_refused() {
    let (store, _tmp) = store_with_transcript("bad-2");
    let mut meta = serde_json::to_value(metadata(Some(boundary()))).unwrap();
    meta["confinement"]["isolation"] = "sealed-ish".into();
    std::fs::write(store.metadata_path("bad-2"), meta.to_string()).unwrap();
    let refusal = plan(&store, "bad-2").unwrap_err();
    assert!(refusal.contains("sealed-ish"), "{refusal}");
    assert!(refusal.contains("'bad-2'"), "{refusal}");
}

#[test]
fn an_unknown_record_field_is_refused() {
    let (store, _tmp) = store_with_transcript("bad-3");
    let mut meta = serde_json::to_value(metadata(Some(boundary()))).unwrap();
    meta["confinement"]["sandbox"] = "off".into();
    std::fs::write(store.metadata_path("bad-3"), meta.to_string()).unwrap();
    let refusal = plan(&store, "bad-3").unwrap_err();
    assert!(refusal.contains("sandbox"), "{refusal}");
}

/// A record's inherited confinement, changed by one setter.
type Inherit = fn(&mut InheritedConfinement);

#[test]
fn an_inherited_confinement_is_refused_with_its_reason() {
    let cases: [(&str, Inherit, &str); 4] = [
        ("w", |i| i.workflow = true, "inside a workflow run"),
        (
            "s",
            |i| i.sealed_repositories = vec!["/repo/.git".into()],
            "sealed repositories (/repo/.git)",
        ),
        (
            "d",
            |i| i.denied_directory_names = vec!["runs".into()],
            "denied directory names (runs)",
        ),
        (
            "p",
            |i| i.parent_subagent = Some("lead".into()),
            "by subagent 'lead'",
        ),
    ];
    for (agent, set, why) in cases {
        let (store, _tmp) = store_with_transcript(agent);
        let mut record = boundary();
        set(&mut record.inherited);
        store.write_metadata(agent, &metadata(Some(record)));
        let refusal = plan(&store, agent).unwrap_err();
        assert!(refusal.contains(&format!("'{agent}'")), "{refusal}");
        assert!(refusal.contains(why), "{refusal}");
        assert!(refusal.contains("main session"), "{refusal}");
    }
}

#[test]
fn no_transcript_means_nothing_to_resume() {
    let tmp = TempDir::new().unwrap();
    let store = AgentTranscriptStore::with_base_dir(tmp.path().to_path_buf());
    store.write_metadata("none", &metadata(Some(boundary())));
    assert!(plan_resume(&store, "none", "x").is_none());
}

#[test]
fn every_missing_record_field_refuses_the_resume() {
    let full = serde_json::to_value(metadata(Some(boundary()))).unwrap();
    let fields: Vec<String> = full["confinement"]
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect();
    assert_eq!(fields.len(), 10, "{fields:?}");
    let mut failures = Vec::new();
    for field in fields {
        let agent = format!("missing-{field}");
        let (store, _tmp) = store_with_transcript(&agent);
        let mut meta = full.clone();
        meta["confinement"].as_object_mut().unwrap().remove(&field);
        std::fs::write(store.metadata_path(&agent), meta.to_string()).unwrap();

        match plan(&store, &agent) {
            Ok(plan) => failures.push(format!(
                "without `{field}` the agent resumed with isolation {:?}",
                plan.request.isolation
            )),
            Err(refusal)
                if !refusal.contains(&format!("'{agent}'"))
                    || !refusal.contains(&format!("`{field}`")) =>
            {
                failures.push(format!("the refusal does not name `{field}`: {refusal}"))
            }
            Err(_) => {}
        }
    }
    for field in [
        "workflow",
        "sealed_repositories",
        "denied_directory_names",
        "parent_subagent",
    ] {
        let agent = format!("missing-inherited-{field}");
        let (store, _tmp) = store_with_transcript(&agent);
        let mut meta = full.clone();
        meta["confinement"]["inherited"]
            .as_object_mut()
            .unwrap()
            .remove(field);
        std::fs::write(store.metadata_path(&agent), meta.to_string()).unwrap();
        match plan(&store, &agent) {
            Ok(_) => failures.push(format!("without `inherited.{field}` the agent resumed")),
            Err(refusal) if !refusal.contains(&format!("`{field}`")) => {
                failures.push(format!("the refusal does not name `{field}`: {refusal}"))
            }
            Err(_) => {}
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

#[test]
fn a_null_isolation_is_refused_not_read_as_unset() {
    let (store, _tmp) = store_with_transcript("null-1");
    let mut meta = serde_json::to_value(metadata(Some(boundary()))).unwrap();
    meta["confinement"]["isolation"] = serde_json::Value::Null;
    std::fs::write(store.metadata_path("null-1"), meta.to_string()).unwrap();
    let refusal = plan(&store, "null-1").expect_err("a null isolation must not resume");
    assert!(refusal.contains("'null-1'"), "{refusal}");
    assert!(refusal.contains("isolation"), "{refusal}");
}

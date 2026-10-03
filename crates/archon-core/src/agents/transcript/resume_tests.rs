//! #241: a resume sends the spawn's confinement again, and refuses an agent
//! whose metadata does not record it.

use super::*;
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

fn confinement(isolation: Option<&str>, tier: &str) -> SpawnConfinement {
    SpawnConfinement {
        isolation: isolation.map(str::to_string),
        tier: tier.into(),
        cwd: "/work/space".into(),
        read_roots: vec!["/project/spec.md".into()],
        write_roots: vec!["/project/out".into()],
        allowed_tools: vec!["Read".into(), "Write".into()],
        model: Some("model-a".into()),
        max_turns: 7,
        timeout_secs: 90,
    }
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
    store.write_metadata(
        "a1",
        &metadata(Some(confinement(Some("workspace-boundary"), "none"))),
    );

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
}

#[test]
fn a_worktree_rung_is_pinned_so_the_resume_reuses_its_checkout() {
    let (store, _tmp) = store_with_transcript("a2");
    // Reached by the automatic policy: the request named no isolation.
    store.write_metadata("a2", &metadata(Some(confinement(None, "worktree"))));
    assert_eq!(
        plan(&store, "a2").unwrap().request.isolation.as_deref(),
        Some("worktree")
    );
}

#[test]
fn a_shared_rung_sends_what_the_spawn_asked_for() {
    let (store, _tmp) = store_with_transcript("a3");
    let shared = archon_tools::isolation::IsolationTier::Shared.as_str();
    store.write_metadata("a3", &metadata(Some(confinement(None, shared))));
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
fn a_record_missing_a_field_is_refused_not_defaulted() {
    let (store, _tmp) = store_with_transcript("bad-1");
    // `cwd` and the roots are gone: the record must not parse into defaults.
    std::fs::write(
        store.metadata_path("bad-1"),
        r#"{"agent_type": "explore", "confinement": {"isolation": "workspace-boundary",
            "tier": "none", "allowed_tools": [], "model": null, "max_turns": 1,
            "timeout_secs": 1}}"#,
    )
    .unwrap();
    assert!(plan(&store, "bad-1").is_err());
}

#[test]
fn an_unknown_recorded_isolation_is_refused() {
    let (store, _tmp) = store_with_transcript("bad-2");
    store.write_metadata(
        "bad-2",
        &metadata(Some(confinement(Some("sealed-ish"), "none"))),
    );
    let refusal = plan(&store, "bad-2").unwrap_err();
    assert!(refusal.contains("'sealed-ish'"), "{refusal}");
    assert!(refusal.contains("'bad-2'"), "{refusal}");
}

#[test]
fn no_transcript_means_nothing_to_resume() {
    let tmp = TempDir::new().unwrap();
    let store = AgentTranscriptStore::with_base_dir(tmp.path().to_path_buf());
    store.write_metadata("none", &metadata(Some(confinement(None, "none"))));
    assert!(plan_resume(&store, "none", "x").is_none());
}

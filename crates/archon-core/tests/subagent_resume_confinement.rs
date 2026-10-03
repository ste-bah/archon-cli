//! #241, end to end: an agent resumed from its transcript keeps the
//! confinement it was spawned with. The resume used to rebuild its request
//! without `isolation`, `cwd` or the roots, so a `workspace-boundary` agent
//! came back unconfined. These tests spawn through the real executor and the
//! real `Read` and `Write` tools, resume the agent from the metadata the spawn
//! wrote, and read every target again from outside the agent.

#[path = "support/boundary_harness.rs"]
mod harness;
use harness::*;

use archon_core::agents::transcript::{AgentTranscriptStore, plan_resume};

const BOUNDARY: &str = "workspace-boundary";

/// A session of its own, so the transcripts the executor writes under the
/// home directory belong to this test alone. Removed when dropped.
struct Session(String);

impl Session {
    fn new() -> Self {
        Self(format!("resume-confinement-{}", uuid::Uuid::new_v4()))
    }

    fn store(&self) -> AgentTranscriptStore {
        AgentTranscriptStore::new(&self.0).expect("a home directory")
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        if let Some(home) = dirs::home_dir() {
            let _ = std::fs::remove_dir_all(home.join(".archon/sessions").join(&self.0));
        }
    }
}

/// Remove the confinement record, as metadata written before it existed.
fn strip_record(store: &AgentTranscriptStore, agent_id: &str) {
    let path = store.metadata_path(agent_id);
    let mut meta: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert!(
        meta.get("confinement").is_some(),
        "the spawn wrote no record"
    );
    meta.as_object_mut().unwrap().remove("confinement");
    std::fs::write(&path, meta.to_string()).unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_resumed_boundary_agent_keeps_its_cwd_roots_and_boundary() {
    let (_t, root) = real_temp();
    let project = dir(&root, "project");
    let workspace = dir(&root, "workspace");
    let prd = dir(&project, "prds").join("spec.md");
    std::fs::write(&prd, "the requirements\n").unwrap();
    let unnamed = project.join("notes.md");
    std::fs::write(&unnamed, "not named\n").unwrap();
    let outside = project.join("outside.txt");
    let first = workspace.join("first.txt");
    let second = workspace.join("second.txt");
    let session = Session::new();
    let host = Host::new(
        &project,
        &session.0,
        vec![
            write(&first, "first\n"),
            STOP,
            write(&outside, "changed\n"),
            write(&second, "second\n"),
            read(&prd),
            read(&unnamed),
        ],
    );
    let agent = "bounded-agent";
    let read_roots = vec![prd.display().to_string()];

    host.spawn(
        agent,
        request(&workspace, Some(BOUNDARY), read_roots.clone()),
        parent(&project, &[]),
    )
    .await
    .expect("the spawn runs");
    assert!(!host.outcome(0).is_error, "{:?}", host.outcome(0));

    let plan = plan_resume(&session.store(), agent, "continue")
        .expect("a transcript")
        .expect("a recorded spawn resumes");
    assert_eq!(plan.request.isolation.as_deref(), Some(BOUNDARY));
    assert_eq!(plan.request.cwd, Some(workspace.display().to_string()));
    assert_eq!(plan.request.read_roots, read_roots);
    assert!(!plan.messages.is_empty());

    // The main agent resumes from its own world: the project directory.
    host.pending
        .lock()
        .await
        .insert(agent.to_string(), plan.messages);
    host.spawn(agent, plan.request, parent(&project, &[]))
        .await
        .expect("the resume runs");

    let refused = host.outcome(2);
    assert!(
        refused.is_error && refused.text.contains("outside"),
        "the resumed agent wrote outside its workspace: {refused:?}"
    );
    // A separate read, not the tool's word: the target is unchanged.
    assert!(!outside.exists(), "the outside write landed");
    assert!(!host.outcome(3).is_error, "{:?}", host.outcome(3));
    assert_eq!(std::fs::read_to_string(&second).unwrap(), "second\n");
    let named = host.outcome(4);
    assert!(
        !named.is_error && named.text.contains("the requirements"),
        "the read root was not restored: {named:?}"
    );
    let other = host.outcome(5);
    assert!(
        other.is_error && !other.text.contains("not named"),
        "the resumed agent reads beyond its roots: {other:?}"
    );
    // The resume rewrote the metadata with the same record.
    let again = session.store().read_metadata(agent).unwrap();
    let record = again.confinement.expect("record kept");
    assert_eq!(record.isolation.as_deref(), Some(BOUNDARY));
    assert_eq!(record.cwd, workspace.display().to_string());
    assert_eq!(record.read_roots, read_roots);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_resumed_unbounded_agent_resumes_as_before() {
    let (_t, root) = real_temp();
    let project = dir(&root, "project");
    let workspace = dir(&root, "workspace");
    let outside = project.join("outside.txt");
    let session = Session::new();
    let host = Host::new(
        &project,
        &session.0,
        vec![STOP, write(&outside, "changed\n")],
    );
    let agent = "unbounded-agent";

    host.spawn(
        agent,
        request(&workspace, None, Vec::new()),
        parent(&project, &[]),
    )
    .await
    .expect("the spawn runs");
    let plan = plan_resume(&session.store(), agent, "continue")
        .unwrap()
        .expect("an unbounded agent resumes");
    assert_eq!(plan.request.isolation, None);
    assert_eq!(plan.request.cwd, Some(workspace.display().to_string()));
    host.pending
        .lock()
        .await
        .insert(agent.to_string(), plan.messages);
    host.spawn(agent, plan.request, parent(&project, &[]))
        .await
        .expect("the resume runs");

    // Unconfined, as it was at spawn: it inherits the parent's directory.
    assert!(!host.outcome(1).is_error, "{:?}", host.outcome(1));
    assert_eq!(std::fs::read_to_string(&outside).unwrap(), "changed\n");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn old_metadata_without_the_record_refuses_the_resume() {
    let (_t, root) = real_temp();
    let project = dir(&root, "project");
    let workspace = dir(&root, "workspace");
    let session = Session::new();
    let host = Host::new(&project, &session.0, vec![STOP, STOP]);
    let store = session.store();

    for (agent, isolation) in [("old-bounded", Some(BOUNDARY)), ("old-unbounded", None)] {
        host.spawn(
            agent,
            request(&workspace, isolation, Vec::new()),
            parent(&project, &[]),
        )
        .await
        .expect("the spawn runs");
        strip_record(&store, agent);

        let refusal = plan_resume(&store, agent, "continue")
            .expect("a transcript")
            .expect_err("an unrecorded spawn must not resume unconfined");
        assert!(refusal.contains(&format!("'{agent}'")), "{refusal}");
        assert!(
            refusal.contains("isolation, tier, cwd, read_roots, write_roots"),
            "{refusal}"
        );
        assert!(refusal.contains("spawn a new agent"), "{refusal}");
    }
}

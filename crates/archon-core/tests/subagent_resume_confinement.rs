//! Boundary and unconfined agents both use process memory on resume.
#[path = "support/boundary_harness.rs"]
mod harness;
use archon_core::agents::transcript::AgentTranscriptStore;
use harness::*;

#[tokio::test]
async fn a_resumed_boundary_agent_keeps_its_cwd_roots_and_boundary() {
    let (_t, root) = real_temp();
    let project = dir(&root, "project");
    let workspace = dir(&root, "workspace");
    let target = project.join("read.txt");
    std::fs::write(&target, "requirements").unwrap();
    let unnamed = project.join("unnamed.txt");
    std::fs::write(&unnamed, "private").unwrap();
    let inside = workspace.join("inside.txt");
    let host = Host::new(
        &project,
        "resume-boundary",
        vec![
            STOP,
            write(&target, "changed"),
            write(&inside, "inside"),
            read(&target),
            read(&unnamed),
            STOP,
        ],
    );
    host.spawn(
        "bounded",
        request(
            &workspace,
            Some("workspace-boundary"),
            vec![target.display().to_string()],
        ),
        parent(&project, &[]),
    )
    .await
    .unwrap();
    let store = AgentTranscriptStore::with_base_dir(root.join("history"));
    store.record_message(
        "bounded",
        &serde_json::json!({"role":"assistant","content":"original history"}),
    );
    let plan = host.plan(&store, "bounded").await.unwrap();
    host.resume("bounded", plan, parent(&project, &[&project]))
        .await
        .unwrap();
    assert!(host.outcome(1).is_error);
    assert!(!host.outcome(2).is_error);
    assert!(!host.outcome(3).is_error);
    assert!(host.outcome(4).is_error);
    assert_eq!(std::fs::read_to_string(target).unwrap(), "requirements");
    assert_eq!(std::fs::read_to_string(inside).unwrap(), "inside");
}

#[tokio::test]
async fn a_resumed_unbounded_agent_keeps_its_inherited_directories() {
    let (_t, root) = real_temp();
    let project = dir(&root, "project");
    let workspace = dir(&root, "workspace");
    let target = project.join("outside.txt");
    let host = Host::new(
        &project,
        "resume-unbounded",
        vec![STOP, write(&target, "changed"), STOP],
    );
    host.spawn(
        "unbounded",
        request(&workspace, None, vec![]),
        parent(&project, &[]),
    )
    .await
    .unwrap();
    let store = AgentTranscriptStore::with_base_dir(root.join("history"));
    store.record_message(
        "unbounded",
        &serde_json::json!({"role":"assistant","content":"done"}),
    );
    let plan = host.plan(&store, "unbounded").await.unwrap();
    host.resume("unbounded", plan, parent(&workspace, &[]))
        .await
        .unwrap();
    assert!(!host.outcome(1).is_error);
    assert_eq!(std::fs::read_to_string(target).unwrap(), "changed");
}

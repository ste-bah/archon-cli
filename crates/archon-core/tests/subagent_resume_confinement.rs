//! A workflow's validation repair of a boundary agent and of an unconfined
//! agent both run under the stored context, not the call passed again.
#[path = "support/boundary_harness.rs"]
mod harness;
#[path = "support/resume_memory_harness.rs"]
mod memory_harness;
use harness::*;
use memory_harness::*;

#[tokio::test]
async fn a_repaired_boundary_agent_keeps_its_cwd_roots_and_boundary() {
    let (_t, root) = temp();
    let project = dir(&root, "project");
    let workspace = dir(&root, "workspace");
    let target = project.join("read.txt");
    std::fs::write(&target, "requirements").unwrap();
    let unnamed = project.join("unnamed.txt");
    std::fs::write(&unnamed, "private").unwrap();
    let inside = workspace.join("inside.txt");
    let host = Host::new(
        &project,
        "repair-boundary",
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
    // Passed again without the boundary and with a wider parent: neither is used.
    host.repair(
        "bounded",
        request(&workspace, None, vec![]),
        parent(&project, &[&project]),
    )
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
async fn a_repaired_unbounded_agent_keeps_its_inherited_directories() {
    let (_t, root) = temp();
    let project = dir(&root, "project");
    let workspace = dir(&root, "workspace");
    let target = project.join("outside.txt");
    let host = Host::new(
        &project,
        "repair-unbounded",
        vec![STOP, write(&target, "changed"), STOP],
    );
    host.spawn(
        "unbounded",
        request(&workspace, None, vec![]),
        parent(&project, &[]),
    )
    .await
    .unwrap();
    host.repair(
        "unbounded",
        request(&workspace, None, vec![]),
        parent(&workspace, &[]),
    )
    .await
    .unwrap();
    assert!(!host.outcome(1).is_error);
    assert_eq!(std::fs::read_to_string(target).unwrap(), "changed");
}

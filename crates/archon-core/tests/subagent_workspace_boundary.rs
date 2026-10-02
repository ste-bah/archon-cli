//! #236, end to end: an agent spawned with isolation `workspace-boundary` is
//! confined to its working directory. The adapter sent that value on every
//! workspace-bounded workflow call, but the executor never applied it. These
//! tests drive the real executor and the real `Read` and `Write` tools. After
//! the run they read every file again from outside the agent, so a refusal
//! that still changed the file fails the test. Each confinement test also
//! has a control run without the boundary. The control shows that the same
//! access works there, so the boundary is what refuses it.

#[path = "support/boundary_harness.rs"]
mod harness;
use harness::*;

const BOUNDARY: &str = "workspace-boundary";

/// The project directory's entries and their contents, read from outside the
/// agent.
fn listing(dir: &std::path::Path) -> Vec<(String, String)> {
    let mut entries = std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| {
            let path = entry.unwrap().path();
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            (name, std::fs::read_to_string(&path).unwrap_or_default())
        })
        .collect::<Vec<_>>();
    entries.sort();
    entries
}

// The outside target is a NEW file. A write to an existing file the agent
// has not read is refused for that reason alone, and that refusal would
// hide whether the boundary did anything.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_bounded_agent_cannot_write_outside_its_workspace_and_writes_inside() {
    let (_t, root) = real_temp();
    let project = dir(&root, "project");
    let workspace = dir(&root, "workspace");
    std::fs::write(project.join("existing.txt"), "original\n").unwrap();
    let before = listing(&project);
    let outside = project.join("outside.txt");
    let inside = workspace.join("inside.txt");

    let outcomes = run(Spawn {
        parent: parent(&project, &[]),
        cwd: &workspace,
        isolation: Some(BOUNDARY),
        read_roots: Vec::new(),
        calls: vec![
            write(&outside, "changed\n"),
            write(&inside, "inside\n"),
            read(&inside),
        ],
    })
    .await
    .expect("the spawn runs");

    assert!(
        outcomes[0].is_error && outcomes[0].text.contains("outside"),
        "outside write not refused by the boundary: {outcomes:?}"
    );
    // A separate read, not the tool's word: the target is unchanged.
    assert!(!outside.exists(), "the outside write landed");
    assert_eq!(listing(&project), before);
    assert!(!outcomes[1].is_error, "inside write refused: {outcomes:?}");
    assert_eq!(std::fs::read_to_string(&inside).unwrap(), "inside\n");
    assert!(
        !outcomes[2].is_error && outcomes[2].text.contains("inside"),
        "the agent cannot read back its own write: {outcomes:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn control_without_the_boundary_the_same_outside_write_lands() {
    let (_t, root) = real_temp();
    let project = dir(&root, "project");
    let workspace = dir(&root, "workspace");
    let outside = project.join("outside.txt");

    let outcomes = run(Spawn {
        parent: parent(&project, &[]),
        cwd: &workspace,
        isolation: None,
        read_roots: Vec::new(),
        calls: vec![write(&outside, "changed\n")],
    })
    .await
    .expect("the spawn runs");

    assert!(!outcomes[0].is_error, "{outcomes:?}");
    assert_eq!(std::fs::read_to_string(&outside).unwrap(), "changed\n");
}

/// The repository-audit assessor's shape: it runs in a sealed snapshot, and
/// its parent can read the live repository.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_bounded_assessor_cannot_read_outside_its_snapshot() {
    let (_t, root) = real_temp();
    let project = dir(&root, "project");
    let repository = dir(&root, "repository");
    let snapshot = dir(&root, "snapshot");
    std::fs::write(repository.join("live.rs"), "live tree\n").unwrap();
    std::fs::write(snapshot.join("sealed.rs"), "sealed tree\n").unwrap();
    let calls = || {
        vec![
            read(&repository.join("live.rs")),
            read(&snapshot.join("sealed.rs")),
        ]
    };

    let bounded = run(Spawn {
        parent: parent(&project, &[&repository]),
        cwd: &snapshot,
        isolation: Some(BOUNDARY),
        read_roots: Vec::new(),
        calls: calls(),
    })
    .await
    .expect("the spawn runs");
    assert!(bounded[0].is_error, "live tree readable: {bounded:?}");
    assert!(!bounded[0].text.contains("live tree"), "{bounded:?}");
    assert!(
        !bounded[1].is_error && bounded[1].text.contains("sealed tree"),
        "{bounded:?}"
    );

    let control = run(Spawn {
        parent: parent(&project, &[&repository]),
        cwd: &snapshot,
        isolation: None,
        read_roots: Vec::new(),
        calls: calls(),
    })
    .await
    .expect("the spawn runs");
    assert!(
        !control[0].is_error && control[0].text.contains("live tree"),
        "{control:?}"
    );
}

/// The acceptance re-author's shape: it runs in the repository, and its
/// prompt sends it to a PRD in the project root.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_bounded_reauthor_reads_its_named_prd_and_nothing_else_outside() {
    let (_t, root) = real_temp();
    let project = dir(&root, "project");
    let repository = dir(&root, "repository");
    let prd = dir(&project, "prds").join("spec.md");
    std::fs::write(&prd, "the requirements\n").unwrap();
    let unnamed = project.join("notes.md");
    std::fs::write(&unnamed, "not named\n").unwrap();

    let outcomes = run(Spawn {
        parent: parent(&project, &[&repository]),
        cwd: &repository,
        isolation: Some(BOUNDARY),
        read_roots: vec![prd.display().to_string()],
        calls: vec![read(&prd), read(&unnamed), write(&prd, "rewritten\n")],
    })
    .await
    .expect("the spawn runs");

    assert!(
        !outcomes[0].is_error && outcomes[0].text.contains("the requirements"),
        "PRD refused: {outcomes:?}"
    );
    assert!(
        outcomes[1].is_error && !outcomes[1].text.contains("not named"),
        "unnamed file readable: {outcomes:?}"
    );
    // The PRD was read first, so only the boundary can refuse this write.
    assert!(
        outcomes[2].is_error && outcomes[2].text.contains("outside"),
        "read root writable: {outcomes:?}"
    );
    assert_eq!(std::fs::read_to_string(&prd).unwrap(), "the requirements\n");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unknown_isolation_value_fails_the_spawn_naming_value_and_source() {
    let (_t, root) = real_temp();
    let workspace = dir(&root, "workspace");
    let marker = workspace.join("never.txt");

    let error = run(Spawn {
        parent: parent(&workspace, &[]),
        cwd: &workspace,
        isolation: Some("sealed-ish"),
        read_roots: Vec::new(),
        calls: vec![write(&marker, "x\n")],
    })
    .await
    .expect_err("an unknown value must fail the spawn");

    let text = error.to_string();
    assert!(text.contains("'sealed-ish'"), "{text}");
    assert!(
        text.contains("the spawn request's isolation field"),
        "{text}"
    );
    assert!(!marker.exists(), "the agent ran despite the refusal");
}

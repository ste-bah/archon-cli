//! Issue-213 C3, end to end: an agent a workflow places in a linked worktree
//! cannot write the repository's other checkouts through its file tools —
//! not by naming them, not through a link, and not by spawning a child
//! somewhere else. Driven through the real executor, the real `Write` tool and
//! a provider that asks for the writes; the canonical checkout is read back
//! afterwards. An interactive agent keeps the access it always had.

#[path = "support/isolated_write_harness.rs"]
mod harness;
use harness::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_write_agent_in_a_worktree_cannot_modify_the_canonical_checkout() {
    let (_t, root) = real_temp();
    let (canonical, worktree) = checkout_and_worktree(&root);
    let escaped = canonical.join("escaped.txt");
    let own = worktree.join("made-here.txt");
    run_child(
        &canonical,
        workflow_parent(&canonical),
        Some(&worktree),
        None,
        &[(&escaped, "x\n"), (&own, "y\n")],
    )
    .await;
    assert!(!escaped.exists(), "the agent wrote the canonical checkout");
    assert_unchanged(&canonical);
    // Not simply stopped from writing: its own workspace took the write.
    assert_eq!(std::fs::read_to_string(&own).expect("own write"), "y\n");
}

/// Decision: an interactive subagent is untouched by the seal.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_interactive_agent_in_a_worktree_keeps_its_access() {
    let (_t, root) = real_temp();
    let (canonical, worktree) = checkout_and_worktree(&root);
    let written = canonical.join("interactive.txt");
    let parent = archon_tools::tool::ToolContext {
        working_dir: canonical.clone(),
        ..archon_tools::tool::ToolContext::default()
    };
    run_child(
        &canonical,
        parent,
        Some(&worktree),
        None,
        &[(&written, "x\n")],
    )
    .await;
    assert_eq!(std::fs::read_to_string(&written).expect("written"), "x\n");
}

/// A child of an isolated agent inherits its seal: naming the canonical
/// checkout as its directory, or naming none, places it in its parent's
/// workspace, and the canonical checkout stays unwritten.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_nested_child_cannot_escape_its_parents_seal() {
    let (_t, root) = real_temp();
    let (canonical, worktree) = checkout_and_worktree(&root);
    let repository = archon_tools::spawn_placement::owning_checkout(&worktree)
        .expect("checkout")
        .repository;
    let isolated = || archon_tools::tool::ToolContext {
        sealed_repositories: vec![repository.clone()],
        ..workflow_parent(&worktree)
    };
    for (case, cwd) in [
        ("canonical cwd", Some(canonical.as_path())),
        ("no cwd", None),
    ] {
        let escaped = canonical.join(format!("{}.txt", case.replace(' ', "-")));
        let own = worktree.join(format!("{}.txt", case.replace(' ', "-")));
        // Relative to where the child is placed: its parent's workspace.
        run_child(
            &canonical,
            isolated(),
            cwd,
            None,
            &[(&escaped, "x\n"), (&own, "y\n")],
        )
        .await;
        assert!(
            !escaped.exists(),
            "{case}: the child wrote the canonical checkout"
        );
        assert_eq!(std::fs::read_to_string(&own).expect(case), "y\n", "{case}");
    }
    assert_unchanged(&canonical);
}

/// A link in the workspace pointing into the canonical checkout is judged by
/// where it lands.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_link_out_of_the_workspace_does_not_reach_the_canonical_checkout() {
    let (_t, root) = real_temp();
    let (canonical, worktree) = checkout_and_worktree(&root);
    std::fs::create_dir_all(canonical.join("src")).expect("src");
    std::os::unix::fs::symlink(canonical.join("src"), worktree.join("x")).expect("link");
    let through = worktree.join("x/lib.rs");
    run_child(
        &canonical,
        workflow_parent(&canonical),
        Some(&worktree),
        None,
        &[(&through, "x\n")],
    )
    .await;
    assert!(
        !canonical.join("src/lib.rs").exists(),
        "written through the link"
    );
    assert_unchanged(&canonical);
}

/// The run store kept inside the canonical checkout stays the run-store
/// guard's to judge: a report in the run's artifact directory is written.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_run_store_inside_the_checkout_stays_writable() {
    let (_t, root) = real_temp();
    let (canonical, worktree) = checkout_and_worktree(&root);
    let store = canonical.join(".store");
    let run = store.join("run");
    let report = run.join("artifacts/report.md");
    std::fs::create_dir_all(report.parent().unwrap()).expect("artifacts");
    let parent = archon_tools::tool::ToolContext {
        run_store: Some(archon_tools::workflow_read_guard::RunStoreScope::new(
            run.to_str(),
            store.to_str(),
            worktree.to_str(),
        )),
        ..workflow_parent(&canonical)
    };
    let escaped = canonical.join("escaped.txt");
    run_child(
        &canonical,
        parent,
        Some(&worktree),
        None,
        &[(&report, "r\n"), (&escaped, "x\n")],
    )
    .await;
    assert_eq!(std::fs::read_to_string(&report).expect("report"), "r\n");
    assert!(!escaped.exists());
}

/// A path the host declared writable (a project artifact it judges where it
/// is) is written in the sealed checkout; anything else there is not.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_declared_artifact_in_the_checkout_stays_writable() {
    use archon_tools::workflow_read_guard::{
        DeclaredTargetScope, HostWriteBoundary, WorkflowReadGuard,
    };
    let (_t, root) = real_temp();
    let (canonical, worktree) = checkout_and_worktree(&root);
    let declared = canonical.join("data");
    std::fs::create_dir_all(&declared).expect("data");
    let boundary = HostWriteBoundary::new(
        &[canonical.display().to_string()],
        &[declared.display().to_string()],
    );
    let guard = WorkflowReadGuard::new(40, 20, false, false).with_declared_targets(
        DeclaredTargetScope::new(&["made-here.txt".to_string()], worktree.to_str())
            .in_isolated_worktree(true)
            .with_write_boundary(boundary),
    );
    let parent = archon_tools::tool::ToolContext {
        workflow_read_guard: Some(std::sync::Arc::new(guard)),
        ..workflow_parent(&canonical)
    };
    let artifact = declared.join("a.json");
    let escaped = canonical.join("escaped.txt");
    run_child(
        &canonical,
        parent,
        Some(&worktree),
        None,
        &[(&artifact, "{}\n"), (&escaped, "x\n")],
    )
    .await;
    assert_eq!(
        std::fs::read_to_string(&artifact).expect("artifact"),
        "{}\n"
    );
    assert!(!escaped.exists());
}

/// Issue-213 C3 (review): a DANGLING link in the workspace into the canonical
/// checkout (as `ln -s` from a shell makes one). A write through it must not
/// create the file it names.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_dangling_link_out_of_the_workspace_creates_nothing_in_the_canonical_checkout() {
    let (_t, root) = real_temp();
    let (canonical, worktree) = checkout_and_worktree(&root);
    std::fs::create_dir_all(canonical.join("src")).expect("src");
    let target = canonical.join("src/evil.rs");
    let link = worktree.join("evil.rs");
    std::os::unix::fs::symlink(&target, &link).expect("link");
    run_child(
        &canonical,
        workflow_parent(&canonical),
        Some(&worktree),
        None,
        &[(&link, "x\n")],
    )
    .await;
    assert!(!target.exists(), "written through the dangling link");
    assert_unchanged(&canonical);
}

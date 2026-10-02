//! Issue-213 C3, end to end: a worktree the SPAWN makes (`isolation:
//! "worktree"`) seals the checkout it was made from. Its own test binary,
//! because the executor puts such worktrees under the user's data directory
//! and this test points that at a temporary one through the environment.

#[path = "support/isolated_write_harness.rs"]
mod harness;
use harness::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_spawn_made_worktree_cannot_write_the_tree_it_came_from() {
    let (_t, root) = real_temp();
    let home = root.join("home");
    std::fs::create_dir_all(&home).expect("home");
    // SAFETY: this binary runs this one test; nothing else reads the
    // environment concurrently.
    unsafe {
        std::env::set_var("HOME", &home);
        std::env::set_var("XDG_DATA_HOME", home.join("data"));
        // HOME and XDG_DATA_HOME redirect nothing on Windows, where the data
        // directory comes from the Known Folder API; the documented override
        // does, on every platform.
        std::env::set_var("ARCHON_DATA_DIR", home.join("data").join("archon"));
    }
    let (canonical, _sibling) = checkout_and_worktree(&root);
    let escaped = canonical.join("escaped.txt");
    run_child(
        &canonical,
        workflow_parent(&canonical),
        Some(&canonical),
        Some("worktree"),
        &[(&escaped, "x\n")],
    )
    .await;
    assert!(
        !escaped.exists(),
        "the spawn-made worktree wrote its source"
    );
    assert_unchanged(&canonical);
    assert!(
        std::fs::read_dir(&home).expect("home").next().is_some(),
        "the worktree was made under the temporary data directory"
    );
}

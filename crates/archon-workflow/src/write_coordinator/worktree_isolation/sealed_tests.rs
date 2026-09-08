use super::tests::{canonical_repo, default_cfg, git, plan_for};
use super::*;

#[test]
fn sealed_source_does_not_follow_live_head_or_dirty_edits() {
    let repo = canonical_repo();
    let root = repo.path();
    std::fs::write(root.join("src/lib.rs"), "captured dirty\n").unwrap();
    let plan = plan_for(root, &["src/lib.rs"]);
    let sealed = capture_sealed_source(root, &plan, &default_cfg()).unwrap();
    std::fs::write(root.join("src/lib.rs"), "later operator edit\n").unwrap();
    git(&["add", "src/lib.rs"], root);
    git(&["commit", "-qm", "operator advances HEAD"], root);
    let workspace = create_item_workspace_from_sealed(root, &plan, &sealed).unwrap();
    assert_eq!(std::fs::read_to_string(workspace.plan.isolated_root.join("src/lib.rs")).unwrap(), "captured dirty\n");
    assert_eq!(std::fs::read_to_string(root.join("src/lib.rs")).unwrap(), "later operator edit\n");
}

#[test]
fn two_branches_share_captured_untracked_bytes_without_live_reread() {
    let repo = canonical_repo();
    let root = repo.path();
    std::fs::write(root.join("src/new.txt"), "shared source\n").unwrap();
    let mut plan = plan_for(root, &["src/lib.rs", "src/new.txt"]);
    let sealed = capture_sealed_source(root, &plan, &default_cfg()).unwrap();
    let first = create_item_workspace_from_sealed(root, &plan, &sealed).unwrap();
    std::fs::write(root.join("src/new.txt"), "later\n").unwrap();
    plan.item_id = "second".into();
    plan.isolated_root = root.join(".archon/wc/run1/second");
    let second = create_item_workspace_from_sealed(root, &plan, &sealed).unwrap();
    for workspace in [&first, &second] {
        assert_eq!(std::fs::read_to_string(workspace.plan.isolated_root.join("src/new.txt")).unwrap(), "shared source\n");
    }
    assert_eq!(run_git(&["rev-parse", "HEAD^{tree}"], &first.plan.isolated_root).unwrap().stdout,
        run_git(&["rev-parse", "HEAD^{tree}"], &second.plan.isolated_root).unwrap().stdout);
}

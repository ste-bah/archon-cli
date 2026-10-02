use super::*;

fn dirs() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = std::fs::canonicalize(temp.path()).expect("real temp");
    let canonical = root.join("canonical");
    let store = canonical.join(".store");
    let workspace = store.join("run/worktrees/mine");
    std::fs::create_dir_all(&workspace).expect("workspace");
    std::fs::create_dir_all(store.join("run/worktrees/sibling")).expect("sibling");
    (temp, canonical, workspace)
}

fn ctx(canonical: &Path, workspace: &Path) -> ToolContext {
    ToolContext {
        working_dir: workspace.to_path_buf(),
        sealed_roots: vec![
            canonical.to_path_buf(),
            canonical.join(".store/run/worktrees/sibling"),
        ],
        ..ToolContext::default()
    }
}

#[test]
fn the_isolated_checkout_is_refused_and_the_workspace_is_not() {
    let (_t, canonical, workspace) = dirs();
    let ctx = ctx(&canonical, &workspace);
    let refused = ensure_not_sealed(&canonical.join("src/lib.rs"), &ctx).unwrap_err();
    assert!(refused.contains("isolated from"), "{refused}");
    assert!(ensure_not_sealed(&workspace.join("src/lib.rs"), &ctx).is_ok());
    assert!(
        ensure_not_sealed(&canonical.join(".store/run/worktrees/sibling/a"), &ctx).is_err(),
        "another branch's worktree is sealed too"
    );
    assert!(ensure_not_sealed(Path::new("/elsewhere/a"), &ctx).is_ok());
    assert!(ensure_not_sealed(&canonical.join("a"), &ToolContext::default()).is_ok());
}

#[test]
fn a_run_store_inside_the_checkout_is_left_to_the_run_store_guard() {
    let (_t, canonical, workspace) = dirs();
    let mut ctx = ctx(&canonical, &workspace);
    let run = canonical.join(".store/run");
    ctx.run_store = Some(crate::workflow_read_guard::RunStoreScope::new(
        run.to_str(),
        canonical.join(".store").to_str(),
        workspace.to_str(),
    ));
    assert!(ensure_not_sealed(&run.join("artifacts/report.md"), &ctx).is_ok());
    assert!(
        ensure_not_sealed(&canonical.join(".store/run/worktrees/sibling/a"), &ctx).is_err(),
        "the most specific seal decides"
    );
    assert!(ensure_not_sealed(&canonical.join("src/lib.rs"), &ctx).is_err());
}

#[test]
fn a_write_root_inside_the_checkout_passes_and_one_covering_it_does_not() {
    let (_t, canonical, workspace) = dirs();
    let mut ctx = ctx(&canonical, &workspace);
    ctx.write_roots = vec![canonical.join("registry")];
    assert!(ensure_not_sealed(&canonical.join("registry/a.json"), &ctx).is_ok());
    ctx.write_roots = vec![canonical.clone()];
    assert!(ensure_not_sealed(&canonical.join("registry/a.json"), &ctx).is_err());
}

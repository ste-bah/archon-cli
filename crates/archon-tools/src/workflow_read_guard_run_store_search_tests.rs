//! Obs-123: a recursive shell search that would walk the run store, or
//! leaves the call's workspace, is refused with a pointer to the workspace;
//! searches inside it, of the agent's own trees and of scratch pass.
use super::RunStoreScope;
use serde_json::json;

struct World {
    _temp: tempfile::TempDir,
    project: String,
    store: String,
    worktree: String,
}

/// A project whose run store sits inside it, and a branch worktree the host
/// planted in the current run.
fn world() -> World {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("project");
    let store = project.join(".host/runs");
    let worktree = store.join("run-1/v2/worktrees/call-1/item-1");
    std::fs::create_dir_all(&worktree).unwrap();
    std::fs::create_dir_all(store.join("run-0/v2/results")).unwrap();
    let text = |p: &std::path::Path| p.display().to_string();
    World {
        project: text(&project),
        store: text(&store),
        worktree: text(&worktree),
        _temp: temp,
    }
}

fn scope(w: &World, working_root: &str) -> RunStoreScope {
    RunStoreScope::new(
        Some(&format!("{}/run-1", w.store)),
        Some(&w.store),
        Some(working_root),
    )
}

fn bash(scope: &RunStoreScope, command: &str) -> Option<String> {
    scope.refusal("Bash", &json!({ "command": command }))
}

#[test]
fn a_recursive_search_of_the_project_root_or_the_store_is_refused() {
    let w = world();
    let branch = scope(&w, &w.worktree);
    for command in [
        // The live shape: the project root holds the store.
        format!("grep -rn 'fn needle' '{}'", w.project),
        format!("cd '{}' && grep -R needle .", w.project),
        format!("rg --hidden needle '{}'", w.project),
        format!("rg -uu -efoo '{}'", w.project),
        format!("rg --files '{}/.host'", w.project),
        // A pattern attached to its option is not the root.
        format!("grep -rn -eneedle '{}'", w.project),
        format!("grep -rnefoo '{}'", w.project),
        format!("grep -r --regexp=foo '{}'", w.project),
        // Compound-statement spellings of the same search.
        format!("(grep -rn x '{}')", w.project),
        format!("(cd '{}' && grep -rn x .)", w.project),
        format!("{{ grep -rn x '{}'; }}", w.project),
        format!("if true; then grep -rn x '{}'; fi", w.project),
        format!("tree -a '{}'", w.project),
        format!("find '{}' -name '*.json'", w.store),
        format!("ls -laR '{}/run-0'", w.store),
        format!(
            "timeout 60 grep --recursive -e needle '{}/run-1/v2/results'",
            w.store
        ),
    ] {
        let refusal = bash(&branch, &command).unwrap_or_else(|| panic!("{command} ran"));
        assert!(
            refusal.contains("recursively") && refusal.contains(&w.worktree),
            "{refusal}"
        );
    }
    let live = bash(&branch, &format!("grep -rn x '{}'", w.project)).unwrap();
    assert!(live.contains("contains the run store"), "{live}");
    let records = bash(&branch, &format!("find '{}/run-0/v2'", w.store)).unwrap();
    assert!(
        records.contains("the host's own record directory"),
        "{records}"
    );
}

#[test]
fn a_recursive_search_outside_the_workspace_is_refused_and_inside_it_passes() {
    let w = world();
    let branch = scope(&w, &w.worktree);
    let outside = bash(&branch, "grep -rn needle /opt/elsewhere").expect("outside");
    assert!(outside.contains("is outside your workspace"), "{outside}");
    // `~` is placed, not waved through as a relative path.
    if std::env::var("HOME").is_ok_and(|home| home.starts_with('/')) {
        assert!(bash(&branch, "grep -rn needle ~").is_some());
    }
    for command in [
        "grep -rn needle src".to_string(),
        "grep -rn needle".to_string(),
        "rg -g '*.rs' needle src tests".to_string(),
        "find . -name '*.rs'".to_string(),
        format!("grep -r needle '{}/src'", w.worktree),
        format!("ls -R '{}/run-1/artifacts'", w.store),
        format!(
            "grep -rn needle '{}'",
            std::env::temp_dir().join("scratch").display()
        ),
        // Not recursive: a named file anywhere is not a walk.
        format!("grep -n needle '{}/Cargo.toml'", w.project),
        "ls -la /opt/elsewhere".to_string(),
        // A pattern that looks like a path is the pattern, not a root.
        "grep -rn /opt/elsewhere src".to_string(),
        // After a cd to an unknown directory, a relative root is not judged.
        "cd $DIR && grep -rn needle .".to_string(),
        // A search that skips hidden directories never enters a store under
        // one, even from the project root.
        format!("rg needle '{}'", w.project),
        format!("tree -L 3 '{}'", w.project),
        format!("ls -R '{}'", w.project),
    ] {
        assert_eq!(bash(&branch, &command), None, "{command}");
    }
}

#[test]
fn a_working_root_that_holds_the_store_is_itself_too_wide_to_search() {
    let w = world();
    // Serial writes and scope discovery run at the project root.
    let serial = scope(&w, &w.project);
    let wide = bash(&serial, "grep -rn needle .").expect("the workspace holds the store");
    assert!(
        wide.contains("a subdirectory of your workspace that does not hold the run store"),
        "{wide}"
    );
    assert!(bash(&serial, "grep -rn needle").is_some());
    assert_eq!(
        bash(&serial, "rg needle"),
        None,
        "rg skips the hidden store"
    );
    assert_eq!(bash(&serial, "grep -rn needle crates"), None);
}

#[test]
fn a_short_name_in_a_quoted_search_root_cannot_hide_the_store() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("RUNNER~1").join("project with spaces");
    let store = project.join(".host/runs");
    let worktree = store.join("run-1/v2/worktrees/call/item");
    std::fs::create_dir_all(&worktree).unwrap();
    let scope = RunStoreScope::new(
        store.join("run-1").to_str(),
        store.to_str(),
        worktree.to_str(),
    );
    let command = format!("grep -r needle '{}'", project.display());
    let refusal = bash(&scope, &command).expect("quoted short path still holds the store");
    assert!(refusal.contains("contains the run store"), "{refusal}");
}

#[cfg(windows)]
#[test]
fn native_scratch_never_exempts_a_store_and_verbatim_workspace_stays_searchable() {
    let w = world();
    let branch = scope(&w, &w.worktree);
    let workspace = std::fs::canonicalize(&w.worktree)
        .map(archon_shell::paths::plain)
        .unwrap();
    let store = std::fs::canonicalize(&w.store)
        .map(archon_shell::paths::plain)
        .unwrap();
    assert_eq!(
        bash(
            &branch,
            &format!("grep -r needle '{}'", workspace.display())
        ),
        None
    );
    let refusal = bash(&branch, &format!("grep -r needle '{}'", store.display()))
        .expect("store under the temp directory must remain refused");
    assert!(refusal.contains("contains the run store"), "{refusal}");
}

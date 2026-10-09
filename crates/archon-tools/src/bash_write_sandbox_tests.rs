//! Issue-124: an isolated write branch cannot modify the project root, the
//! canonical checkout or the run store outside its worktree — from its shell
//! or with a file tool — and can still do its work in the worktree, the run's
//! artifact directory, its declared project artifacts and the host's
//! temp/cache directories.
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::json;

use super::bash_write_sandbox::{WRITE_BOUNDARY_NOTE_MARKER, available, for_shell};
use super::*;
use crate::tool::ToolContext;
use crate::workflow_read_guard::{
    DeclaredTargetScope, HostWriteBoundary, RunStoreScope, WorkflowReadGuard,
};

/// A project root with a run store, one run, one branch worktree inside it,
/// a canonical checkout beside it and a live data file — the incident layout.
struct Layout {
    _base: tempfile::TempDir,
    project: PathBuf,
    checkout: PathBuf,
    store: PathBuf,
    run: PathBuf,
    worktree: PathBuf,
    data: PathBuf,
    declared: PathBuf,
}

fn layout() -> Layout {
    let base = tempfile::tempdir().unwrap();
    let project = base.path().join("project");
    let checkout = base.path().join("checkout");
    let store = project.join(".archon/workflows");
    let run = store.join("wf-synthetic");
    let worktree = run.join("v2/worktrees/item-a/item-a-0");
    let data = project.join(".archon/lab/data/registry.json");
    let declared = project.join(".archon/lab/reports/summary.json");
    for dir in [
        worktree.join("src"),
        run.join("artifacts"),
        checkout.clone(),
        data.parent().unwrap().to_path_buf(),
        declared.parent().unwrap().to_path_buf(),
    ] {
        std::fs::create_dir_all(dir).unwrap();
    }
    std::fs::write(&data, "{\"shape\":\"v1\"}").unwrap();
    std::fs::write(checkout.join("lib.rs"), "base").unwrap();
    Layout {
        _base: base,
        project,
        checkout,
        store,
        run,
        worktree,
        data,
        declared,
    }
}

fn path_str(path: &Path) -> String {
    path.display().to_string()
}

/// The guard as the live dispatch builds it for an isolated branch: the
/// declared-target scope carrying the host's `_write_boundary` stamp, and the
/// run-store scope.
fn guard(layout: &Layout, isolated: bool, git_mutation: bool) -> Arc<WorkflowReadGuard> {
    let worktree = layout.worktree.to_str().unwrap();
    let boundary = HostWriteBoundary::new(
        &[path_str(&layout.project), path_str(&layout.checkout)],
        &[path_str(&layout.declared)],
    );
    Arc::new(
        WorkflowReadGuard::new(40, 20, false, git_mutation)
            .with_declared_targets(
                DeclaredTargetScope::new(&["src/lib.rs".to_string()], Some(worktree))
                    .in_isolated_worktree(isolated)
                    .with_write_boundary(boundary),
            )
            .with_run_store(RunStoreScope::new(
                layout.run.to_str(),
                layout.store.to_str(),
                Some(worktree),
            )),
    )
}

/// `write_roots` is left EMPTY: that is the live default
/// (`workflow.write_confinement = false`), and the boundary must not need it.
fn branch_ctx(layout: &Layout, isolated: bool) -> ToolContext {
    ToolContext {
        working_dir: layout.worktree.clone(),
        session_id: "write-boundary-session".into(),
        workflow_read_guard: Some(guard(layout, isolated, false)),
        ..ToolContext::default()
    }
}

async fn bash(ctx: &ToolContext, command: &str) -> crate::tool::ToolResult {
    BashTool::default()
        .execute(json!({"command": command}), ctx)
        .await
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_default()
}

/// The boundary needs `sandbox-exec` (which cannot be applied inside an
/// already-sandboxed process: this suite run by a bounded branch, say) or
/// Landlock.
fn bounded() -> bool {
    let bounded = available();
    if !bounded {
        eprintln!("skipped: no OS write boundary can be applied in this process");
    }
    bounded
}

/// Landlock is stricter than the macOS profile in two documented ways (see
/// `archon_shell::write_boundary::landlock`): a declared artifact file is re-opened in
/// place only, and nothing new may be made directly in an ancestor of a
/// sealed root.
fn landlock() -> bool {
    matches!(
        archon_shell::write_boundary::mechanism(),
        Ok(archon_shell::write_boundary::Mechanism::Landlock { .. })
    )
}

/// The live shape: an interpreter heredoc that rewrites a project data file
/// in place with `os.replace`, from the worktree.
/// Forward slashes (MSYS bash and Python accept them): a backslashed path in
/// the `-c` text is mangled on its way into MSYS bash (Issue-234).
fn sh(path: &Path) -> String {
    path.display().to_string().replace('\\', "/")
}

fn heredoc_rewrite(target: &Path) -> String {
    let target = path_str(target);
    #[cfg(windows)]
    let target = target.replace('\\', "/");
    format!(
        "python3 - <<'EOF'\nimport os\np = {target}\nopen(p + '.tmp', 'w').write('{{\"shape\":\"v2\"}}')\nos.replace(p + '.tmp', p)\nEOF",
        target = serde_json::to_string(&target).unwrap()
    )
}

#[tokio::test]
async fn an_isolated_branch_cannot_rewrite_project_data_from_its_shell() {
    let layout = layout();
    if !bounded() {
        return;
    }
    let ctx = branch_ctx(&layout, true);
    let result = bash(&ctx, &heredoc_rewrite(&layout.data)).await;
    assert!(result.is_error, "{}", result.content);
    assert_eq!(
        read(&layout.data),
        "{\"shape\":\"v1\"}",
        "{}",
        result.content
    );
    assert!(
        result.content.contains(WRITE_BOUNDARY_NOTE_MARKER),
        "{}",
        result.content
    );
    assert!(!result.is_guard_refusal(), "{}", result.content);

    // Relative climbs, `cd` out of the tree, links, renames of the file or
    // of an ancestor of the project, the run's records and the checkout.
    let escape = sh(&layout.data);
    let base = sh(layout.project.parent().unwrap());
    let commands = vec![
        "printf x > ../../../../../../lab-escape.txt".to_string(),
        format!("cd {} && printf x > escape.txt", sh(&layout.project)),
        format!("ln {escape} hard.json && printf x >> hard.json"),
        format!("mv {escape} moved.json"),
        format!("mv {base} {base}-moved"),
        format!("printf x > {}/state.json", sh(&layout.run)),
        format!("printf x > {}/lib.rs", sh(&layout.checkout)),
    ];
    // Unix-only: without privilege MSYS `ln -s` copies instead of linking, and
    // writing that worktree copy is the branch's own work.
    let symlink = cfg!(unix).then(|| format!("ln -s {escape} link.json; printf x > link.json"));
    for command in commands.into_iter().chain(symlink) {
        let result = bash(&ctx, &command).await;
        assert!(result.is_error, "{command}: {}", result.content);
    }
    assert_eq!(read(&layout.data), "{\"shape\":\"v1\"}");
    assert_eq!(read(&layout.checkout.join("lib.rs")), "base");
    assert!(!layout.project.join("escape.txt").exists());
    assert!(!layout.run.join("state.json").exists());
}

#[tokio::test]
async fn a_declared_artifact_can_be_written_atomically_into_a_new_directory() {
    let layout = layout();
    if !bounded() {
        return;
    }
    let ctx = branch_ctx(&layout, true);
    std::fs::remove_dir_all(layout.declared.parent().unwrap()).unwrap();
    let artifact = sh(&layout.declared);
    if landlock() {
        // Not expressible in Landlock: refused, never widened to its directory.
        let result = bash(&ctx, &format!("mkdir -p \"$(dirname {artifact})\"")).await;
        assert!(result.is_error, "{}", result.content);
        assert!(!layout.declared.parent().unwrap().exists());
        return;
    }
    let result = bash(
        &ctx,
        &format!(
            "mkdir -p \"$(dirname {artifact})\" && python3 - <<'EOF'\nimport os\np = {artifact:?}\n\
             open(p + '.tmp', 'w').write('{{}}')\nos.replace(p + '.tmp', p)\nEOF"
        ),
    )
    .await;
    assert!(!result.is_error, "{}", result.content);
    assert_eq!(read(&layout.declared), "{}");
    // The directory it sits in is not thereby opened.
    let sibling = layout.declared.with_file_name("other.json");
    let result = bash(&ctx, &format!("printf x > {}", sh(&sibling))).await;
    assert!(result.is_error, "{}", result.content);
    assert!(!sibling.exists());
}

#[tokio::test]
async fn an_isolated_branch_still_writes_what_it_owns() {
    let layout = layout();
    if !bounded() {
        return;
    }
    let ctx = branch_ctx(&layout, true);
    let report = layout.run.join("artifacts/report.md");
    let base = layout.project.parent().unwrap();
    let sibling = if landlock() {
        std::fs::write(&layout.declared, "").unwrap();
        std::fs::create_dir_all(base.join("beside")).unwrap();
        let refused = bash(&ctx, &format!("printf x > {}/new.txt", sh(base))).await;
        assert!(refused.is_error, "{}", refused.content);
        base.join("beside/sibling.txt")
    } else {
        base.join("sibling.txt")
    };
    let command = format!(
        "printf 'fn a() {{}}' > src/lib.rs && mkdir -p target/debug && printf ok > {} \
         && printf '{{}}' > {} && printf sib > {} \
         && t=$(mktemp) && printf x > \"$t\" && rm \"$t\" && printf ok > /dev/null",
        sh(&report),
        sh(&layout.declared),
        sh(&sibling)
    );
    let result = bash(&ctx, &command).await;
    assert!(!result.is_error, "{}", result.content);
    assert_eq!(read(&layout.worktree.join("src/lib.rs")), "fn a() {}");
    assert_eq!(read(&report), "ok");
    assert_eq!(read(&layout.declared), "{}");
    assert_eq!(read(&sibling), "sib");
}

fn git(dir: &Path, args: &[&str]) {
    let output = std::process::Command::new("git")
        .args(["-c", "user.email=t@example.invalid", "-c", "user.name=t"])
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
}

/// Turn the layout's checkout into a repository and the worktree into a
/// linked worktree of it.
fn linked_worktree(layout: &Layout) {
    std::fs::remove_dir_all(&layout.worktree).unwrap();
    git(&layout.checkout, &["init", "-q"]);
    git(&layout.checkout, &["add", "lib.rs"]);
    git(&layout.checkout, &["commit", "-q", "-m", "base"]);
    git(
        &layout.checkout,
        &["worktree", "add", "-q", layout.worktree.to_str().unwrap()],
    );
}

#[tokio::test]
async fn git_works_in_a_linked_worktree_and_its_gitdir_cannot_be_redirected() {
    let layout = layout();
    if !bounded() {
        return;
    }
    linked_worktree(&layout);
    let ctx = branch_ctx(&layout, true);
    let result = bash(
        &ctx,
        "printf changed > lib.rs && git status --porcelain && git diff --stat \
         && git checkout -- lib.rs",
    )
    .await;
    assert!(!result.is_error, "{}", result.content);
    assert_eq!(read(&layout.worktree.join("lib.rs")), "base");

    // The `.git` file is the agent's: pointing it at the project must not
    // make the project writable on the next command.
    let data_dir = sh(layout.data.parent().unwrap());
    let result = bash(&ctx, &format!("printf 'gitdir: {data_dir}' > .git")).await;
    assert!(!result.is_error, "{}", result.content);
    let result = bash(&ctx, &heredoc_rewrite(&layout.data)).await;
    assert!(result.is_error, "{}", result.content);
    assert_eq!(read(&layout.data), "{\"shape\":\"v1\"}");
}

#[tokio::test]
async fn a_call_not_stamped_isolated_is_not_bounded() {
    // The boundary is for the isolated worktree branch only: a serial or
    // coordinated write shares the project tree and writes it by design.
    let layout = layout();
    let ctx = branch_ctx(&layout, false);
    let result = bash(&ctx, &heredoc_rewrite(&layout.data)).await;
    assert!(!result.is_error, "{}", result.content);
    assert_eq!(read(&layout.data), "{\"shape\":\"v2\"}");
}

#[test]
fn the_file_tools_answer_to_the_same_boundary() {
    let layout = layout();
    let guard = guard(&layout, true, false);
    let write = |path: &Path| {
        guard.before_tool(
            "Write",
            &json!({"file_path": path_str(path), "content": "x"}),
        )
    };
    let refusal = write(&layout.data).expect("project data is sealed");
    assert!(
        refusal.contains("outside this isolated write branch's worktree"),
        "{refusal}"
    );
    assert!(write(&layout.checkout.join("lib.rs")).is_some());
    #[cfg(unix)]
    {
        // A link in the worktree is judged where it lands, dangling or not.
        std::os::unix::fs::symlink(&layout.data, layout.worktree.join("link.json")).unwrap();
        std::os::unix::fs::symlink(
            layout.data.with_file_name("new.json"),
            layout.worktree.join("dangling.json"),
        )
        .unwrap();
        assert!(write(&layout.worktree.join("link.json")).is_some());
        assert!(write(&layout.worktree.join("dangling.json")).is_some());
    }
    for owned in [
        layout.worktree.join("src/lib.rs"),
        layout.run.join("artifacts/report.md"),
        layout.declared.clone(),
    ] {
        assert_eq!(write(&owned), None, "{owned:?}");
    }
    assert_eq!(
        guard.before_tool("Edit", &json!({"file_path": "src/lib.rs"})),
        None
    );
}

fn profile(layout: &Layout, env: &[(String, String)], git_mutation: bool) -> String {
    let paths = guard(layout, true, git_mutation)
        .boundary_paths()
        .expect("a boundary");
    for_shell(paths, env, &["PROJECT_CACHE".to_string()], git_mutation).profile()
}

fn clause<'a>(profile: &'a str, head: &str) -> &'a str {
    profile
        .lines()
        .find(|line| line.starts_with(head))
        .unwrap_or_else(|| panic!("no {head} clause in {profile}"))
}

fn subpath(path: &Path) -> String {
    let normalized: PathBuf = path.components().collect();
    format!("(subpath {:?})", path_str(&normalized))
}

#[test]
fn the_profile_seals_host_roots_and_reopens_only_what_the_branch_owns() {
    let layout = layout();
    let cache = layout.project.join("target-cache");
    let outside = tempfile::tempdir().unwrap();
    let base = layout.project.parent().unwrap();
    let env = vec![
        // Contains the project root: allowing it would re-open the project.
        ("TMPDIR".to_string(), path_str(base)),
        ("CARGO_TARGET_DIR".to_string(), path_str(&cache)),
        ("PROJECT_CACHE".to_string(), path_str(outside.path())),
        // Not a host-selected directory: never re-opened.
        (
            "LAB_DATA".to_string(),
            path_str(layout.data.parent().unwrap()),
        ),
    ];
    let profile = profile(&layout, &env, false);
    let deny = clause(&profile, "(deny file-write* (subpath");
    let ancestors = clause(&profile, "(deny file-write* (literal");
    let allow = clause(&profile, "(allow file-write*");
    for sealed in [&layout.project, &layout.checkout, &layout.store] {
        assert!(deny.contains(&subpath(sealed)), "{sealed:?}: {profile}");
    }
    assert!(!deny.contains(&subpath(&layout.worktree)), "{profile}");
    assert!(
        ancestors.contains(&format!("(literal {:?})", path_str(base))),
        "{profile}"
    );
    for dir in [
        &layout.worktree,
        &layout.run.join("artifacts"),
        &layout.declared,
        &cache,
    ] {
        assert!(allow.contains(&subpath(dir)), "{dir:?}: {profile}");
    }
    assert!(allow.contains(&subpath(outside.path())), "{profile}");
    assert!(!allow.contains(&subpath(base)), "{profile}");
    assert!(
        !allow.contains(&subpath(layout.data.parent().unwrap())),
        "{profile}"
    );
    // The denies come first: the last matching rule wins.
    assert!(profile.find("(deny").unwrap() < profile.find("(allow file-write*").unwrap());
}

#[test]
fn the_shared_git_directory_opens_only_its_commit_state_for_git_mutation() {
    let layout = layout();
    linked_worktree(&layout);
    let common = archon_shell::paths::canonicalize(layout.checkout.join(".git")).unwrap();
    let text = std::fs::read_to_string(layout.worktree.join(".git")).unwrap();
    let named = text.trim().strip_prefix("gitdir:").unwrap().trim();
    let gitdir = archon_shell::paths::canonicalize(named).unwrap();

    let without = profile(&layout, &[], false);
    let allow = clause(&without, "(allow file-write*");
    assert!(allow.contains(&subpath(&gitdir)), "{without}");
    assert!(
        !allow.contains(&subpath(&common.join("objects"))),
        "{without}"
    );

    // Pointed at a sibling branch's git directory, the `.git` file opens
    // nothing: that directory names its own worktree, not this one.
    let sibling = layout.worktree.with_file_name("item-b-0");
    git(
        &layout.checkout,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "b",
            sibling.to_str().unwrap(),
        ],
    );
    let sibling_text = std::fs::read_to_string(sibling.join(".git")).unwrap();
    let ours = std::fs::read_to_string(layout.worktree.join(".git")).unwrap();
    std::fs::write(layout.worktree.join(".git"), &sibling_text).unwrap();
    let redirected = profile(&layout, &[], false);
    let sibling_dir = sibling_text.trim().strip_prefix("gitdir:").unwrap().trim();
    let sibling_dir = archon_shell::paths::canonicalize(sibling_dir).unwrap();
    assert!(
        !clause(&redirected, "(allow file-write*").contains(&subpath(&sibling_dir)),
        "{redirected}"
    );
    std::fs::write(layout.worktree.join(".git"), ours).unwrap();

    let with = profile(&layout, &[], true);
    let allow = clause(&with, "(allow file-write*");
    for shared in ["objects", "refs", "logs"] {
        assert!(allow.contains(&subpath(&common.join(shared))), "{with}");
    }
    for sealed in ["hooks", "config"] {
        assert!(!allow.contains(&subpath(&common.join(sealed))), "{with}");
    }
    assert!(!allow.contains(&subpath(&common)), "{with}");
}

#[path = "bash_write_sandbox_boundary_tests.rs"]
mod boundary_tests;

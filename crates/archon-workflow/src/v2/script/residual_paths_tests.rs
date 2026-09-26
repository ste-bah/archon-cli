//! Issue-117: which named files count, who owns them, which may be opened.

use super::*;
use crate::task_universe::WorkflowV2TaskUniverseTask;

fn repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    for path in [
        "crates/x/src/store.rs",
        "crates/x/src/lib.rs",
        "crates/y/src/lib.rs",
        "crates/y/src/deep/one.rs",
        "docs/guide.md",
        ".mcp.json",
        "crates/x/src/frozen.rs",
    ] {
        let target = dir.path().join(path);
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(target, "//\n").unwrap();
    }
    dir
}

fn task(id: &str, owns: &[&str], forbids: &[&str]) -> WorkflowV2TaskUniverseTask {
    WorkflowV2TaskUniverseTask {
        canonical_task_id: id.into(),
        source_path: format!("tasks/{id}.md"),
        files_expected_to_change: owns.iter().map(|f| f.to_string()).collect(),
        files_forbidden_to_change: forbids.iter().map(|f| f.to_string()).collect(),
        ..Default::default()
    }
}

fn universe(root: &Path) -> WorkflowV2TaskUniverse {
    WorkflowV2TaskUniverse {
        schema_version: "test".into(),
        source_roots: Vec::new(),
        tasks: vec![
            task(
                "TASK-A",
                &[&format!(
                    "`{}/crates/x/src/lib.rs` — exists",
                    root.display()
                )],
                &[
                    "`crates/x/src/store.rs` (observed)",
                    "`crates/x/src/frozen.rs`",
                    "`docs/`",
                ],
            ),
            task("TASK-B", &["crates/y/src/"], &["`crates/x/**`"]),
        ],
    }
}

#[test]
fn only_exact_existing_files_count_and_locations_are_dropped() {
    let dir = repo();
    let root = dir.path();
    let text = format!(
        "see crates/x/src/store.rs:226-229 and (crates/y/src/lib.rs::ingest), `crates/x/src/lib.rs:12:4`, \
         {}/crates/y/src/deep/one.rs#L3; not crates/x/src/absent.rs, \
         not providers/store.rs, not ../crates/x/src/lib.rs",
        root.display()
    );
    assert_eq!(
        named_files(&text, root),
        [
            "crates/x/src/lib.rs",
            "crates/x/src/store.rs",
            "crates/y/src/deep/one.rs",
            "crates/y/src/lib.rs",
        ]
    );
    // A directory and a glob name what they match.
    assert_eq!(
        named_files("crates/*/src/lib.rs and crates/y/src/deep/", root),
        [
            "crates/x/src/lib.rs",
            "crates/y/src/deep/one.rs",
            "crates/y/src/lib.rs"
        ]
    );
    assert!(named_files("/etc/passwd and https://x.io/a/b.rs", root).is_empty());
    // A link is never a repository file, wherever it points.
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink("/etc/hosts", root.join("crates/x/src/hosts.rs")).unwrap();
        std::os::unix::fs::symlink(
            root.join("crates/x/src/lib.rs"),
            root.join("crates/x/src/alias.rs"),
        )
        .unwrap();
        assert!(named_files("crates/x/src/hosts.rs crates/x/src/alias.rs", root).is_empty());
    }
}

#[test]
fn owners_come_from_declarations_and_unowned_must_be_proven() {
    let dir = repo();
    let root = dir.path();
    let u = universe(root);
    assert_eq!(
        owners(&u, "crates/x/src/lib.rs", root),
        BTreeSet::from(["TASK-A".to_string()])
    );
    assert_eq!(
        owners(&u, "crates/y/src/deep/one.rs", root),
        BTreeSet::from(["TASK-B".to_string()]),
        "a declared directory owns what lies under it"
    );
    assert!(owners(&u, "crates/x/src/store.rs", root).is_empty());
    assert!(provably_unowned(&u, "crates/x/src/store.rs", root));
    assert!(!provably_unowned(&u, "crates/x/src/lib.rs", root));
    // One unreadable declared entry proves nothing about any path.
    let mut unreadable = u.clone();
    unreadable.tasks[1]
        .files_expected_to_change
        .push("<DATASET>/x.json".into());
    assert!(!provably_unowned(
        &unreadable,
        "crates/x/src/store.rs",
        root
    ));
}

#[test]
fn protected_paths_are_never_opened() {
    for path in [
        ".mcp.json",
        "crates/a/.mcp.json",
        "docs/x.md",
        "prds/p.md",
        "tasks/T.md",
        ".archon/x.json",
        "config.toml",
        "config/providers.yaml",
        ".claude/settings.json",
    ] {
        assert!(protected(path), "{path}");
    }
    assert!(!protected("crates/x/src/store.rs"));
}

#[test]
fn related_tasks_are_the_naming_tasks_else_the_unit() {
    let set = |ids: &[&str]| ids.iter().map(|id| id.to_string()).collect::<BTreeSet<_>>();
    assert_eq!(
        related_tasks(&set(&["A", "B", "C"]), &set(&["A"])),
        set(&["A"])
    );
    assert_eq!(related_tasks(&set(&["B"]), &set(&["A"])), set(&["A"]));
    assert_eq!(related_tasks(&set(&["B"]), &set(&[])), set(&["B"]));
    assert!(related_tasks(&set(&[]), &set(&[])).is_empty());
}

#[test]
fn an_exact_forbidden_file_is_lifted_but_a_wider_pattern_never_is() {
    let dir = repo();
    let root = dir.path();
    let u = universe(root);
    let files: BTreeSet<String> = [
        "crates/x/src/store.rs",
        "crates/x/src/frozen.rs",
        ".mcp.json",
    ]
    .iter()
    .map(|f| f.to_string())
    .collect();
    let a = BTreeSet::from(["TASK-A".to_string()]);
    assert_eq!(
        expandable(&u, &a, &files, root),
        ["crates/x/src/frozen.rs", "crates/x/src/store.rs"]
            .iter()
            .map(|f| f.to_string())
            .collect::<BTreeSet<_>>(),
        "A forbids both files exactly: the round's lift opens them; .mcp.json never"
    );
    let forbidden = residual_forbidden(&u, &["TASK-A".into()], &["crates/x/src/store.rs".into()]);
    assert!(!forbidden.matches("crates/x/src/store.rs"));
    assert!(
        forbidden.matches("crates/x/src/frozen.rs"),
        "only the granted file"
    );
    assert!(
        forbidden.matches("docs/guide.md"),
        "a directory stays forbidden"
    );
    // B forbids the whole crate: nothing under it may be granted to B.
    let b = BTreeSet::from(["TASK-B".to_string()]);
    assert!(expandable(&u, &b, &files, root).is_empty());
}

#[test]
fn a_task_file_naming_the_path_relates_the_task() {
    let dir = repo();
    let root = dir.path();
    std::fs::create_dir_all(root.join("tasks")).unwrap();
    std::fs::write(
        root.join("tasks/TASK-A.md"),
        "`crates/x/src/store.rs` must stay consistent with the ingest lane\n",
    )
    .unwrap();
    std::fs::write(
        root.join("tasks/TASK-B.md"),
        format!("see {}/crates/y/src/lib.rs\n", root.display()),
    )
    .unwrap();
    let texts = TaskTexts::read(&universe(root), root);
    assert_eq!(
        texts.naming("crates/x/src/store.rs", root),
        BTreeSet::from(["TASK-A".to_string()])
    );
    assert_eq!(
        texts.naming("crates/y/src/lib.rs", root),
        BTreeSet::from(["TASK-B".to_string()]),
        "the absolute spelling counts"
    );
}

#[test]
fn files_are_read_at_the_commit_the_verifier_judged() {
    let dir = repo();
    let root = dir.path();
    let git = |args: &[&str]| {
        let out = std::process::Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap().trim().to_string()
    };
    git(&["init", "-q"]);
    git(&["config", "user.email", "t@example.invalid"]);
    git(&["config", "user.name", "t"]);
    git(&["add", "."]);
    git(&["commit", "-qm", "base"]);
    let judged = git(&["rev-parse", "HEAD"]);
    // A later round deletes one named file and creates another.
    std::fs::remove_file(root.join("crates/x/src/store.rs")).unwrap();
    std::fs::write(root.join("crates/x/src/new.rs"), "//\n").unwrap();
    let text = "crates/x/src/store.rs:12 and crates/x/src/new.rs";
    assert_eq!(
        named_files_at(text, root, Some(&judged)),
        ["crates/x/src/store.rs"]
    );
    assert_eq!(named_files(text, root), ["crates/x/src/new.rs"]);
    assert_eq!(
        named_files_at(text, root, Some("0000000000000000000000000000000000000000")),
        ["crates/x/src/new.rs"],
        "a commit git cannot read falls back to the working tree"
    );
}

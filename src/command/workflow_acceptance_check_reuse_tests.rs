use super::*;

#[test]
fn identical_bounded_check_reuses() {
    let assessment = assess("test -f feature.txt", "abc", "logic-1", "env-1");
    assert!(assessment.reusable, "{assessment:?}");
    assert_eq!(assessment.reason, "all inputs are host-proven identical");
}

#[test]
fn changed_check_text_reruns() {
    let old = key("test -f feature.txt", "abc", "logic-1", "env-1");
    let changed = key("test -s feature.txt", "abc", "logic-1", "env-1");
    assert_ne!(old, changed);
}

#[test]
fn changed_repository_closure_reruns() {
    let old = key("test -f feature.txt", "tree-a", "logic-1", "env-1");
    let changed = key("test -f feature.txt", "tree-b", "logic-1", "env-1");
    assert_ne!(old, changed);
}

#[test]
fn unbounded_check_is_volatile() {
    let assessment = assess("cargo test", "tree", "logic-1", "env-1");
    assert!(!assessment.reusable);
    assert_eq!(assessment.reason, "check read closure is unbounded");
}

#[test]
fn logic_upgrade_reruns() {
    let old = key("test -f feature.txt", "tree", "logic-1", "env-1");
    let upgraded = key("test -f feature.txt", "tree", "logic-2", "env-1");
    assert_ne!(old, upgraded);
}

#[test]
fn reuse_decision_and_reason_are_recordable() {
    let assessment = assess("test -f feature.txt", "tree", "logic-1", "env-1");
    let record = serde_json::json!({
        "reused": assessment.reusable,
        "why": assessment.reason,
        "evidence": "probe-results/original.json"
    });
    assert_eq!(record["reused"], true);
    assert_eq!(record["why"], "all inputs are host-proven identical");
    assert_eq!(record["evidence"], "probe-results/original.json");
}

fn key(check: &str, tree: &str, logic: &str, environment: &str) -> String {
    super::reuse_key(check, tree, logic, environment)
}

#[test]
fn shell_globs_and_expansions_are_volatile() {
    assert!(!assess("test -f *.txt", "tree", "logic-1", "env-1").reusable);
    assert!(!assess("test -f $FILE", "tree", "logic-1", "env-1").reusable);
}

#[test]
fn acceptance_freeze_reuse_logic_newline_command_is_volatile() {
    assert!(!assess("test\n-f\nfoo", "tree", "logic-1", "env-1").reusable);
}

#[test]
fn acceptance_freeze_reuse_logic_command_separator_is_volatile() {
    assert!(!assess("test -f foo; foo", "tree", "logic-1", "env-1").reusable);
}

#[test]
fn acceptance_freeze_reuse_logic_command_substitution_is_volatile() {
    assert!(!assess("test -f $(foo)", "tree", "logic-1", "env-1").reusable);
}

#[test]
fn acceptance_freeze_reuse_logic_selected_shell_identity_reruns() {
    let root = std::env::temp_dir().join(format!(
        "archon-shell-identity-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let selected = root.join("selected-sh");
    std::fs::write(&selected, b"shell version one").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&selected, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let original = shell_identity_for(&selected, std::slice::from_ref(&root)).unwrap();
    std::fs::write(&selected, b"shell version two").unwrap();
    let changed = shell_identity_for(&selected, std::slice::from_ref(&root)).unwrap();
    let _ = std::fs::remove_dir_all(root);
    assert_ne!(
        original, changed,
        "changed selected shell must invalidate reuse"
    );
}

#[test]
fn acceptance_freeze_reuse_logic_selected_shell_path_change_reruns() {
    let root = std::env::temp_dir().join(format!(
        "archon-shell-path-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let first = root.join("first");
    let second = root.join("second");
    std::fs::create_dir_all(&first).unwrap();
    std::fs::create_dir_all(&second).unwrap();
    for directory in [&first, &second] {
        let shell = directory.join("sh");
        std::fs::write(&shell, b"same shell bytes").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&shell, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
    }
    let selected = std::path::Path::new("sh");
    let original = shell_identity_for(selected, std::slice::from_ref(&first)).unwrap();
    let changed = shell_identity_for(selected, std::slice::from_ref(&second)).unwrap();
    let _ = std::fs::remove_dir_all(root);
    assert_ne!(
        original, changed,
        "a different selected shell path must invalidate reuse"
    );
}

/// Issue 288: a bounded verdict key binds the freeze-acceptance logic version
/// and pinned source digest instead of the build. That is sound only when the
/// code that runs and judges such a check is in the digest's closure.
#[test]
fn bounded_verdict_logic_closure_covers_check_execution() {
    use crate::command::workflow_host_command_logic::{CAPABILITY_LOGIC, LOGIC_DENYLIST};
    let repo = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let logic = CAPABILITY_LOGIC
        .iter()
        .find(|logic| logic.id == "freeze-acceptance")
        .expect("freeze-acceptance has a logic version");
    let roots = logic.sources.concat();
    let mut workspace = crate::command::workflow_host_command_logic_closure::Workspace::new(&repo);
    let closure = workspace.closure(&roots, LOGIC_DENYLIST);
    let hashed: std::collections::BTreeSet<String> = closure
        .files
        .iter()
        .map(|file| file.to_string_lossy().replace('\\', "/"))
        .collect();
    let missing: Vec<_> = [
        "src/command/workflow_acceptance_check_reuse.rs",
        "src/command/workflow_acceptance_executability_sites.rs",
        "src/command/workflow_acceptance_executability_sites_reuse.rs",
        "src/command/workflow_acceptance_executability_resume.rs",
        "src/command/workflow_acceptance_executability_hermetic.rs",
        "crates/archon-workflow/src/acceptance_scratch_process.rs",
        "crates/archon-workflow/src/acceptance_check_environment.rs",
        "crates/archon-shell/src/lib.rs",
        // Sets the resource environment of every check just before launch.
        "crates/archon-tools/src/workflow_resource_env.rs",
    ]
    .into_iter()
    .filter(|file| !hashed.contains(*file))
    .collect();
    assert!(
        missing.is_empty(),
        "freeze-acceptance logic digest does not hash {missing:?}"
    );
    // Everything the check process launcher reaches shapes the process
    // (environment, argv, cwd, shell): none of it may stop at a denylist
    // entry, or a change there would keep a bounded verdict.
    let launcher =
        std::path::PathBuf::from("crates/archon-workflow/src/acceptance_scratch_process.rs");
    let denied: std::collections::BTreeSet<_> = closure.denied.values().flatten().collect();
    let mut seen = std::collections::BTreeSet::from([launcher.clone()]);
    let mut queue = std::collections::VecDeque::from([launcher]);
    let mut unhashed = Vec::new();
    while let Some(file) = queue.pop_front() {
        for target in closure.references.get(&file).into_iter().flatten() {
            if denied.contains(target) {
                unhashed.push(format!("{} (from {})", target.display(), file.display()));
            } else if closure.files.contains(target) && seen.insert(target.clone()) {
                queue.push_back(target.clone());
            }
        }
    }
    assert!(
        unhashed.is_empty(),
        "the check launcher reaches code outside the freeze-acceptance digest:\n{}",
        unhashed.join("\n")
    );
}

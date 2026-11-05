use super::*;

#[test]
#[allow(non_snake_case)]
fn R6_B1_author_and_site_share_missing_root_decisions() {
    // Input: absent sibling root versus absent descendants of an existing root.
    // Expected: only the missing root is refused by both author and actual site setup.
    let tree = tempfile::tempdir().unwrap();
    let project = tree.path().join("workspace");
    let repository = tree.path().join("repository");
    std::fs::create_dir(&project).unwrap();
    std::fs::create_dir(&repository).unwrap();
    for (command, refused) in [
        ("test -e ../missing/item", true),
        ("cd ../missing && test -f x", true),
        ("test -f ../repository/missing/data.txt", false),
        ("cd ../repository && test -f src/new.rs", false),
        ("cd ../repository && cd sub && test -f y", false),
    ] {
        let author = crate::acceptance_sibling_paths::missing_root(&project, command);
        let site = setup_failure(
            &project,
            crate::task_set_contract::TrustedCwd::ProjectRoot,
            command,
        );
        let named = crate::acceptance_sibling_paths::scan(command)
            .pop()
            .unwrap()
            .path;
        let predicted = super::super::site_path::site_path_alone(
            &project,
            &repository,
            command,
            crate::task_set_contract::TrustedCwd::ProjectRoot,
            &named,
            false,
        );
        assert_eq!(predicted.is_err(), refused, "{command}: {predicted:?}");
        assert_eq!(author.is_some(), refused, "{command}");
        assert_eq!(site.is_some(), refused, "{command}: {site:?}");
        if let Some(reason) = author {
            assert_eq!(site.unwrap().0, format!("the check {reason}"));
        }
    }
}

#[test]
#[allow(non_snake_case)]
fn S3_existing_site_scoping_agrees_at_all_deciders() {
    // Input: data, program file reads and shell reads, in both working roots.
    // Expected: author/policy, root preflight, site prediction and actual site setup agree.
    let tree = tempfile::tempdir().unwrap();
    let project = tree.path().join("workspace");
    let repository = tree.path().join("repository");
    std::fs::create_dir(&project).unwrap();
    std::fs::create_dir(&repository).unwrap();
    use crate::task_set_contract::{AcceptanceCriterion, TrustedCwd};
    for cwd in [TrustedCwd::ProjectRoot, TrustedCwd::RepoRoot] {
        let root = if cwd == TrustedCwd::ProjectRoot {
            &project
        } else {
            &repository
        };
        for (command, refused) in [
            ("echo ../missing/item", false),
            ("grep ../missing/item input", false),
            ("python3 -c 'print(\"../missing/x\")'", false),
            ("python3 -c 'open(\"../missing/x\")'", true),
            (
                "node -e 'require(\"fs\").readFileSync(\"../missing/x\")'",
                false,
            ),
            ("perl -e 'open F, \"../missing/x\"'", false),
            ("node -e 'fs.readFileSync(\"../missing/x\")'", true),
            ("echo --manifest-path ../missing/Cargo.toml", false),
            ("cargo run -- --manifest-path ../missing/Cargo.toml", false),
            (
                "cd $place; cargo metadata --manifest-path ../missing/Cargo.toml",
                false,
            ),
            ("sh -c 'cat ../missing/x'", true),
            ("cat <<'EOF'\nopen('../missing/x')\nEOF", false),
            ("sh <<'EOF'\ncat ../missing/x\nEOF", true),
            ("test -e ../missing/item", true),
            ("runner - <<'EOF'\nopen('../missing/x')\nEOF", true),
            ("cargo metadata --manifest-path ../missing/Cargo.toml", true),
            (
                "test -f input || cargo metadata --manifest-path ../missing/Cargo.toml",
                true,
            ),
            ("cat ../missing/item", true),
            ("cat < ../missing/item", true),
            ("cd ../missing && test -f x", true),
        ] {
            let candidate: AcceptanceCriterion = serde_json::from_value(serde_json::json!({
                "id":"check", "criterion":"checks data", "check":{"kind":"command", "command":command, "cwd":cwd},
                "judgment":{"verdict":"accepted","counterexample":"","reason":"","host_call_id":"test"}
            })).unwrap();
            let policy = crate::task_set_contract::acceptance_site_path_defect(
                &candidate,
                &project,
                &repository,
                false,
            )
            .is_some();
            let preflight = crate::acceptance_sibling_paths::missing_root(root, command).is_some();
            let setup = setup_failure(root, cwd, command).is_some();
            let prediction = super::super::site_path::site_path_alone(
                &project,
                &repository,
                command,
                cwd,
                Path::new("../missing/item"),
                false,
            )
            .is_err();
            assert_eq!(
                [policy, preflight, setup, prediction],
                [refused; 4],
                "{cwd:?}: {command}"
            );
        }
    }
}

#[test]
#[allow(non_snake_case)]
fn N2_mixed_sibling_paths_name_only_the_absent_root() {
    // Input: existing ../a/x and absent ../z -> only the ../z root refusal.
    let tree = tempfile::tempdir().unwrap();
    let project = tree.path().join("workspace");
    let repository = tree.path().join("repository");
    std::fs::create_dir(&project).unwrap();
    std::fs::create_dir(&repository).unwrap();
    std::fs::create_dir(tree.path().join("a")).unwrap();
    std::fs::write(tree.path().join("a/x"), "input").unwrap();
    let refusal = super::super::site_path::site_path_alone(
        &project,
        &repository,
        "test -f ../a/x && test -e ../z",
        crate::task_set_contract::TrustedCwd::ProjectRoot,
        Path::new("../a/x"),
        false,
    )
    .unwrap_err();
    assert_eq!(
        refusal.reason.unwrap(),
        "names sibling directory ../z, which does not exist next to the working directory; name an existing directory"
    );
    let candidate: crate::task_set_contract::AcceptanceCriterion = serde_json::from_value(serde_json::json!({
        "id":"check", "criterion":"checks inputs", "check":{"kind":"command", "command":"test -f ../a/x && test -e ../z", "cwd":"project_root"},
        "judgment":{"verdict":"accepted","counterexample":"","reason":"","host_call_id":"test"}
    })).unwrap();
    let policy = crate::task_set_contract::acceptance_site_path_defect(
        &candidate,
        &project,
        &repository,
        false,
    )
    .unwrap();
    assert!(policy.contains("sibling directory ../z,"), "{policy}");
    assert!(!policy.contains("../a/x"), "{policy}");
}

#[test]
#[allow(non_snake_case)]
fn N3_or_exit_missing_cd_names_root_at_site() {
    // Input: missing cd followed by || exit -> B1 root finding at the site.
    let tree = tempfile::tempdir().unwrap();
    let refusal = setup_failure(
        tree.path(),
        crate::task_set_contract::TrustedCwd::ProjectRoot,
        "cd ../absent || exit 1; test -f f",
    )
    .unwrap();
    assert_eq!(
        refusal.0,
        "the check names sibling directory ../absent, which does not exist next to the working directory; name an existing directory"
    );
}

#[test]
#[allow(non_snake_case)]
fn N4_absent_root_trigger_describes_the_refusal() {
    // Input: missing sibling root -> author fault trigger explicitly describes absence.
    let tree = tempfile::tempdir().unwrap();
    let (_, cause) = setup_failure(
        tree.path(),
        crate::task_set_contract::TrustedCwd::ProjectRoot,
        "test -e ../absent/x",
    )
    .unwrap();
    let super::super::faults::CheckFault::Check { trigger, .. } = cause.unwrap().fault() else {
        panic!("missing root belongs to the check author");
    };
    assert!(trigger.contains("sibling root is absent"), "{trigger}");
}

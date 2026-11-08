use super::*;

#[test]
fn binary_inputs_are_not_source_bytes_but_remain_probe_inputs() {
    let root = tempfile::tempdir().unwrap();
    let binary = std::fs::File::create(root.path().join("input")).unwrap();
    binary.set_len(100_173_120).unwrap();
    let subject = command_check("inspect input", TrustedCwd::ProjectRoot);
    let roots = JudgeRoots {
        covered_texts: Default::default(),
        project: Some(root.path().to_path_buf()),
        repository: Some(root.path().to_path_buf()),
        task_root: Some(root.path().to_path_buf()),
        repository_commit: None,
    };
    assert_eq!(
        crate::command::workflow_task_set::executability::named_inputs(
            &subject.acceptance[0],
            root.path(),
            root.path()
        ),
        ["input"]
    );
    let mut budget = 1;
    let files = named_files_with_budget(&subject.acceptance[0], &roots, &mut budget);
    assert_eq!(files[0]["status"], "not_judgeable");
    assert!(
        files[0]["detail"]
            .as_str()
            .unwrap()
            .contains("not judgeable text")
    );
    assert!(files[0].get("content").is_none());
    assert_eq!(budget, 1);
    assert!(super::super::named_files::named_file_sizes(&subject.acceptance[0], &roots).is_empty());
    assert!(super::super::each::withheld_files(&subject.acceptance[0], &roots).is_empty());
    let digest = named_files_content_digest(&subject.acceptance[0], &roots);
    std::fs::write(root.path().join("input"), b"\0changed").unwrap();
    assert_ne!(
        digest,
        named_files_content_digest(&subject.acceptance[0], &roots)
    );
    git(root.path(), &["init", "-q"]);
    git(root.path(), &["add", "input"]);
    git(
        root.path(),
        &[
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.com",
            "commit",
            "-qm",
            "input",
        ],
    );
    let subject = command_check("inspect input", TrustedCwd::RepoRoot);
    let roots = super::super::each::with_commit(&roots);
    let files = named_files_with_budget(&subject.acceptance[0], &roots, &mut budget);
    assert_eq!(files[0]["status"], "not_judgeable");
    assert_eq!(budget, 1);
}

#[test]
fn loop_substitution_does_not_supply_unrelated_root_binary() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join("archon-cli/crates")).unwrap();
    std::fs::write(root.path().join("archon-cli/crates/input.rs"), "source").unwrap();
    std::fs::File::create(root.path().join("archon"))
        .unwrap()
        .set_len(100_173_120)
        .unwrap();
    let subject = command_check(
        r#"set -eu
cd archon-cli
for t in A B C D; do
    out=$(cargo test --bin archon --lib "$t" -- --exact 2>&1) || { printf '%s\n' "$out"; exit 1; }
    printf '%s\n' "$out" | grep -q "test result: ok" || { printf '%s\n' "$out"; exit 1; }
done
python3 - <<'PY'
from pathlib import Path
Path('crates/input.rs').read_text()
PY
"#,
        TrustedCwd::ProjectRoot,
    );
    let roots = JudgeRoots {
        covered_texts: Default::default(),
        project: Some(root.path().to_path_buf()),
        repository: None,
        task_root: Some(root.path().to_path_buf()),
        repository_commit: None,
    };
    let files = named_files(&subject.acceptance[0], &roots);
    assert!(
        files.iter().all(|file| file["path"] != "archon"),
        "{files:?}"
    );
    assert!(
        files.iter().any(
            |file| file["path"] == "archon-cli/crates/input.rs" && file["status"] == "supplied"
        ),
        "{files:?}"
    );
}

#[test]
fn missing_live_cd_target_does_not_name_root_text_or_binary() {
    let root = tempfile::tempdir().unwrap();
    let subject = command_check(
        r#"set -eu
cd archon-cli
for t in A B C D; do
    out=$(cargo test --bin archon --lib "$t" -- --exact 2>&1) || { printf '%s\n' "$out"; exit 1; }
    printf '%s\n' "$out" | grep -q "test result: ok" || { printf '%s\n' "$out"; exit 1; }
done
python3 - <<'PY'
from pathlib import Path
Path('crates/input.rs').read_text()
PY
"#,
        TrustedCwd::ProjectRoot,
    );
    let roots = JudgeRoots {
        covered_texts: Default::default(),
        project: Some(root.path().to_path_buf()),
        repository: None,
        task_root: Some(root.path().to_path_buf()),
        repository_commit: None,
    };
    assert!(!root.path().join("archon-cli").exists());
    for binary in [false, true] {
        let path = root.path().join("archon");
        if binary {
            std::fs::File::create(path)
                .unwrap()
                .set_len(100_173_120)
                .unwrap();
        } else {
            std::fs::write(path, vec![b'x'; 300_000]).unwrap();
        }
        assert!(
            crate::command::workflow_task_set::executability::named_inputs(
                &subject.acceptance[0],
                root.path(),
                root.path()
            )
            .is_empty()
        );
        assert!(named_files(&subject.acceptance[0], &roots).is_empty());
        assert!(
            super::super::named_files::named_file_sizes(&subject.acceptance[0], &roots).is_empty()
        );
        assert!(super::super::each::withheld_files(&subject.acceptance[0], &roots).is_empty());
    }
}

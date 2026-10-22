use super::*;

#[cfg(unix)]
#[test]
fn repository_sibling_alias_uses_the_tree_commit() {
    let source = tempfile::tempdir().unwrap();
    let destination = tempfile::tempdir().unwrap();
    let project = source.path().join("project");
    let repository = source.path().join("repo-dir");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::create_dir_all(&repository).unwrap();
    committed_repository(&repository);
    std::fs::write(
        repository.join("Cargo.toml"),
        "[package]\nname = \"tree_manifest\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    std::fs::create_dir_all(repository.join("src")).unwrap();
    std::fs::write(repository.join("src/lib.rs"), "pub fn tree() {}\n").unwrap();
    let git = |args: &[&str]| {
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(&repository)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    };
    git(&["add", "."]);
    git(&["commit", "-qm", "manifest at tree commit"]);
    let tree_commit = git(&["rev-parse", "HEAD"]);
    std::fs::write(
        repository.join("Cargo.toml"),
        "[package]\nname = \"live_manifest\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    git(&["add", "-A"]);
    git(&["commit", "-qm", "manifest changed at live head"]);
    let live_head = git(&["rev-parse", "HEAD"]);
    assert_ne!(tree_commit, live_head);

    let command = "cargo metadata --manifest-path ../repo-dir/Cargo.toml --no-deps >/dev/null && grep -q tree_manifest ../repo-dir/Cargo.toml";
    let contract: AcceptanceContract = serde_json::from_value(serde_json::json!({
        "schema_version": 1, "prd": {"path":"p.md", "digest":"d"},
        "gap_policy": {"permitted_acceptance_ids":[], "forbidden_phrases":[], "required_fields":[]},
        "acceptance": [{"id":"AC-SIBLING-REPO", "criterion":"uses the committed manifest", "check":{"kind":"command", "command":command, "cwd":"project_root"}, "judgment":{"verdict":"accepted", "counterexample":"", "reason":"", "host_call_id":"test"}}],
        "supplementary": []
    })).unwrap();
    let check_command =
        "test -f ../repo-dir/Cargo.toml && grep -q tree_manifest ../repo-dir/Cargo.toml";
    let live_check = std::process::Command::new("/bin/sh")
        .args(["-c", check_command])
        .current_dir(&project)
        .output()
        .unwrap();
    assert!(
        !live_check.status.success(),
        "live HEAD should fail this tree check"
    );
    let copy = HermeticCopy::at(
        destination.path(),
        &repository,
        &project,
        None,
        &tree_commit,
    )
    .unwrap();
    let mut layout = copy.layout();
    super::super::super::siblings::install_copy_layout(
        &mut layout,
        &copy.project,
        &project,
        &copy.repository,
        &repository,
        &contract,
    )
    .unwrap();

    let manifest_alias = copy.project.join("../repo-dir/Cargo.toml");
    assert!(manifest_alias.is_file(), "{}", manifest_alias.display());
    let check = std::process::Command::new("/bin/sh")
        .args(["-c", check_command])
        .current_dir(&copy.project)
        .output()
        .unwrap();
    assert!(check.status.success());
    assert_eq!(
        std::fs::canonicalize(&manifest_alias).unwrap(),
        std::fs::canonicalize(copy.repository.join("Cargo.toml")).unwrap()
    );
    assert_eq!(git(&["rev-parse", "HEAD"]), live_head);
    let copied_head = std::process::Command::new("git")
        .arg("-C")
        .arg(&copy.repository)
        .args(["rev-parse", "HEAD"])
        .output()
        .unwrap();
    assert!(copied_head.status.success());
    assert_eq!(
        String::from_utf8_lossy(&copied_head.stdout).trim(),
        tree_commit
    );
    copy.remove().unwrap();
}

#[cfg(unix)]
#[test]
fn program_text_sibling_manifests_are_copied_for_the_can_fail_probe() {
    let source = tempfile::tempdir().unwrap();
    let project = source.path().join("project");
    let sibling = source.path().join("sibling");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::create_dir_all(&sibling).unwrap();
    std::fs::write(
        sibling.join("Cargo.toml"),
        "[package]\nname='sibling'\nversion='0.1.0'\n",
    )
    .unwrap();
    let repository = repository(&source);
    let commit = committed_repository(&repository);
    let commands = [
        "python3 - <<'PY'\nMANIFEST='../sibling/Cargo.toml'\nimport os\nassert os.path.isfile(MANIFEST)\nraise SystemExit(17)\nPY",
        "python3 -c 'MANIFEST=\"../sibling/Cargo.toml\"; import os; assert os.path.isfile(MANIFEST); raise SystemExit(17)'",
    ];
    for (index, command) in commands.into_iter().enumerate() {
        let mut contract: AcceptanceContract = serde_json::from_value(serde_json::json!({
            "schema_version": 1, "prd": {"path":"p.md", "digest":"d"},
            "gap_policy": {"permitted_acceptance_ids":[], "forbidden_phrases":[], "required_fields":[]},
            "acceptance": [{"id":format!("AC-420-PROGRAM-{index}"), "criterion":"runs sibling manifest", "check":{"kind":"command", "command":command, "cwd":"project_root"}, "judgment":{"verdict":"accepted", "counterexample":"", "reason":"", "host_call_id":"test"}}],
            "supplementary": []
        })).unwrap();
        let destination = tempfile::tempdir().unwrap();
        let copy =
            HermeticCopy::at(destination.path(), &repository, &project, None, &commit).unwrap();
        super::super::siblings::copy_for_checks(
            &copy.project,
            &project,
            &copy.repository,
            &repository,
            &mut contract,
        )
        .unwrap();
        assert!(
            copy.project
                .parent()
                .unwrap()
                .join("sibling/Cargo.toml")
                .is_file()
        );
        let output = std::process::Command::new("/bin/sh")
            .args(["-c", command])
            .current_dir(&copy.project)
            .output()
            .unwrap();
        assert_eq!(
            output.status.code(),
            Some(17),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        copy.remove().unwrap();
    }
}

#[test]
fn non_git_sibling_copy_uses_current_files_without_a_commit() {
    // EXPECTED: current non-Git sibling content is copied with no commit.
    let source = tempfile::tempdir().unwrap();
    let destination = tempfile::tempdir().unwrap();
    let project = source.path().join("project");
    let sibling = source.path().join("sibling");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::create_dir_all(&sibling).unwrap();
    std::fs::write(sibling.join("value.txt"), "current on-disk content").unwrap();
    let repository = repository(&source);
    let commit = committed_repository(&repository);
    let copy = HermeticCopy::at(destination.path(), &repository, &project, None, &commit).unwrap();
    let evidence = super::super::siblings::copy_for_checks(
        &copy.project,
        &project,
        &copy.repository,
        &repository,
        &contract_for("cat ../sibling/value.txt"),
    )
    .unwrap();
    let actual =
        std::fs::read_to_string(copy.project.parent().unwrap().join("sibling/value.txt")).unwrap();
    println!(
        "EVIDENCE non-git-sibling BEFORE: copy absent; AFTER: content={actual}, commit={:?}",
        evidence[0].commit
    );
    assert_eq!(actual, "current on-disk content");
    assert_eq!(evidence[0].path, PathBuf::from("../sibling"));
    assert_eq!(evidence[0].commit, None);
    copy.remove().unwrap();
}

#[test]
fn a_sibling_root_named_target_is_copied() {
    // EXPECTED: ../target itself is copied; nested target directories are not.
    let source = tempfile::tempdir().unwrap();
    let destination = tempfile::tempdir().unwrap();
    let project = source.path().join("project");
    let target = source.path().join("target");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::create_dir_all(target.join("target/debug")).unwrap();
    std::fs::write(target.join("needed.txt"), "named sibling root").unwrap();
    std::fs::write(target.join("target/debug/output"), "nested build").unwrap();
    let repository = repository(&source);
    let commit = committed_repository(&repository);
    let copy = HermeticCopy::at(destination.path(), &repository, &project, None, &commit).unwrap();
    super::super::siblings::copy_for_checks(
        &copy.project,
        &project,
        &copy.repository,
        &repository,
        &contract_for("cat ../target/needed.txt"),
    )
    .unwrap();
    let copied_target = copy.project.parent().unwrap().join("target");
    let actual = std::fs::read_to_string(copied_target.join("needed.txt")).unwrap();
    println!(
        "EVIDENCE target-root BEFORE: named sibling absent; AFTER: content={actual}, nested target exists={}",
        copied_target.join("target").exists()
    );
    assert_eq!(actual, "named sibling root");
    assert!(!copied_target.join("target").exists());
    copy.remove().unwrap();
}

#[test]
#[ignore = "run against the external project contract with ARCHON_REAL_ACCEPTANCE_CONTRACT"]
fn current_sibling_gate_accepts_the_two_real_checks_after_copying_their_inputs() {
    let source = std::path::PathBuf::from(
        std::env::var_os("ARCHON_REAL_ACCEPTANCE_CONTRACT")
            .expect("set ARCHON_REAL_ACCEPTANCE_CONTRACT to the read-only task contract"),
    );
    let root = tempfile::tempdir().unwrap();
    let project = root.path().join("project-1");
    let repository_sibling = root.path().join("archon-cli");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::create_dir_all(&repository_sibling).unwrap();
    std::fs::copy(source, project.join("acceptance-contract.json")).unwrap();
    std::fs::write(repository_sibling.join("Cargo.toml"), "[workspace]\n").unwrap();
    let contract: AcceptanceContract =
        serde_json::from_slice(&std::fs::read(project.join("acceptance-contract.json")).unwrap())
            .unwrap();
    let ids = BTreeSet::from(["SUP-REQ-AHDM-032".to_string(), "SUP-REQ-DL-055".to_string()]);
    let findings =
        super::super::siblings::missing_for_checks(&project, &repository_sibling, &contract, &ids);
    assert!(
        findings.is_empty(),
        "current sibling gate findings: {findings:?}"
    );
}

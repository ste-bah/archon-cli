use super::*;

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

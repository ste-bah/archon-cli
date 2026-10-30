use super::*;

#[cfg(unix)]
#[test]
fn probe_copy_materializes_tracked_sibling_at_the_same_relative_path() {
    // EXPECTED before running: sibling `../sibling` is absent in the current
    // copy, so the real command reports setup failure; after the fix the
    // tracked script runs and exits 17, while untracked build output stays out.
    let source = tempfile::tempdir().unwrap();
    let destination = tempfile::tempdir().unwrap();
    let project = source.path().join("project");
    let sibling = source.path().join("sibling");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::create_dir_all(&sibling).unwrap();
    std::fs::write(sibling.join("check.sh"), "#!/bin/sh\nexit 17\n").unwrap();
    committed_repository(&sibling);
    for args in [
        &["add", "check.sh"][..],
        &["commit", "-m", "tracked check"][..],
    ] {
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(&sibling)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    std::fs::create_dir_all(sibling.join("target/debug")).unwrap();
    std::fs::write(sibling.join("target/debug/output"), b"build output").unwrap();
    let repository = repository(&source);
    let commit = committed_repository(&repository);
    let command = "cd ../sibling && sh check.sh";

    let mut contract: AcceptanceContract = serde_json::from_value(serde_json::json!({
        "schema_version": 1, "prd": {"path":"p.md", "digest":"d"},
        "gap_policy": {"permitted_acceptance_ids":[], "forbidden_phrases":[], "required_fields":[]},
        "acceptance": [{"id":"AC-420", "criterion":"runs sibling", "check":{"kind":"command", "command":command, "cwd":"project_root"}, "judgment":{"verdict":"accepted", "counterexample":"", "reason":"", "host_call_id":"test"}}],
        "supplementary": []
    })).unwrap();
    let copy = HermeticCopy::at(destination.path(), &repository, &project, None, &commit).unwrap();
    super::super::siblings::copy_for_checks(
        &copy.project,
        &project,
        &copy.repository,
        &repository,
        &mut contract,
    )
    .unwrap();
    let sibling_copy = copy.project.parent().unwrap().join("sibling");
    let output = std::process::Command::new("/bin/sh")
        .args(["-c", command])
        .current_dir(&copy.project)
        .output()
        .unwrap();
    let after = if output.status.code() == Some(17) {
        "tracked sibling ran; status=17"
    } else {
        "sibling absent; setup failure"
    };
    println!("EVIDENCE #420 sibling-copy BEFORE: sibling absent; setup failure; AFTER: {after}");
    assert_eq!(
        output.status.code(),
        Some(17),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(sibling_copy.join("check.sh").is_file());
    assert!(!sibling_copy.join("target").exists());
    copy.remove().unwrap();
}

#[test]
fn a_missing_sibling_is_an_author_finding_with_path_and_check_id() {
    // EXPECTED before running: the absent sibling is refused during author
    // validation, with both its path and acceptance check id in the finding.
    let project = tempfile::tempdir().unwrap();
    let command = "cargo test --manifest-path ../absent/Cargo.toml";
    let contract: AcceptanceContract = serde_json::from_value(serde_json::json!({
        "schema_version": 1, "prd": {"path":"p.md", "digest":"d"},
        "gap_policy": {"permitted_acceptance_ids":[], "forbidden_phrases":[], "required_fields":[]},
        "acceptance": [{"id":"AC-420-MISSING", "criterion":"runs sibling", "check":{"kind":"command", "command":command, "cwd":"project_root"}, "judgment":{"verdict":"accepted", "counterexample":"", "reason":"", "host_call_id":"test"}}],
        "supplementary": []
    })).unwrap();
    let ids = BTreeSet::from(["AC-420-MISSING".to_string()]);
    let findings =
        super::super::siblings::missing_for_checks(project.path(), project.path(), &contract, &ids);
    let finding = &findings["AC-420-MISSING"];
    println!(
        "EVIDENCE #420 missing-sibling BEFORE: check AC-420-MISSING would run; AFTER: author refusal={finding}"
    );
    assert!(finding.contains("AC-420-MISSING"));
    assert!(finding.contains("../absent/Cargo.toml"));
}

#[test]
fn quoted_comma_separated_input_argument_resolves_existing_sibling() {
    let root = tempfile::tempdir().unwrap();
    let project = root.path().join("project");
    let sibling_manifest = root.path().join("sibling/package.manifest");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::create_dir_all(sibling_manifest.parent().unwrap()).unwrap();
    std::fs::write(&sibling_manifest, "package = \"fixture\"\n").unwrap();
    let command = "runner -c 'run([\"--manifest-path\", \"../sibling/package.manifest\", \"-p\"])'";
    let contract: AcceptanceContract = serde_json::from_value(serde_json::json!({
        "schema_version": 1, "prd": {"path":"p.md", "digest":"d"},
        "gap_policy": {"permitted_acceptance_ids":[], "forbidden_phrases":[], "required_fields":[]},
        "acceptance": [{"id":"AC-X-001", "criterion":"uses sibling input", "check":{"kind":"command", "command":command, "cwd":"project_root"}, "judgment":{"verdict":"accepted", "counterexample":"", "reason":"", "host_call_id":"test"}}],
        "supplementary": []
    })).unwrap();
    let ids = BTreeSet::from(["AC-X-001".to_string()]);
    assert!(
        super::super::siblings::missing_for_checks(&project, &project, &contract, &ids).is_empty()
    );
    assert_eq!(
        super::super::siblings::copied_root_names(&project, &project, &contract),
        vec!["sibling"]
    );
}

#[test]
fn copied_root_names_follow_the_check_working_directory_and_existing_entries() {
    // EXPECTED before running: repository-root-relative sibling inputs that
    // really exist appear in the copied tree; absent sibling names do not.
    let root = tempfile::tempdir().unwrap();
    let project = root.path().join("project");
    let repository = root.path().join("repository");
    let sibling = root.path().join("sibling");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::create_dir_all(&repository).unwrap();
    std::fs::create_dir_all(&sibling).unwrap();
    std::fs::write(sibling.join("present.txt"), "actual sibling").unwrap();
    let contract: AcceptanceContract = serde_json::from_value(serde_json::json!({
        "schema_version": 1, "prd": {"path":"p.md", "digest":"d"},
        "gap_policy": {"permitted_acceptance_ids":[], "forbidden_phrases":[], "required_fields":[]},
        "acceptance": [{"id":"AC-420", "criterion":"uses sibling", "check":{"kind":"command", "command":"test -f ../sibling/present.txt && test -f ../absent/file", "cwd":"repo_root"}, "judgment":{"verdict":"accepted", "counterexample":"", "reason":"", "host_call_id":"test"}}],
        "supplementary": []
    })).unwrap();
    let names = super::super::siblings::copied_root_names(&project, &repository, &contract);
    assert_eq!(names, vec!["sibling"]);
}

#[cfg(unix)]
#[test]
fn prose_path_is_not_a_missing_sibling_finding() {
    // EXPECTED: an absent path used only as echo text creates no author
    // finding, while the same path in a setup position does.
    let project = tempfile::tempdir().unwrap();
    let command = "echo '../absent'";
    let contract: AcceptanceContract = serde_json::from_value(serde_json::json!({
        "schema_version": 1, "prd": {"path":"p.md", "digest":"d"},
        "gap_policy": {"permitted_acceptance_ids":[], "forbidden_phrases":[], "required_fields":[]},
        "acceptance": [{"id":"AC-420-PROSE", "criterion":"prints prose", "check":{"kind":"command", "command":command, "cwd":"project_root"}, "judgment":{"verdict":"accepted", "counterexample":"", "reason":"", "host_call_id":"test"}}],
        "supplementary": []
    })).unwrap();
    let ids = BTreeSet::from(["AC-420-PROSE".to_string()]);
    assert!(super::super::siblings::missing_for_checks(
        project.path(),
        project.path(),
        &contract,
        &ids,
    )
    .is_empty());
}

#[cfg(unix)]
#[test]
fn an_existing_relative_file_is_copied_even_when_it_is_prose() {
    // EXPECTED: ordinary prose does not require a path, but an existing
    // relative sibling file is still materialized at the path it names.
    let source = tempfile::tempdir().unwrap();
    let destination = tempfile::tempdir().unwrap();
    let project = source.path().join("project");
    let sibling = source.path().join("sibling");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::create_dir_all(sibling.join("target/debug")).unwrap();
    std::fs::create_dir_all(sibling.join("node_modules/pkg")).unwrap();
    std::fs::write(source.path().join("note.txt"), "real sibling file").unwrap();
    std::fs::write(sibling.join("note.txt"), "sibling directory").unwrap();
    std::fs::write(sibling.join("target/debug/output"), "build").unwrap();
    std::fs::write(sibling.join("node_modules/pkg/output"), "dependency").unwrap();
    let repository = repository(&source);
    let commit = committed_repository(&repository);
    let copy = HermeticCopy::at(destination.path(), &repository, &project, None, &commit).unwrap();
    let evidence = super::super::siblings::copy_for_checks(
        &copy.project,
        &project,
        &copy.repository,
        &repository,
        &contract_for("echo ../note.txt ../sibling"),
    )
    .unwrap();
    let copied = copy.project.parent().unwrap().join("note.txt");
    let actual = std::fs::read_to_string(&copied).unwrap();
    println!(
        "EVIDENCE #420 sibling-file BEFORE: copy path absent; AFTER: path=../note.txt, content={actual}"
    );
    assert_eq!(actual, "real sibling file");
    let copied_sibling = copy.project.parent().unwrap().join("sibling");
    assert_eq!(
        std::fs::read_to_string(copied_sibling.join("note.txt")).unwrap(),
        "sibling directory"
    );
    assert!(!copied_sibling.join("target").exists());
    assert!(!copied_sibling.join("node_modules").exists());
    assert_eq!(evidence[0].path, PathBuf::from("../note.txt"));
    assert!(
        copy.layout()
            .site_tree()
            .unwrap()
            .1
            .contains(&"note.txt".to_string())
    );
    copy.remove().unwrap();
}

#[cfg(unix)]
#[test]
fn symlinked_sibling_is_materialized_at_its_alias_path() {
    // EXPECTED: the check's `../alias` path is present in the copy and holds
    // target content even though the source path is a symlink.
    let source = tempfile::tempdir().unwrap();
    let destination = tempfile::tempdir().unwrap();
    let project = source.path().join("project");
    let target = source.path().join("target");
    let alias = source.path().join("alias");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::create_dir_all(&target).unwrap();
    std::fs::write(target.join("tracked.txt"), "alias-content").unwrap();
    std::os::unix::fs::symlink(&target, &alias).unwrap();
    let repository = repository(&source);
    let commit = committed_repository(&repository);
    let contract = contract_for("cat ../alias/tracked.txt");
    let copy = HermeticCopy::at(destination.path(), &repository, &project, None, &commit).unwrap();
    let evidence = super::super::siblings::copy_for_checks(
        &copy.project,
        &project,
        &copy.repository,
        &repository,
        &contract,
    )
    .unwrap();
    let alias_copy = copy.project.parent().unwrap().join("alias");
    let materialized = std::fs::read_to_string(alias_copy.join("tracked.txt")).unwrap();
    println!(
        "EVIDENCE #420 symlink-alias BEFORE: copy path absent; AFTER: path=../alias, content={materialized}"
    );
    assert_eq!(materialized, "alias-content");
    assert_eq!(evidence[0].path, PathBuf::from("../alias"));
    copy.remove().unwrap();
}

#[cfg(unix)]
#[test]
fn copied_git_sibling_records_the_current_head() {
    // EXPECTED: after HEAD advances, the probe clone has that commit and its
    // path and commit are recorded in copy evidence.
    let source = tempfile::tempdir().unwrap();
    let destination = tempfile::tempdir().unwrap();
    let project = source.path().join("project");
    let sibling = source.path().join("sibling");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::create_dir_all(&sibling).unwrap();
    std::fs::write(sibling.join("value.txt"), "first").unwrap();
    committed_repository(&sibling);
    let write_commit = |value: &str| {
        std::fs::write(sibling.join("value.txt"), value).unwrap();
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(&sibling)
            .args(["add", "value.txt"])
            .output()
            .unwrap();
        assert!(output.status.success());
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(&sibling)
            .args(["commit", "-m", value])
            .output()
            .unwrap();
        assert!(output.status.success());
    };
    write_commit("second");
    let current = std::process::Command::new("git")
        .arg("-C")
        .arg(&sibling)
        .args(["rev-parse", "HEAD"])
        .output()
        .unwrap();
    let current = String::from_utf8_lossy(&current.stdout).trim().to_string();
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
    let copied = copy.project.parent().unwrap().join("sibling");
    let copied_head = std::process::Command::new("git")
        .arg("-C")
        .arg(&copied)
        .args(["rev-parse", "HEAD"])
        .output()
        .unwrap();
    let copied_head = String::from_utf8_lossy(&copied_head.stdout)
        .trim()
        .to_string();
    println!(
        "EVIDENCE #420 sibling-revision BEFORE: old revision could be copied; AFTER: path=../sibling, source={current}, copied={copied_head}"
    );
    assert_eq!(evidence[0].path, PathBuf::from("../sibling"));
    assert_eq!(evidence[0].commit.as_deref(), Some(current.as_str()));
    assert_eq!(copied_head, current);
    assert_eq!(
        std::fs::read_to_string(copied.join("value.txt")).unwrap(),
        "second"
    );
    copy.remove().unwrap();
}

fn contract_for(command: &str) -> AcceptanceContract {
    serde_json::from_value(serde_json::json!({
        "schema_version": 1, "prd": {"path":"p.md", "digest":"d"},
        "gap_policy": {"permitted_acceptance_ids":[], "forbidden_phrases":[], "required_fields":[]},
        "acceptance": [{"id":"AC-420", "criterion":"runs sibling", "check":{"kind":"command", "command":command, "cwd":"project_root"}, "judgment":{"verdict":"accepted", "counterexample":"", "reason":"", "host_call_id":"test"}}],
        "supplementary": []
    })).unwrap()
}

#[cfg(unix)]
#[test]
fn a_copy_error_drops_the_entire_temporary_parent() {
    let source = tempfile::tempdir().unwrap();
    let destination = tempfile::tempdir().unwrap();
    let project = source.path().join("project");
    let sibling = source.path().join("broken");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::create_dir_all(sibling.join(".git")).unwrap();
    let repository = repository(&source);
    let commit = committed_repository(&repository);
    let copy = HermeticCopy::at(destination.path(), &repository, &project, None, &commit).unwrap();
    let root = copy.root.clone();
    let contract: AcceptanceContract = serde_json::from_value(serde_json::json!({
        "schema_version": 1, "prd": {"path":"p.md", "digest":"d"},
        "gap_policy": {"permitted_acceptance_ids":[], "forbidden_phrases":[], "required_fields":[]},
        "acceptance": [{"id":"AC-420-ERROR", "criterion":"runs sibling", "check":{"kind":"command", "command":"cd ../broken", "cwd":"project_root"}, "judgment":{"verdict":"accepted", "counterexample":"", "reason":"", "host_call_id":"test"}}],
        "supplementary": []
    })).unwrap();
    let error = super::super::siblings::copy_for_checks(
        &copy.project,
        &project,
        &copy.repository,
        &repository,
        &contract,
    )
    .unwrap_err();
    assert!(
        error.0.contains("git rev-parse HEAD") || error.0.contains("git clone"),
        "{}",
        error.0
    );
    drop(copy);
    println!(
        "EVIDENCE #420 cleanup-on-error BEFORE: temporary parent exists; AFTER: removed after sibling copy error"
    );
    assert!(!root.exists());
}

#[cfg(unix)]
#[test]
fn two_simultaneous_copies_of_one_sibling_succeed_without_touching_source() {
    // EXPECTED before running: independent copy roots both materialize the
    // same real sibling; source bytes and HEAD remain unchanged.
    let source = tempfile::tempdir().unwrap();
    let project = source.path().join("project");
    let sibling = source.path().join("sibling");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::create_dir_all(&sibling).unwrap();
    std::fs::write(sibling.join("value.txt"), "source stays intact").unwrap();
    committed_repository(&sibling);
    let before = std::fs::read(sibling.join("value.txt")).unwrap();
    let add = std::process::Command::new("git")
        .arg("-C")
        .arg(&sibling)
        .args(["add", "value.txt"])
        .output()
        .unwrap();
    assert!(add.status.success());
    let commit = std::process::Command::new("git")
        .arg("-C")
        .arg(&sibling)
        .args(["commit", "-m", "tracked value"])
        .output()
        .unwrap();
    assert!(commit.status.success());
    let before_head = std::process::Command::new("git")
        .arg("-C")
        .arg(&sibling)
        .args(["rev-parse", "HEAD"])
        .output()
        .unwrap();
    let before_head = String::from_utf8_lossy(&before_head.stdout)
        .trim()
        .to_owned();
    let repository = repository(&source);
    let commit = committed_repository(&repository);
    let contract = contract_for("cat ../sibling/value.txt");
    let mut workers = Vec::new();
    for _ in 0..2 {
        let destination = tempfile::tempdir().unwrap();
        let project = project.clone();
        let repository = repository.clone();
        let commit = commit.clone();
        let contract = contract.clone();
        workers.push(std::thread::spawn(move || {
            let copy =
                HermeticCopy::at(destination.path(), &repository, &project, None, &commit).unwrap();
            super::super::siblings::copy_for_checks(
                &copy.project,
                &project,
                &copy.repository,
                &repository,
                &contract,
            )
            .unwrap();
            let copied = copy.project.parent().unwrap().join("sibling/value.txt");
            assert_eq!(
                std::fs::read_to_string(copied).unwrap(),
                "source stays intact"
            );
            copy.remove().unwrap();
        }));
    }
    for worker in workers {
        worker.join().unwrap();
    }
    let after_head = std::process::Command::new("git")
        .arg("-C")
        .arg(&sibling)
        .args(["rev-parse", "HEAD"])
        .output()
        .unwrap();
    assert_eq!(std::fs::read(sibling.join("value.txt")).unwrap(), before);
    assert_eq!(
        String::from_utf8_lossy(&after_head.stdout).trim(),
        before_head
    );
}

#[test]
fn empty_sibling_layout_serializes_like_the_original_layout_shape() {
    // EXPECTED before implementation: a no-sibling copy layout has the
    // established root/repository/project shape and no sibling key.
    let layout = super::super::hermetic::CopyLayout {
        root: PathBuf::from("copy"),
        repository: PathBuf::from("copy/repository"),
        site_repository: None,
        project: PathBuf::from("copy/project"),
        siblings: Vec::new(),
        check_siblings: std::collections::BTreeMap::new(),
        site_build_cache: None,
    };
    let encoded = serde_json::to_string(&layout).unwrap();
    assert_eq!(
        encoded,
        r#"{"root":"copy","repository":"copy/repository","project":"copy/project"}"#
    );
}

#[path = "workflow_acceptance_executability_sibling_extra_tests.rs"]
mod extra_tests;

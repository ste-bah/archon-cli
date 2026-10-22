use super::*;

#[test]
fn named_existing_file_content_is_shown_and_judge_rule_is_updated() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("data")).unwrap();
    std::fs::write(
        root.path().join("data/input"),
        "asserts every component against its expected value\n",
    )
    .unwrap();
    let mut subject = contract();
    if let archon_workflow::task_set_contract::AcceptanceCheck::Command { command, .. } =
        &mut subject.acceptance[0].check
    {
        *command = "operation data/input".into();
    }
    let roots = JudgeRoots {
        project: Some(root.path().to_path_buf()),
        repository: None,
        task_root: Some(root.path().to_path_buf()),
        repository_commit: None,
    };
    let prompt = batched_judge_prompt_with_roots(&subject, &roots).unwrap();
    assert!(prompt.contains("\"named_files\":[{\"content\":\"asserts every component against its expected value\\n\",\"path\":\"data/input\"}]"), "{prompt}");
    assert!(prompt.contains(NAMED_FILES_INSTRUCTION));
}

#[test]
fn named_file_judge_rules_cover_asserting_regression_guards() {
    let prompt = batched_judge_prompt(&contract()).unwrap();
    for rule in [
        "A check that runs an existing test or implementation whose given file contents (named_files) assert the criterion is a valid regression guard.",
        "A check that names its deciding file by a literal path relative to its working directory and whose named_files assert the criterion is a valid regression guard.",
        "For a criterion that already holds before implementation, the host proves the check by moving the literally named input file or files aside and requiring the check to fail on its own assertion.",
        "Duplicated or relocated implementation elsewhere is not an in-bounds counterexample for a guard whose named test asserts the criterion.",
        "A counterexample against such a guard must show that the named file's assertions pass while the criterion is false.",
        "When a check names a path outside the given named_files, content not given must not be assumed weaker or stronger than the criterion needs; judge only what is given and the command text.",
    ] {
        assert!(prompt.contains(rule), "missing: {rule}\n{prompt}");
    }
    let rules = NAMED_FILES_INSTRUCTION.to_ascii_lowercase();
    for forbidden in [
        "cargo",
        "rustc",
        "cargo.toml",
        "npm",
        "pnpm",
        "yarn",
        "pytest",
        "python",
        "gradle",
        "mvn",
        "anthropic",
        "claude",
        "openai",
    ] {
        assert!(!rules.contains(forbidden), "found {forbidden:?} in {rules}");
    }
}

#[test]
fn named_source_budget_is_shared_across_the_batch() {
    let root = tempfile::tempdir().unwrap();
    let size = archon_workflow::check_source_settle::MAX_JUDGED_SOURCE_BYTES * 3 / 4;
    let first = "a".repeat(size);
    let second = "b".repeat(size);
    std::fs::write(root.path().join("first"), &first).unwrap();
    std::fs::write(root.path().join("second"), &second).unwrap();
    let mut subject = contract();
    subject.acceptance[0].check = AcceptanceCheck::Command {
        command: "operation first".into(),
        cwd: TrustedCwd::ProjectRoot,
    };
    let mut next = subject.acceptance[0].clone();
    next.id = "AC-X-002".into();
    next.check = AcceptanceCheck::Command {
        command: "operation second".into(),
        cwd: TrustedCwd::ProjectRoot,
    };
    subject.acceptance.push(next);
    let prompt = batched_judge_prompt_with_roots(
        &subject,
        &JudgeRoots {
            project: Some(root.path().to_path_buf()),
            repository: None,
            task_root: Some(root.path().to_path_buf()),
            repository_commit: None,
        },
    )
    .unwrap();
    let checks = prompt_json(&prompt);
    let files = checks
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|check| check["named_files"].as_array().cloned().unwrap_or_default())
        .collect::<Vec<_>>();
    let shown: usize = files
        .iter()
        .map(|file| file["content"].as_str().unwrap().len())
        .sum();
    assert_eq!(files.len(), 2, "{prompt}");
    assert!(
        shown <= archon_workflow::check_source_settle::MAX_JUDGED_SOURCE_BYTES + 80,
        "{shown}"
    );
    assert!(
        files[1]["content"]
            .as_str()
            .unwrap()
            .contains("truncated at the existing judged-source byte limit")
    );
}

#[test]
fn a_small_batch_keeps_the_single_check_named_file_prompt_unchanged() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("input"), "stable source\n").unwrap();
    let subject = command_check("operation input", TrustedCwd::ProjectRoot);
    let roots = JudgeRoots {
        project: Some(root.path().to_path_buf()),
        repository: None,
        task_root: Some(root.path().to_path_buf()),
        repository_commit: None,
    };
    let one = named_files(&subject.acceptance[0], &roots);
    let prompt = batched_judge_prompt_with_roots(&subject, &roots).unwrap();
    assert_eq!(
        prompt_json(&prompt)[0]["named_files"],
        serde_json::json!(one)
    );
}

#[test]
fn checks_without_files_keep_their_entry_shape_and_retain_existing_prompt_bytes() {
    let mut subject = contract();
    if let archon_workflow::task_set_contract::AcceptanceCheck::Command { command, .. } =
        &mut subject.acceptance[0].check
    {
        *command = "true".into();
    }
    let original = batched_judge_prompt(&subject).unwrap();
    let root = tempfile::tempdir().unwrap();
    let rooted = batched_judge_prompt_with_roots(
        &subject,
        &JudgeRoots {
            project: Some(root.path().to_path_buf()),
            repository: None,
            task_root: Some(root.path().to_path_buf()),
            repository_commit: None,
        },
    )
    .unwrap();
    let original_json = original.split_once("Checks: ").unwrap().1;
    let rooted_json = rooted.split_once("Checks: ").unwrap().1;
    assert_eq!(rooted_json, original_json);
}

#[test]
fn outside_and_missing_named_paths_are_omitted() {
    let root = tempfile::tempdir().unwrap();
    let outside = root.path().parent().unwrap().join("outside-input");
    std::fs::write(&outside, "outside").unwrap();
    let mut subject = contract();
    if let archon_workflow::task_set_contract::AcceptanceCheck::Command { command, .. } =
        &mut subject.acceptance[0].check
    {
        *command = "operation ../outside-input && operation missing-input".into();
    }
    let prompt = batched_judge_prompt_with_roots(
        &subject,
        &JudgeRoots {
            project: Some(root.path().to_path_buf()),
            repository: None,
            task_root: Some(root.path().to_path_buf()),
            repository_commit: None,
        },
    )
    .unwrap();
    let checks: serde_json::Value =
        serde_json::from_str(prompt.split_once("Checks: ").unwrap().1).unwrap();
    assert!(checks[0].get("named_files").is_none(), "{checks}");
    std::fs::remove_file(outside).unwrap();
}

#[test]
fn named_file_content_is_truncated_at_existing_judged_source_limit() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("data")).unwrap();
    let limit = archon_workflow::check_source_settle::MAX_JUDGED_SOURCE_BYTES;
    std::fs::write(root.path().join("data/input"), vec![b'x'; limit + 1]).unwrap();
    let mut subject = contract();
    if let archon_workflow::task_set_contract::AcceptanceCheck::Command { command, .. } =
        &mut subject.acceptance[0].check
    {
        *command = "operation data/input".into();
    }
    let prompt = batched_judge_prompt_with_roots(
        &subject,
        &JudgeRoots {
            project: Some(root.path().to_path_buf()),
            repository: None,
            task_root: Some(root.path().to_path_buf()),
            repository_commit: None,
        },
    )
    .unwrap();
    assert!(prompt.contains("[truncated at the existing judged-source byte limit]"));
    assert!(!prompt.contains(&"x".repeat(limit + 1)));
}

#[test]
fn untracked_repository_file_is_not_named_to_the_judge() {
    let root = tempfile::tempdir().unwrap();
    git(root.path(), &["init", "-q"]);
    git(root.path(), &["config", "user.email", "judge@example.test"]);
    git(root.path(), &["config", "user.name", "Judge Test"]);
    std::fs::write(root.path().join("tracked"), "safe").unwrap();
    git(root.path(), &["add", "tracked"]);
    git(root.path(), &["commit", "-qm", "initial"]);
    std::fs::write(root.path().join("untracked"), "sensitive content").unwrap();
    let prompt = batched_judge_prompt_with_roots(
        &command_check("operation untracked", TrustedCwd::RepoRoot),
        &JudgeRoots {
            project: Some(root.path().to_path_buf()),
            repository: Some(root.path().to_path_buf()),
            task_root: Some(root.path().to_path_buf()),
            repository_commit: None,
        },
    )
    .unwrap();
    assert!(
        prompt_json(&prompt)[0].get("named_files").is_none(),
        "{prompt}"
    );
}

#[test]
fn tracked_repository_file_is_named_to_the_judge() {
    let root = tempfile::tempdir().unwrap();
    git(root.path(), &["init", "-q"]);
    git(root.path(), &["config", "user.email", "judge@example.test"]);
    git(root.path(), &["config", "user.name", "Judge Test"]);
    std::fs::write(root.path().join("tracked"), "tracked assertion").unwrap();
    git(root.path(), &["add", "tracked"]);
    git(root.path(), &["commit", "-qm", "initial"]);
    let prompt = batched_judge_prompt_with_roots(
        &command_check("operation tracked", TrustedCwd::RepoRoot),
        &JudgeRoots {
            project: Some(root.path().to_path_buf()),
            repository: Some(root.path().to_path_buf()),
            task_root: Some(root.path().to_path_buf()),
            repository_commit: None,
        },
    )
    .unwrap();
    assert!(prompt.contains("tracked assertion"), "{prompt}");
}

// Creating a symlink needs extra privileges on Windows; the rule is host-independent.
#[cfg(unix)]
#[test]
fn repository_named_file_uses_committed_blob_and_skips_committed_symlink() {
    let root = tempfile::tempdir().unwrap();
    git(root.path(), &["init", "-q"]);
    git(root.path(), &["config", "user.email", "judge@example.test"]);
    git(root.path(), &["config", "user.name", "Judge Test"]);
    std::fs::write(root.path().join("tracked"), "committed assertion").unwrap();
    std::fs::write(root.path().join("ignored-target"), "ignored secret").unwrap();
    std::os::unix::fs::symlink("ignored-target", root.path().join("link")).unwrap();
    git(root.path(), &["add", "tracked", "link"]);
    git(root.path(), &["commit", "-qm", "initial"]);
    std::fs::write(root.path().join("tracked"), "working tree assertion").unwrap();
    let prompt = batched_judge_prompt_with_roots(
        &command_check("operation tracked", TrustedCwd::RepoRoot),
        &JudgeRoots {
            project: Some(root.path().to_path_buf()),
            repository: Some(root.path().to_path_buf()),
            task_root: Some(root.path().to_path_buf()),
            repository_commit: None,
        },
    )
    .unwrap();
    assert!(prompt.contains("committed assertion"), "{prompt}");
    assert!(!prompt.contains("working tree assertion"), "{prompt}");

    let prompt = batched_judge_prompt_with_roots(
        &command_check("operation link", TrustedCwd::RepoRoot),
        &JudgeRoots {
            project: Some(root.path().to_path_buf()),
            repository: Some(root.path().to_path_buf()),
            task_root: Some(root.path().to_path_buf()),
            repository_commit: None,
        },
    )
    .unwrap();
    assert!(
        prompt_json(&prompt)[0].get("named_files").is_none(),
        "{prompt}"
    );
}

#[test]
fn project_paths_drop_curdir_and_reject_parent_components() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("x"), "included").unwrap();
    std::fs::write(root.path().join("secret"), "must not be reached").unwrap();
    let roots = JudgeRoots {
        project: Some(root.path().to_path_buf()),
        repository: None,
        task_root: Some(root.path().to_path_buf()),
        repository_commit: None,
    };
    let included = batched_judge_prompt_with_roots(
        &command_check("operation ./x", TrustedCwd::ProjectRoot),
        &roots,
    )
    .unwrap();
    assert!(included.contains("included"), "{included}");
    let rejected = batched_judge_prompt_with_roots(
        &command_check("operation a/../secret", TrustedCwd::ProjectRoot),
        &roots,
    )
    .unwrap();
    assert!(
        prompt_json(&rejected)[0].get("named_files").is_none(),
        "{rejected}"
    );
}

#[test]
fn project_file_under_nested_repository_is_omitted() {
    let project = tempfile::tempdir().unwrap();
    let nested = project.path().join("nested");
    std::fs::create_dir(&nested).unwrap();
    std::fs::write(nested.join("input"), "nested repository content").unwrap();
    let prompt = batched_judge_prompt_with_roots(
        &command_check("operation nested/input", TrustedCwd::ProjectRoot),
        &JudgeRoots {
            project: Some(project.path().to_path_buf()),
            repository: Some(nested),
            task_root: Some(project.path().to_path_buf()),
            repository_commit: None,
        },
    )
    .unwrap();
    assert!(
        prompt_json(&prompt)[0].get("named_files").is_none(),
        "{prompt}"
    );
}

#[test]
fn named_file_digest_tracks_content_and_truncates_at_utf8_boundary() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("x");
    let mut subject = command_check("operation x", TrustedCwd::ProjectRoot);
    let roots = JudgeRoots {
        project: Some(root.path().to_path_buf()),
        repository: None,
        task_root: Some(root.path().to_path_buf()),
        repository_commit: None,
    };
    std::fs::write(&path, "first").unwrap();
    let first = named_files_digest(&subject.acceptance[0], &roots);
    assert_eq!(first, named_files_digest(&subject.acceptance[0], &roots));
    std::fs::write(&path, "other").unwrap();
    assert_ne!(first, named_files_digest(&subject.acceptance[0], &roots));

    let limit = archon_workflow::check_source_settle::MAX_JUDGED_SOURCE_BYTES;
    let mut bytes = vec![b'a'; limit - 1];
    bytes.extend_from_slice("é".as_bytes());
    std::fs::write(&path, bytes).unwrap();
    let prompt = batched_judge_prompt_with_roots(&subject, &roots).unwrap();
    assert!(!prompt.contains('�'), "{prompt}");
    assert!(prompt.contains("[truncated at the existing judged-source byte limit]"));
    subject.acceptance[0].check = AcceptanceCheck::Command {
        command: "operation nested/file".into(),
        cwd: TrustedCwd::ProjectRoot,
    };
}

#[test]
fn project_copy_exclusions_are_omitted_and_named_content_is_redacted() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("credentials")).unwrap();
    std::fs::write(root.path().join("credentials/input"), "password=abc123").unwrap();
    let prompt = batched_judge_prompt_with_roots(
        &command_check("operation credentials/input", TrustedCwd::ProjectRoot),
        &JudgeRoots {
            project: Some(root.path().to_path_buf()),
            repository: None,
            task_root: Some(root.path().to_path_buf()),
            repository_commit: None,
        },
    )
    .unwrap();
    assert!(
        prompt_json(&prompt)[0].get("named_files").is_none(),
        "{prompt}"
    );

    std::fs::write(root.path().join("data-input"), "password=abc123").unwrap();
    let prompt = batched_judge_prompt_with_roots(
        &command_check("operation data-input", TrustedCwd::ProjectRoot),
        &JudgeRoots {
            project: Some(root.path().to_path_buf()),
            repository: None,
            task_root: Some(root.path().to_path_buf()),
            repository_commit: None,
        },
    )
    .unwrap();
    assert!(prompt.contains("REDACTED"), "{prompt}");
    assert!(!prompt.contains("password=abc123"), "{prompt}");
}

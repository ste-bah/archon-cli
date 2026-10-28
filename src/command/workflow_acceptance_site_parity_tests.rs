use super::*;
use archon_workflow::task_set_contract::{AcceptanceCheck, TrustedCwd};

#[path = "workflow_acceptance_site_command_tests.rs"]
mod command_tests;

fn decisions(
    set: &super::super::super::republish::test_fixture::FrozenSet,
    project: &Path,
    command: &str,
    cwd: TrustedCwd,
) -> [bool; 3] {
    let mut contract = set.contract();
    contract.acceptance[0].check = AcceptanceCheck::Command {
        command: command.into(),
        cwd,
    };
    decisions_for_contract(set, project, contract)
}

fn decisions_for_contract(
    set: &super::super::super::republish::test_fixture::FrozenSet,
    project: &Path,
    contract: AcceptanceContract,
) -> [bool; 3] {
    let candidate = contract.acceptance[0].clone();
    let author_scope = AuthorScope::for_task_set(
        project,
        &set.tasks,
        &set.prd,
        site_uses_build_cache(project, &set.tasks),
    )
    .unwrap();
    let reauthor = check_defects::check_defects(&contract, candidate, &author_scope).is_ok();
    let mut text = std::fs::read_to_string(&set.prd).unwrap();
    for (original, candidate) in set.contract().acceptance.iter().zip(&contract.acceptance) {
        text = text.replace(original.criterion.as_str(), candidate.criterion.as_str());
    }
    let exact = contract
        .acceptance
        .iter()
        .map(|entry| (entry.id.clone(), entry.criterion.clone()))
        .collect();
    let expected = contract
        .acceptance
        .iter()
        .map(|entry| entry.id.clone())
        .collect();
    let first = super::super::super::preflight::prepare(
        project,
        &set.tasks,
        &set.prd,
        &content_digest(text.as_bytes()),
        &text,
        &exact,
        &expected,
        &serde_json::to_vec(&contract).unwrap(),
    );
    let published = super::super::super::findings::acceptance_findings(
        project,
        &set.prd,
        &text,
        &set.tasks.join(ACCEPTANCE_CONTRACT_FILE),
        &contract,
    )
    .unwrap();
    let publication = !published.iter().any(|finding| {
        finding.remediation_scope == archon_workflow::RemediationScope::CandidateArtifact
            && finding
                .deterministic_defect
                .as_ref()
                .is_some_and(|defect| defect.code == "unavailable_site_path")
    });
    [reauthor, first.is_ok(), publication]
}

#[test]
fn contract_policy_preflight_and_reauthor_agree_on_prescribed_site_paths() {
    let set = frozen_set(&[("AC-X-001", "./verify input", true)]);
    let root = set.project.path();
    std::fs::write(root.join("input"), "ready").unwrap();
    let name = root.file_name().unwrap().to_string_lossy();
    let mut contract = set.contract();
    contract.acceptance[0].criterion = "The check uses artifact_path exactly as stated.".into();
    for cwd in [TrustedCwd::ProjectRoot, TrustedCwd::RepoRoot] {
        contract.acceptance[0].check = AcceptanceCheck::Command {
            command: format!("test -f ../{name}/input"),
            cwd,
        };
        assert_eq!(
            decisions_for_contract(&set, root, contract.clone()),
            [false; 3],
            "{cwd:?}"
        );
    }
}

#[test]
fn contract_policy_preflight_and_reauthor_agree_on_effective_directory_paths() {
    let set = frozen_set(&[("AC-X-001", "./verify input", true)]);
    let root = set.project.path();
    std::fs::create_dir(root.join("checks")).unwrap();
    std::fs::write(root.join("input"), "ready").unwrap();
    let name = root.file_name().unwrap().to_string_lossy();
    let mut failures = Vec::new();
    for cwd in [TrustedCwd::ProjectRoot, TrustedCwd::RepoRoot] {
        for (command, valid) in [
            ("cd checks && ./verify ../input".to_owned(), true),
            ("cd checks; ./verify ../input".to_owned(), true),
            (format!("test -f ../{name}/input"), false),
            ("cat ../missing/input".to_owned(), false),
            ("cat ../home/input".to_owned(), false),
            ("./verify checks/input".to_owned(), true),
        ] {
            let actual = decisions(&set, root, &command, cwd);
            if actual != [valid; 3] {
                failures.push(format!("{cwd:?} {command}: {actual:?}, expected {valid}"));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn contract_policy_preflight_and_reauthor_agree_on_cached_sibling_paths() {
    let set = frozen_set(&[("AC-X-001", "./verify input", true)]);
    let storage = tempfile::tempdir().unwrap();
    let project = set.project.path().join("workspace");
    std::fs::create_dir_all(project.join(".archon")).unwrap();
    std::fs::write(project.join("input"), "ready").unwrap();
    for name in ["target", "home", "external"] {
        std::fs::create_dir(set.project.path().join(name)).unwrap();
        std::fs::write(set.project.path().join(name).join("needed.txt"), "ready").unwrap();
    }
    let config = serde_json::json!({"workflow": {"acceptance_execution": {
        "repository": set.project.path(), "scratch_parent": storage.path(),
        "project_inputs": [], "project_repository_view": "separate",
        "toolchain_path": std::env::var("PATH").unwrap(),
        "timeout_secs": 30, "output_bytes": 4096, "scratch_bytes": 16 * 1024 * 1024
    }}});
    let config_path = project.join(".archon/config.toml");
    std::fs::write(&config_path, toml::to_string(&config).unwrap()).unwrap();
    let binding = crate::command::acceptance_scratch_policy::capture(&project, &set.tasks)
        .unwrap()
        .unwrap();
    assert!(
        binding.policy.build_cache.is_none(),
        "capture precedes cache selection"
    );
    let mut failures = Vec::new();
    for cached in [true, false] {
        if !cached {
            std::fs::remove_file(&config_path).unwrap();
        }
        for (command, valid) in [
            ("test -f ../target/needed.txt", cached),
            ("test -f ../home/needed.txt", false),
            ("test -f ../external/needed.txt", true),
            ("test -f ../missing/needed.txt", false),
            ("test -f ../workspace/input", false),
        ] {
            let actual = decisions(&set, &project, command, TrustedCwd::ProjectRoot);
            if actual != [valid; 3] {
                failures.push(format!(
                    "cached={cached} {command}: {actual:?}, expected {valid}"
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn site_parity_uses_the_execution_repository_in_both_directions() {
    let set = frozen_set(&[("AC-X-001", "./verify input", true)]);
    let root = set.project.path();
    for name in ["recorded", "current"] {
        std::fs::create_dir(root.join(name)).unwrap();
        std::fs::write(root.join(name).join("input"), "ready").unwrap();
        crate::command::workflow_task_set::passability_tests_support::commit_project(
            &root.join(name),
        );
    }
    let record = archon_workflow::repository_record::RepositoryRecordV1 {
        schema_version: archon_workflow::repository_record::REPOSITORY_RECORD_SCHEMA_VERSION,
        repository_root: root.join("recorded").display().to_string(),
        base_commit: super::super::super::executability::Baseline::head_of(&root.join("recorded"))
            .unwrap()
            .commit,
        decomposition_run_id: "fixture".into(),
        recorded_at: "2026-01-01T00:00:00Z".into(),
    };
    archon_workflow::repository_record::write_repository_record(&set.tasks, &record).unwrap();
    let storage = tempfile::tempdir().unwrap();
    let config = serde_json::json!({"workflow": {"acceptance_execution": {
        "repository": root.join("current"), "scratch_parent": storage.path(),
        "project_inputs": [], "project_repository_view": "separate",
        "toolchain_path": std::env::var("PATH").unwrap(),
        "timeout_secs": 30, "output_bytes": 4096, "scratch_bytes": 16 * 1024 * 1024
    }}});
    std::fs::create_dir_all(root.join(".archon")).unwrap();
    std::fs::write(
        root.join(".archon/config.toml"),
        toml::to_string(&config).unwrap(),
    )
    .unwrap();
    let mut failures = Vec::new();
    for (command, valid) in [
        ("test -f ../current/input", false),
        ("test -f ../recorded/input", true),
    ] {
        let actual = decisions(&set, root, command, TrustedCwd::RepoRoot);
        if actual != [valid; 3] {
            failures.push(format!("{command}: {actual:?}, expected {valid}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[cfg(unix)]
#[test]
fn site_parity_models_only_the_positive_directory_grammar() {
    let set = frozen_set(&[("AC-X-001", "./verify input", true)]);
    let root = set.project.path();
    std::fs::create_dir(root.join("checks")).unwrap();
    std::fs::create_dir(root.join("other")).unwrap();
    std::os::unix::fs::symlink("../other", root.join("checks/link")).unwrap();
    std::fs::write(root.join("checks/verify"), "ready").unwrap();
    std::fs::write(root.join("input"), "ready").unwrap();
    let name = root.file_name().unwrap().to_string_lossy();
    let alias = format!("test -f ../{name}/input");
    let mut cases = vec![
        ("cd checks/link && cd .. && ./verify ../input".into(), true),
        (format!("cd missing || {alias}"), true),
        (format!("cd checks | cat; {alias}"), true),
        ("command cd checks && ./verify ../input".into(), true),
        (format!("cd checks && cat ../../{name}/input"), false),
        (alias.clone(), false),
        (format!("echo ready; {alias}"), false),
    ];
    for change in [
        "cd checks || true",
        "cd checks | cat",
        "cd checks & true",
        "(cd checks)",
        "command cd checks",
        "builtin cd checks",
        "pushd checks",
        "cd -P checks",
        "cd -L checks",
        "cd $place",
        "cd ${place}",
        "cd $(location)",
        "cd `location`",
        "cd",
        "cd -",
        "cd missing",
        "cd checks*",
        "cd ~",
        "cd /",
        "cd checks extra",
        "echo cd",
        "echo pushd",
        "echo popd",
    ] {
        cases.push((format!("{change}; {alias}"), true));
    }
    let mut failures = Vec::new();
    for cwd in [TrustedCwd::ProjectRoot, TrustedCwd::RepoRoot] {
        for (command, valid) in &cases {
            let actual = decisions(&set, root, command, cwd);
            if actual != [*valid; 3] {
                failures.push(format!("{cwd:?} {command}: {actual:?}, expected {valid}"));
            }
        }
    }
    // Unknown relative directories do not disable the independent absolute-root rule.
    let mut candidate = set.contract().acceptance[0].clone();
    candidate.check = AcceptanceCheck::Command {
        command: format!("cd $place; ./verify {}/input", root.display()),
        cwd: TrustedCwd::RepoRoot,
    };
    let forms = super::super::super::live_root::root_forms([root]);
    assert!(super::super::author::live_root_refusal(candidate, &forms).is_err());
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

fn assert_site_parity_unknown_directory(commands: &[&str], symlink: bool) {
    let set = frozen_set(&[("AC-X-001", "./verify input", true)]);
    let root = set.project.path();
    std::fs::create_dir_all(root.join("checks/deep")).unwrap();
    for path in ["input", "checks/verify", "checks/deep/verify"] {
        std::fs::write(root.join(path), "ready").unwrap();
    }
    #[cfg(unix)]
    if symlink {
        std::os::unix::fs::symlink("checks/deep", root.join("link")).unwrap();
    }
    #[cfg(not(unix))]
    assert!(!symlink);
    let name = root.file_name().unwrap().to_string_lossy();
    let control = format!("cd checks && cat ../../{name}/input");
    // Keep the reviewed reproduction and exercise its path-reading counterpart.
    let commands = commands
        .iter()
        .flat_map(|command| {
            [
                (*command).to_owned(),
                command.replace("./verify ", "test -f "),
            ]
        })
        .collect::<Vec<_>>();
    let mut failures = Vec::new();
    for cwd in [TrustedCwd::ProjectRoot, TrustedCwd::RepoRoot] {
        for (command, valid) in commands
            .iter()
            .map(|command| (command.as_str(), true))
            .chain([(control.as_str(), false)])
        {
            let actual = decisions(&set, root, command, cwd);
            if actual != [valid; 3] {
                failures.push(format!("{cwd:?} {command}: {actual:?}, expected {valid}"));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[cfg(unix)]
#[test]
fn site_parity_symlinked_directory_operands_are_unknown() {
    assert_site_parity_unknown_directory(
        &[
            "cd link && ./verify ../../input",
            "cd link/.. && ./verify ../../input",
            "cd link/../deep && ./verify ../../input",
        ],
        true,
    );
}

#[test]
fn site_parity_negated_directory_changes_are_unknown() {
    assert_site_parity_unknown_directory(&["! cd checks; ./verify ../input"], false);
}

#[test]
fn site_parity_compound_directory_changes_are_unknown() {
    assert_site_parity_unknown_directory(
        &[
            "if :; then cd checks; fi; ./verify ../input",
            "for item in checks; do cd checks; done; ./verify ../input",
            "while :; do cd checks; done; ./verify ../input",
            "until :; do cd checks; done; ./verify ../input",
            "case value in value) cd checks;; esac; ./verify ../input",
            "{ cd checks; }; ./verify ../input",
            "change() { cd checks; }; change; ./verify ../input",
        ],
        false,
    );
}

#[test]
fn site_parity_nested_inline_programs_use_their_own_directory() {
    assert_site_parity_unknown_directory(
        &[
            "sh -c 'cd checks && ./verify ../input'",
            "sh -c 'sh -c \"cd checks && ./verify ../input\"'",
            "cd checks; sh -c './verify ../input'",
            "cd $place; sh -c './verify ../missing/input'",
            "sh -c 'cd checks'; ./verify checks/input",
        ],
        false,
    );
    let set = frozen_set(&[("AC-X-001", "./verify input", true)]);
    for cwd in [TrustedCwd::ProjectRoot, TrustedCwd::RepoRoot] {
        let command = "sh -c 'cat ../missing/input'";
        assert_eq!(
            decisions(&set, set.project.path(), command, cwd),
            [false; 3]
        );
        assert_eq!(
            decisions(
                &set,
                set.project.path(),
                "sh -c 'cd checks'; cat ../missing/input",
                cwd
            ),
            [false; 3],
        );
        let mut candidate = set.contract().acceptance[0].clone();
        candidate.check = AcceptanceCheck::Command {
            command: command.into(),
            cwd,
        };
        let defect = archon_workflow::task_set_contract::acceptance_site_path_defect(
            &candidate,
            set.project.path(),
            set.project.path(),
            false,
        )
        .unwrap();
        assert!(defect.contains("../missing/input"), "{defect}");
        assert!(defect.contains("site does not create"), "{defect}");
    }
}

#[test]
fn site_parity_nested_heredoc_programs_use_their_own_directory() {
    assert_site_parity_unknown_directory(&["sh <<'EOF'\ncd checks\n./verify ../input\nEOF"], false);
}

#[test]
fn site_parity_continued_directory_command_is_recognized() {
    assert_site_parity_unknown_directory(
        &[
            "c\\\nd checks && ./verify ../input",
            "\"c\\\nd\" checks && ./verify ../input",
            "${step} checks; ./verify ../missing/input",
        ],
        false,
    );
}

#[test]
fn site_parity_interpreter_directory_changes_are_unknown() {
    assert_site_parity_unknown_directory(
        &[
            "python -c 'import os; os.chdir(\"checks\"); open(\"../input\")'",
            "python -c 'import os; getattr(os, \"chdir\")(\"checks\"); open(\"../input\")'",
            "python -c 'relocate(); open(\"../input\")'",
        ],
        false,
    );
    let set = frozen_set(&[("AC-X-001", "./verify input", true)]);
    for cwd in [TrustedCwd::ProjectRoot, TrustedCwd::RepoRoot] {
        assert_eq!(
            decisions(
                &set,
                set.project.path(),
                "python -c 'open(\"../missing/input\")'",
                cwd
            ),
            [false; 3],
        );
    }
}

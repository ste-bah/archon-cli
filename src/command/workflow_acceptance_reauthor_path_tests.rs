//! Re-author decisions for relative sibling paths.
use super::*;
use archon_workflow::task_set_contract::TrustedCwd;
#[test]
fn a_parent_path_into_the_repository_sibling_is_available_before_judging() {
    let set = frozen_set(&[("AC-X-001", "test -f feature.txt", false)]);
    let project = set.project.path().join("project");
    let repo = set.project.path().join("repository");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::create_dir_all(&repo).unwrap();
    std::fs::write(repo.join("item"), "present").unwrap();
    let mut scope = scope(&set);
    scope.project_root = project;
    scope.repository_root = repo;
    let mut candidate = set.contract().acceptance[0].clone();
    candidate.check = AcceptanceCheck::Command {
        command: "test -f ../repository/item".into(),
        cwd: TrustedCwd::ProjectRoot,
    };
    check_defects::check_defects(&set.contract(), candidate, &scope)
        .expect("the sibling discovery exposes the repository alias to the site");
}
#[test]
fn project_root_can_name_a_file_through_the_repository_sibling_alias() {
    let set = frozen_set(&[("AC-X-001", "true", false)]);
    let project = set.project.path().join("project");
    let repo = set.project.path().join("repository");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::create_dir_all(&repo).unwrap();
    std::fs::write(repo.join("item"), "present").unwrap();
    let mut author_scope = scope(&set);
    author_scope.project_root = project;
    author_scope.repository_root = repo;
    let mut candidate = set.contract().acceptance[0].clone();
    candidate.check = AcceptanceCheck::Command {
        command: "test -f ../repository/item".into(),
        cwd: TrustedCwd::ProjectRoot,
    };
    check_defects::check_defects(&set.contract(), candidate, &author_scope)
        .expect("a sibling copied by the site is available to the project-root check");
}
#[test]
fn repository_root_cannot_name_its_own_repository_alias_through_parent() {
    let set = frozen_set(&[("AC-X-002", "true", false)]);
    let project = set.project.path().join("project");
    let repo = set.project.path().join("repository");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::create_dir_all(&repo).unwrap();
    std::fs::write(repo.join("item"), "present").unwrap();
    let mut author_scope = scope(&set);
    author_scope.project_root = project;
    author_scope.repository_root = repo;
    let mut candidate = set.contract().acceptance[0].clone();
    candidate.check = AcceptanceCheck::Command {
        command: "test -f ../repository/item".into(),
        cwd: TrustedCwd::RepoRoot,
    };
    let finding = check_defects::check_defects(&set.contract(), candidate, &author_scope)
        .expect_err("the repository root is not copied under its own sibling alias");
    assert!(
        finding.contains("repo_root names repository paths relative to repo_root")
            && finding.contains("only when a project_root check names it")
            && finding.contains("not always provided"),
        "{finding}"
    );
}
#[test]
fn parent_paths_that_return_to_the_selected_root_are_refused_for_both_roots() {
    let set = frozen_set(&[("AC-X-001", "true", false)]);
    let base = set.project.path().join("roots");
    let (project, repo) = (base.join("workspace"), base.join("repository"));
    for root in [&project, &repo] {
        std::fs::create_dir_all(root).unwrap();
    }
    let mut author_scope = scope(&set);
    author_scope.project_root = project;
    author_scope.repository_root = repo;
    for path in ["workspace", "repository"] {
        let mut candidate = set.contract().acceptance[0].clone();
        candidate.check = AcceptanceCheck::Command {
            command: format!("test -f ../{path}/item"),
            cwd: if path == "workspace" {
                TrustedCwd::ProjectRoot
            } else {
                TrustedCwd::RepoRoot
            },
        };
        let finding = check_defects::check_defects(&set.contract(), candidate, &author_scope)
            .expect_err("a parent path that re-enters the selected root is not portable");
        let reason = if path == "workspace" {
            "the site names its copy of the project root differently, so `..` and the project directory name do not lead back into it; name the path relative to project_root"
        } else {
            "repo_root names repository paths relative to repo_root; the site links `../<repository dir>` only when a project_root check names it, so this path is not always provided"
        };
        assert!(finding.contains(reason), "{finding}");
    }
}
#[test]
fn a_lexical_parent_step_inside_the_repository_is_not_scanned_as_cross_root() {
    let set = frozen_set(&[("AC-X-003", "true", false)]);
    let project = set.project.path().join("project");
    let repo = set.project.path().join("repository");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::create_dir_all(repo.join("a")).unwrap();
    std::fs::write(repo.join("b"), "present").unwrap();
    let mut author_scope = scope(&set);
    author_scope.project_root = project;
    author_scope.repository_root = repo;
    let mut candidate = set.contract().acceptance[0].clone();
    candidate.check = AcceptanceCheck::Command {
        command: "test -f a/../b".into(),
        cwd: TrustedCwd::RepoRoot,
    };
    check_defects::check_defects(&set.contract(), candidate, &author_scope)
        .expect("a lexical parent step that stays inside the repository is valid");
}
#[cfg(unix)]
#[tokio::test]
async fn author_path_decisions_match_the_real_scratch_site() {
    use archon_workflow::acceptance_scratch::{
        ObserveHooks, ScratchPolicy, observe_commands_hooked,
    };
    use archon_workflow::acceptance_world::{AcceptanceCommandKind, FrozenCommandRef};
    use std::sync::{Arc, atomic::AtomicBool};
    async fn observe(
        policy: &archon_workflow::acceptance_scratch::ScratchPolicy,
        commit: &str,
        contract: &archon_workflow::task_set_contract::AcceptanceContract,
        chain_digest: &str,
        refs: &[FrozenCommandRef],
        evidence: &std::path::Path,
    ) -> archon_workflow::acceptance_scratch::ObservationResult {
        observe_commands_hooked(
            policy,
            commit,
            contract,
            chain_digest,
            refs,
            evidence,
            Arc::new(AtomicBool::new(false)),
            &ObserveHooks::for_resource_limits(Default::default()),
        )
        .await
        .unwrap()
    }
    fn git(path: &std::path::Path, args: &[&str]) {
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(path)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let set = frozen_set(&[
        ("AC-X-001", "true", true),
        ("AC-X-002", "true", true),
        ("AC-X-003", "true", true),
        ("AC-X-004", "true", true),
    ]);
    let project = set.project.path().join("workspace");
    let repository = project.parent().unwrap().join("repository");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::create_dir_all(&repository).unwrap();
    let task_root = project.join("tasks");
    std::fs::create_dir_all(&task_root).unwrap();
    std::fs::write(repository.join("item"), "present").unwrap();
    std::fs::write(project.join("item"), "present").unwrap();
    let project_name = project.file_name().unwrap().to_str().unwrap();
    assert_ne!(project_name, "project");
    let parent_project_command = format!("test -f ../{project_name}/item");
    assert!(!repository.join("missing/data.txt").exists());
    let missing_sibling_status = std::process::Command::new("sh")
        .arg("-c")
        .arg("test -f ../repository/missing/data.txt")
        .current_dir(&project)
        .status()
        .unwrap();
    assert!(!missing_sibling_status.success());
    let parent_project_live_status = std::process::Command::new("sh")
        .arg("-c")
        .arg(&parent_project_command)
        .current_dir(&project)
        .status()
        .unwrap();
    assert!(parent_project_live_status.success());
    git(&repository, &["init", "-q"]);
    git(
        &repository,
        &["config", "user.email", "test@example.invalid"],
    );
    git(&repository, &["config", "user.name", "test"]);
    git(&repository, &["config", "commit.gpgsign", "false"]);
    git(&repository, &["add", "-f", "item"]);
    git(&repository, &["commit", "--allow-empty", "-qm", "initial"]);
    let commit = String::from_utf8(
        std::process::Command::new("git")
            .arg("-C")
            .arg(&repository)
            .args(["rev-parse", "HEAD"])
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap()
    .trim()
    .to_string();
    let commands = [
        ("test -f ../repository/item", TrustedCwd::ProjectRoot),
        ("test -f ../repository/item", TrustedCwd::RepoRoot),
        (
            "test -f ../repository/missing/data.txt",
            TrustedCwd::ProjectRoot,
        ),
        (parent_project_command.as_str(), TrustedCwd::ProjectRoot),
    ];
    let mut contract = set.contract();
    let mut author_scope = scope(&set);
    author_scope.project_root = project.clone();
    author_scope.repository_root = repository.clone();
    for (entry, (command, cwd)) in contract.acceptance.iter_mut().zip(commands) {
        entry.check = AcceptanceCheck::Command {
            command: command.into(),
            cwd,
        };
    }
    for (index, entry) in contract.acceptance.iter().enumerate().skip(1) {
        let finding = check_defects::check_defects(&contract, entry.clone(), &author_scope)
            .expect_err("each refused path shape is rejected before judging");
        let expected_reason = match index {
            1 => "repo_root names repository paths relative to repo_root",
            2 => "discovery reports the path missing at the selected root",
            3 => "the site names its copy of the project root differently",
            _ => unreachable!(),
        };
        assert!(finding.contains(expected_reason), "{finding}");
    }
    check_defects::check_defects(&contract, contract.acceptance[0].clone(), &author_scope)
        .expect("the copied sibling path is accepted");
    let chain_digest =
        archon_workflow::task_set_contract::content_digest(&serde_json::to_vec(&contract).unwrap());
    let refs = contract
        .acceptance
        .iter()
        .zip(commands.iter())
        .map(|(entry, (command, _))| FrozenCommandRef {
            acceptance_id: entry.id.clone(),
            kind: AcceptanceCommandKind::Command,
            chain_digest: chain_digest.clone(),
            command_digest: archon_workflow::task_set_contract::content_digest(command.as_bytes()),
        })
        .collect::<Vec<_>>();
    let scratch = tempfile::tempdir().unwrap();
    let policy: ScratchPolicy = serde_json::from_value(serde_json::json!({
        "repository": repository,
        "project": project,
        "task_root": task_root,
        "scratch_parent": scratch.path(),
        "project_inputs": [],
        "project_input_excludes": [],
        "combined": false,
        "toolchain_path": std::env::var("PATH").unwrap_or_else(|_| "/usr/bin:/bin".into()),
        "environment": {},
        "environment_allowlist": [],
        "timeout_secs": 30,
        "output_bytes": 4096,
        "scratch_bytes": 16 * 1024 * 1024
    }))
    .unwrap();
    let mut parent_project_site_exit = None;
    for (index, reference) in refs.iter().enumerate() {
        let evidence = scratch.path().join(format!("evidence-{index}"));
        let observation = observe(
            &policy,
            &commit,
            &contract,
            &chain_digest,
            std::slice::from_ref(reference),
            &evidence,
        )
        .await;
        let check = &observation.checks[0];
        if index == 3 {
            parent_project_site_exit = check.exit_code;
        }
        if index == 0 {
            assert!(observation.operational_errors.is_empty());
            assert_eq!(check.exit_code, Some(0), "{check:?}");
            assert!(
                observation.check_evidence[0]
                    .sibling_revisions
                    .iter()
                    .any(|item| item.path == std::path::Path::new("../repository"))
            );
        } else {
            assert_ne!(check.exit_code, Some(0), "check {index}: {check:?}");
        }
    }
    assert!(
        parent_project_site_exit.is_some_and(|exit_code| exit_code != 0),
        "the parent-relative project path is absent from the site"
    );
    let shared = observe(
        &policy,
        &commit,
        &contract,
        &chain_digest,
        &refs[..2],
        &scratch.path().join("evidence-shared"),
    )
    .await;
    assert_eq!(
        shared.checks[0].exit_code,
        Some(0),
        "{:?}",
        shared.checks[0]
    );
    assert_eq!(
        shared.checks[1].exit_code,
        Some(0),
        "{:?}",
        shared.checks[1]
    );
}
#[test]
fn a_path_naming_the_project_through_its_parent_is_refused_with_that_reason() {
    let set = frozen_set(&[("AC-X-003", "true", false)]);
    let base = set.project.path().join("nested");
    let project = base.join("parent/child");
    let repo = base.join("repository");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::create_dir_all(&repo).unwrap();
    let mut author_scope = scope(&set);
    author_scope.project_root = project;
    author_scope.repository_root = repo;
    let mut candidate = set.contract().acceptance[0].clone();
    candidate.check = AcceptanceCheck::Command {
        command: "test -f ../child".into(),
        cwd: TrustedCwd::ProjectRoot,
    };
    let finding = check_defects::check_defects(&set.contract(), candidate, &author_scope)
        .expect_err("the site does not lead back into its project copy by directory name");
    assert!(
        finding.contains("the site names its copy of the project root differently"),
        "{finding}"
    );
}
#[test]
fn a_missing_parent_path_into_the_other_root_is_still_refused() {
    let set = frozen_set(&[("AC-X-004", "test -f feature.txt", false)]);
    let project = set.project.path().join("project");
    let repo = set.project.path().join("repo");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::create_dir_all(&repo).unwrap();
    let mut scope = scope(&set);
    scope.project_root = project;
    scope.repository_root = repo;
    let mut candidate = set.contract().acceptance[0].clone();
    candidate.check = AcceptanceCheck::Command {
        command: "test -f ../repo/missing/data.txt".into(),
        cwd: TrustedCwd::ProjectRoot,
    };
    let finding = check_defects::check_defects(&set.contract(), candidate, &scope).unwrap_err();
    assert!(finding.contains("../repo/missing/data.txt"), "{finding}");
    assert!(
        finding.contains("missing at the selected root"),
        "{finding}"
    );
}
#[test]
fn discovered_sibling_paths_are_available_to_a_check() {
    let set = frozen_set(&[("AC-X-001", "check", false)]);
    let project = set.project.path().join("project");
    let repo = set.project.path().join("repo");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::create_dir_all(repo.join("src")).unwrap();
    std::fs::write(repo.join("src/item"), "x").unwrap();
    let mut scope = scope(&set);
    scope.project_root = project;
    scope.repository_root = repo;
    for command in ["test -f ../repo/src/item", "cd ../repo && test -f src/item"] {
        let mut candidate = set.contract().acceptance[0].clone();
        candidate.check = AcceptanceCheck::Command {
            command: command.into(),
            cwd: TrustedCwd::ProjectRoot,
        };
        check_defects::check_defects(&set.contract(), candidate, &scope).unwrap_or_else(
            |finding| panic!("site discovers `{command}` but it was refused: {finding}"),
        );
    }
}
#[test]
fn an_existing_external_sibling_alias_is_not_a_cross_root_refusal() {
    let set = frozen_set(&[("AC-X-005", "test -f feature.txt", false)]);
    let alias = set.project.path().parent().unwrap().join("alias");
    std::fs::create_dir_all(&alias).unwrap();
    std::fs::write(alias.join("feature.txt"), "present").unwrap();
    let candidate = AcceptanceCheck::Command {
        command: "test -f ../alias/feature.txt".into(),
        cwd: TrustedCwd::ProjectRoot,
    };
    let mut entry = set.contract().acceptance[0].clone();
    entry.check = candidate;
    assert!(check_defects::check_defects(&set.contract(), entry, &scope(&set)).is_ok());
}
#[test]
fn nested_roots_allow_targets_that_normalize_inside_the_selected_root() {
    for repository_contains_project in [true, false] {
        let set = frozen_set(&[("AC-X-006", "test -f feature.txt", false)]);
        let base = set.project.path().join("roots");
        let (project, repository) = if repository_contains_project {
            (base.join("repo/project"), base.join("repo"))
        } else {
            (base.join("project"), base.join("project/repo"))
        };
        std::fs::create_dir_all(&project).unwrap();
        std::fs::create_dir_all(&repository).unwrap();
        std::fs::write(project.join("b"), "present").unwrap();
        let mut author_scope = scope(&set);
        author_scope.project_root = project;
        author_scope.repository_root = repository;
        let mut candidate = set.contract().acceptance[0].clone();
        candidate.check = AcceptanceCheck::Command {
            command: "test -f a/../b".into(),
            cwd: TrustedCwd::ProjectRoot,
        };
        assert!(check_defects::check_defects(&set.contract(), candidate, &author_scope).is_ok());
    }
}
#[cfg(unix)]
#[test]
fn symlink_path_discovery_cannot_provide_is_refused_with_its_reason() {
    let set = frozen_set(&[("AC-X-007", "test -f feature.txt", false)]);
    let project = set.project.path().join("project");
    let repo = set.project.path().join("repo");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::create_dir_all(repo.join("subdir")).unwrap();
    std::fs::write(repo.join("data.txt"), "present").unwrap();
    std::os::unix::fs::symlink(repo.join("subdir"), project.join("alias")).unwrap();
    let mut author_scope = scope(&set);
    author_scope.project_root = project;
    author_scope.repository_root = repo;
    let mut candidate = set.contract().acceptance[0].clone();
    candidate.check = AcceptanceCheck::Command {
        command: "test -f ../project/alias/../data.txt".into(),
        cwd: TrustedCwd::ProjectRoot,
    };
    let finding = check_defects::check_defects(&set.contract(), candidate, &author_scope)
        .expect_err("canonical target in the repository root must be refused");
    assert!(
        finding.contains("missing at the selected root"),
        "{finding}"
    );
}
#[test]
fn a_discovered_sibling_in_here_document_program_text_is_available() {
    let set = frozen_set(&[("AC-X-001", "check", false)]);
    let project = set.project.path().join("project");
    let repo = set.project.path().join("repo");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::create_dir_all(repo.join("bin")).unwrap();
    std::fs::write(repo.join("package.manifest"), "x").unwrap();
    std::fs::write(repo.join("bin/tool"), "x").unwrap();
    let mut scope = scope(&set);
    scope.project_root = project;
    scope.repository_root = repo;
    for (command, named) in [
        (
            "runner - <<'PROGRAM'\nMANIFEST = '../repo/package.manifest'\nprint(MANIFEST)\nPROGRAM",
            "../repo/package.manifest",
        ),
        (
            "runner - <<'PROGRAM'\nresult = run(['../repo/bin/tool', '--version'])\nPROGRAM",
            "../repo/bin/tool",
        ),
        (
            "runner - <<'PROGRAM'\nresult = run(['tool','--input','../repo/bin/tool']+args)\nPROGRAM",
            "../repo/bin/tool",
        ),
        (
            "# the tool's own manifest decides\ntest -f ../repo/package.manifest",
            "../repo/package.manifest",
        ),
    ] {
        let mut candidate = set.contract().acceptance[0].clone();
        candidate.check = AcceptanceCheck::Command {
            command: command.into(),
            cwd: TrustedCwd::ProjectRoot,
        };
        check_defects::check_defects(&set.contract(), candidate, &scope).unwrap_or_else(
            |finding| panic!("discovery provides `{named}` but it was refused: {finding}"),
        );
    }
}

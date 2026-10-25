use super::*;
use archon_workflow::acceptance_scratch::{ObserveHooks, ScratchPolicy, observe_commands_hooked};
use archon_workflow::acceptance_world::{AcceptanceCommandKind, FrozenCommandRef};
use archon_workflow::task_set_contract::TrustedCwd;
use std::sync::{Arc, atomic::AtomicBool};

#[test]
fn an_unresolvable_root_is_refused_with_its_actual_reason() {
    let set = frozen_set(&[("AC-X-001", "true", false)]);
    let missing = set.project.path().join("not-created");
    let refusal = archon_workflow::acceptance_scratch::site_path_alone(
        &missing,
        &missing,
        "test -f ../input",
        TrustedCwd::ProjectRoot,
        std::path::Path::new("../input"),
        false,
    )
    .expect_err("an unavailable root cannot model a prepared site");
    assert!(refusal.site_target == std::path::Path::new("<site>"));
    assert!(
        refusal
            .reason
            .as_deref()
            .is_some_and(|reason| reason.contains("project root cannot be resolved")),
        "{refusal:?}"
    );
}

#[test]
fn repository_root_path_to_project_root_is_refused() {
    let set = frozen_set(&[("AC-X-001", "true", false)]);
    let base = set.project.path().join("roots");
    let project = base.join("workspace");
    let repo = base.join("repository");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::create_dir_all(&repo).unwrap();
    std::fs::write(project.join("item"), "present").unwrap();
    let mut author_scope = scope(&set);
    author_scope.project_root = project;
    author_scope.repository_root = repo;
    let mut candidate = set.contract().acceptance[0].clone();
    candidate.check = AcceptanceCheck::Command {
        command: "test -f ../workspace/item".into(),
        cwd: TrustedCwd::RepoRoot,
    };
    let finding = check_defects::check_defects(&set.contract(), candidate, &author_scope)
        .expect_err("the site refuses a repository-root path into the project root");
    assert!(finding.contains("`../workspace`"), "{finding}");
    assert!(finding.contains("<site>/workspace/item"), "{finding}");
    assert!(
        finding.contains(archon_workflow::acceptance_scratch::faults::SIBLING_ROOT_REFUSED),
        "{finding}"
    );
}

#[test]
fn a_whole_check_refusal_names_the_sibling_that_caused_it() {
    let set = frozen_set(&[("AC-X-002", "true", false)]);
    let project = set.project.path().join("workspace");
    let repository = set.project.path().join("repository");
    let other = set.project.path().join("other");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::create_dir_all(&repository).unwrap();
    std::fs::create_dir_all(&other).unwrap();
    std::fs::write(project.join("item"), "project input").unwrap();
    std::fs::write(other.join("item"), "other input").unwrap();
    let mut author_scope = scope(&set);
    author_scope.project_root = project;
    author_scope.repository_root = repository;
    let mut candidate = set.contract().acceptance[0].clone();
    candidate.check = AcceptanceCheck::Command {
        command: "test -e ../other/item && test -e ../workspace/item".into(),
        cwd: TrustedCwd::RepoRoot,
    };
    let finding = check_defects::check_defects(&set.contract(), candidate, &author_scope)
        .expect_err("the later project alias refuses the whole check");
    assert!(finding.contains("`../workspace`"), "{finding}");
    assert!(!finding.contains("`../other/item`"), "{finding}");
    assert!(finding.contains("<site>/workspace/item"), "{finding}");
    assert!(
        finding.contains(archon_workflow::acceptance_scratch::faults::SIBLING_ROOT_REFUSED),
        "{finding}"
    );
}

#[test]
fn a_cross_root_path_in_here_document_finding_names_the_path() {
    let set = frozen_set(&[("AC-X-001", "true", false)]);
    let project = set.project.path().join("workspace");
    let repo = set.project.path().join("repository");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::create_dir_all(&repo).unwrap();
    let mut author_scope = scope(&set);
    author_scope.project_root = project;
    author_scope.repository_root = repo;
    let named = "../external/item";
    let mut candidate = set.contract().acceptance[0].clone();
    candidate.check = AcceptanceCheck::Command {
        command: format!("runner - <<'PROGRAM'\nopen('{named}')\nPROGRAM"),
        cwd: TrustedCwd::ProjectRoot,
    };
    let finding = check_defects::check_defects(&set.contract(), candidate, &author_scope)
        .expect_err("the here-document path has no site copy");
    assert!(finding.contains(named), "{finding}");
    assert!(finding.contains("<site>/external/item"), "{finding}");
}

#[test]
fn repository_root_path_cannot_leave_and_reenter_a_directory_named_repo() {
    let set = frozen_set(&[("AC-X-001", "true", false)]);
    let base = set.project.path().join("roots");
    for (index, repository_name) in ["repository", "repo"].into_iter().enumerate() {
        let project = base.join(format!("workspace-{index}"));
        let repository = base.join(repository_name);
        std::fs::create_dir_all(&project).unwrap();
        std::fs::create_dir_all(&repository).unwrap();
        std::fs::write(repository.join("item"), "present").unwrap();
        let mut author_scope = scope(&set);
        author_scope.project_root = project;
        author_scope.repository_root = repository;
        let mut candidate = set.contract().acceptance[0].clone();
        candidate.check = AcceptanceCheck::Command {
            command: "test -f ../repo/item".into(),
            cwd: TrustedCwd::RepoRoot,
        };
        let finding = check_defects::check_defects(&set.contract(), candidate, &author_scope)
            .expect_err("the named path leaves the live repository root");
        assert!(finding.contains("leaves the repository root"), "{finding}");
    }
}

#[cfg(unix)]
#[tokio::test]
async fn author_path_decisions_agree_with_one_check_sites_across_root_layouts() {
    fn git(root: &std::path::Path, args: &[&str]) {
        let result = std::process::Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}: {}",
            args.join(" "),
            String::from_utf8_lossy(&result.stderr)
        );
    }
    async fn site_passes(
        policy: &ScratchPolicy,
        commit: &str,
        base_contract: &AcceptanceContract,
        entry: &AcceptanceCriterion,
        scratch: &std::path::Path,
        index: usize,
    ) -> (bool, String) {
        let mut contract = base_contract.clone();
        contract.acceptance[0] = entry.clone();
        let chain_digest = archon_workflow::task_set_contract::content_digest(
            &serde_json::to_vec(&contract).unwrap(),
        );
        let command = match &entry.check {
            AcceptanceCheck::Command { command, .. } => command,
            _ => unreachable!(),
        };
        let reference = FrozenCommandRef {
            acceptance_id: entry.id.clone(),
            kind: AcceptanceCommandKind::Command,
            chain_digest: chain_digest.clone(),
            command_digest: archon_workflow::task_set_contract::content_digest(command.as_bytes()),
        };
        let result = observe_commands_hooked(
            policy,
            commit,
            &contract,
            &chain_digest,
            &[reference],
            &scratch.join(format!("evidence-{index}")),
            Arc::new(AtomicBool::new(false)),
            &ObserveHooks::for_resource_limits(Default::default()),
        )
        .await
        .unwrap();
        (
            result
                .checks
                .first()
                .is_some_and(|check| check.exit_code == Some(0)),
            format!(
                "checks={:?}, errors={:?}",
                result.checks, result.operational_errors
            ),
        )
    }

    let layouts = [
        "neutral",
        "site_names",
        "repository_inside_project",
        "project_inside_repository",
        "same_path",
    ];
    for (cache_index, build_cache) in [false, true].into_iter().enumerate() {
        for (layout_index, layout) in layouts.into_iter().enumerate() {
            let set = frozen_set(&[("AC-X-001", "true", true)]);
            let base = set
                .project
                .path()
                .join("matrix")
                .join(format!("cache-{cache_index}"));
            let (project, repository) = match layout {
                "neutral" => (base.join("workspace"), base.join("repository")),
                "site_names" => (base.join("project"), base.join("repo")),
                "repository_inside_project" => {
                    (base.join("workspace"), base.join("workspace/repository"))
                }
                "project_inside_repository" => {
                    (base.join("repository/workspace"), base.join("repository"))
                }
                "same_path" => (base.join("shared"), base.join("shared")),
                _ => unreachable!(),
            };
            std::fs::create_dir_all(&project).unwrap();
            std::fs::create_dir_all(&repository).unwrap();
            std::fs::create_dir_all(project.join("tasks")).unwrap();
            std::fs::write(project.join("item"), "project input").unwrap();
            std::fs::write(repository.join("item"), "repository input").unwrap();
            std::fs::write(repository.join("Manifest-like.toml"), "manifest input").unwrap();
            std::fs::create_dir_all(repository.join("src")).unwrap();
            std::fs::write(repository.join("src/committed.rs"), "committed input").unwrap();
            std::fs::write(repository.join("src/uncommitted.rs"), "uncommitted input").unwrap();
            let loose_file = project.parent().unwrap().join("loose.txt");
            std::fs::write(&loose_file, "sibling file").unwrap();
            let collision_paths = [
                "target",
                "home",
                "tmp",
                "process-groups",
                "cargo-home",
                "project-baseline",
            ];
            for name in collision_paths {
                let collision = project.parent().unwrap().join(name);
                std::fs::create_dir_all(&collision).unwrap();
                std::fs::write(collision.join("item"), "collision input").unwrap();
            }
            let other_git = project.parent().unwrap().join("othergit");
            std::fs::create_dir_all(&other_git).unwrap();
            std::fs::write(other_git.join("item"), "other git sibling").unwrap();
            let external = base.join("external");
            let missing = base.join("missing");
            std::fs::create_dir_all(&external).unwrap();
            std::fs::write(external.join("item"), "external input").unwrap();
            if layout == "site_names" {
                std::fs::create_dir_all(external.join("sub")).unwrap();
                std::fs::write(external.join("data.txt"), "external symlink target").unwrap();
                #[cfg(unix)]
                std::os::unix::fs::symlink(external.join("sub"), project.join("alias")).unwrap();
            }
            std::fs::create_dir_all(&missing).unwrap();
            if !repository.join(".git").exists() {
                git(&repository, &["init", "-q"]);
                git(
                    &repository,
                    &["config", "user.email", "test@example.invalid"],
                );
                git(&repository, &["config", "user.name", "test"]);
                git(&repository, &["config", "commit.gpgsign", "false"]);
                git(&repository, &["add", "-f", "item"]);
                git(&repository, &["add", "-f", "Manifest-like.toml"]);
                git(&repository, &["add", "-f", "src/committed.rs"]);
                if loose_file.starts_with(&repository) {
                    git(&repository, &["add", "-f", "loose.txt"]);
                }
                git(&repository, &["commit", "-qm", "initial"]);
            }
            git(&other_git, &["init", "-q"]);
            git(
                &other_git,
                &["config", "user.email", "test@example.invalid"],
            );
            git(&other_git, &["config", "user.name", "test"]);
            git(&other_git, &["config", "commit.gpgsign", "false"]);
            git(&other_git, &["add", "-f", "item"]);
            git(&other_git, &["commit", "-qm", "initial"]);
            let uncommitted = repository.join("new-item");
            std::fs::write(&uncommitted, "not in the committed copy").unwrap();
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
            let policy: ScratchPolicy = serde_json::from_value(serde_json::json!({
                "repository": repository,
                "project": project,
                "task_root": project.join("tasks"),
                "scratch_parent": set.project.path().join("sites"),
                "build_cache": build_cache.then(|| set.project.path().join("sites/cache")),
                "project_inputs": ["item"],
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
            std::fs::create_dir_all(&policy.scratch_parent).unwrap();
            let mut scope = scope(&set);
            scope.project_root = policy.project.clone();
            scope.repository_root = policy.repository.clone();
            scope.build_cache = build_cache;
            let repo_name = repository.file_name().unwrap().to_string_lossy();
            let project_name = project.file_name().unwrap().to_string_lossy();
            let mut commands = vec![
                ("test -e item".into(), TrustedCwd::ProjectRoot),
                // A single file beside the selected root is copied at its alias.
                ("test -f ../loose.txt".into(), TrustedCwd::ProjectRoot),
                // These names already belong to the one-check site.
                if build_cache {
                    (
                        "test -d ../target && test ! -e ../target/out/bin".into(),
                        TrustedCwd::ProjectRoot,
                    )
                } else {
                    ("test -f ../target/item".into(), TrustedCwd::ProjectRoot)
                },
                ("test -f ../home/item".into(), TrustedCwd::ProjectRoot),
                ("test -f ../tmp/item".into(), TrustedCwd::ProjectRoot),
                ("test -f ../cargo-home/item".into(), TrustedCwd::ProjectRoot),
                (
                    "test -f ../project-baseline/item".into(),
                    TrustedCwd::ProjectRoot,
                ),
                (
                    "test -f ../process-groups/item".into(),
                    TrustedCwd::ProjectRoot,
                ),
            ];
            match layout {
                "repository_inside_project" => {
                    commands.push(("test -f ../item".into(), TrustedCwd::RepoRoot))
                }
                "project_inside_repository" => {
                    commands.push(("test ! -e ../othergit/item".into(), TrustedCwd::ProjectRoot));
                    // Both the top-level file and a subdirectory alias come from
                    // the committed repository copy at this layout.
                    commands.push(("test -f ../item".into(), TrustedCwd::ProjectRoot));
                    commands.push((
                        "test ! -e ../src/uncommitted.rs".into(),
                        TrustedCwd::ProjectRoot,
                    ));
                    commands.push((
                        "test -f ../src/committed.rs".into(),
                        TrustedCwd::ProjectRoot,
                    ));
                }
                "same_path" => {}
                _ => {
                    commands.push((format!("test -e ../{repo_name}/item"), TrustedCwd::RepoRoot));
                    commands.push((
                        format!("test -e ../{project_name}/item"),
                        TrustedCwd::ProjectRoot,
                    ));
                    commands.push((format!("cd ../{project_name}"), TrustedCwd::RepoRoot));
                    commands.push(("test -e ../external/item".into(), TrustedCwd::ProjectRoot));
                    commands.push(("test -e ../missing/item".into(), TrustedCwd::ProjectRoot));
                    // This Git sibling is not the declared repository.
                    commands.push(("test -f ../othergit/item".into(), TrustedCwd::ProjectRoot));
                    commands.push((
                        format!("test -f ../{repo_name}/item"),
                        TrustedCwd::ProjectRoot,
                    ));
                    commands.push((
                        format!("test -f ../{project_name}/item"),
                        TrustedCwd::RepoRoot,
                    ));
                    commands.push((
                        format!("test ! -e ../{repo_name}/src/uncommitted.rs"),
                        TrustedCwd::ProjectRoot,
                    ));
                    commands.push((
                        format!("test -f ../{repo_name}/src/committed.rs"),
                        TrustedCwd::ProjectRoot,
                    ));
                    commands.push((
                    format!(
                        "test -f ../{repo_name}/Manifest-like.toml && test -d ../{repo_name}/target && test ! -e ../{repo_name}/target/out/bin"
                    ),
                    TrustedCwd::ProjectRoot,
                ));
                    if layout == "site_names" {
                        commands.push((
                            "test -f ../project/alias/../data.txt".into(),
                            TrustedCwd::ProjectRoot,
                        ));
                    }
                }
            }
            for (case_index, (command, cwd)) in commands.into_iter().enumerate() {
                let mut entry = set.contract().acceptance[0].clone();
                entry.check = AcceptanceCheck::Command { command, cwd };
                let author_accepts =
                    check_defects::check_defects(&set.contract(), entry.clone(), &scope).is_ok();
                let (observed, observation) = site_passes(
                    &policy,
                    &commit,
                    &set.contract(),
                    &entry,
                    &set.project.path().join("observations"),
                    (cache_index * 10 + layout_index) * 20 + case_index,
                )
                .await;
                assert_eq!(
                    author_accepts, observed,
                    "layout={layout}, case={case_index}, command={:?}, cwd={cwd:?}, {observation}",
                    entry.check
                );
            }
        }
    }
}

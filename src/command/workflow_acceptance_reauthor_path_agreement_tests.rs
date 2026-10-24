use super::*;
use archon_workflow::acceptance_scratch::{ObserveHooks, ScratchPolicy, observe_commands_hooked};
use archon_workflow::acceptance_world::{AcceptanceCommandKind, FrozenCommandRef};
use archon_workflow::task_set_contract::TrustedCwd;
use std::sync::{Arc, atomic::AtomicBool};

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
    assert!(finding.contains("../workspace/item"), "{finding}");
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
    for (layout_index, layout) in layouts.into_iter().enumerate() {
        let set = frozen_set(&[("AC-X-001", "true", true)]);
        let base = set.project.path().join("matrix");
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
        let external = base.join("external");
        let missing = base.join("missing");
        std::fs::create_dir_all(&external).unwrap();
        std::fs::write(external.join("item"), "external input").unwrap();
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
            git(&repository, &["commit", "-qm", "initial"]);
        }
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
        let repo_name = repository.file_name().unwrap().to_string_lossy();
        let project_name = project.file_name().unwrap().to_string_lossy();
        let commands = [
            (
                format!("test -e ../{repo_name}/item"),
                TrustedCwd::ProjectRoot,
            ),
            (
                format!("test -e ../{repo_name}/new-item"),
                TrustedCwd::ProjectRoot,
            ),
            (
                format!("test -e ../{project_name}/item"),
                TrustedCwd::ProjectRoot,
            ),
            (format!("test -e ../{repo_name}/item"), TrustedCwd::RepoRoot),
            (
                format!("test -e ../{project_name}/item"),
                TrustedCwd::RepoRoot,
            ),
            (format!("cd ../{project_name}"), TrustedCwd::RepoRoot),
            ("cd ..".into(), TrustedCwd::ProjectRoot),
            ("test -e ../external/item".into(), TrustedCwd::ProjectRoot),
            ("test -e ../missing/item".into(), TrustedCwd::ProjectRoot),
            ("test -e item".into(), TrustedCwd::ProjectRoot),
        ];
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
                layout_index * 10 + case_index,
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

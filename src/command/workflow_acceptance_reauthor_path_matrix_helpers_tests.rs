use super::*;
use archon_workflow::acceptance_scratch::{ObserveHooks, ScratchPolicy, observe_commands_hooked};
use archon_workflow::acceptance_world::{AcceptanceCommandKind, FrozenCommandRef};
use std::path::{Path, PathBuf};
use std::sync::{Arc, atomic::AtomicBool};

pub(super) fn copy_key(
    project: &Path,
    repository: &Path,
    command: &str,
    cwd: TrustedCwd,
    build_cache: bool,
) -> (u8, Vec<(PathBuf, PathBuf)>) {
    let (root, copied_root, cwd_key) = match cwd {
        TrustedCwd::ProjectRoot => (project, "project", 0),
        TrustedCwd::RepoRoot => (repository, "repo", 1),
    };
    let relative = |to: &Path| {
        let left = root.components().collect::<Vec<_>>();
        let right = to.components().collect::<Vec<_>>();
        let common = left.iter().zip(&right).take_while(|(a, b)| a == b).count();
        let mut path = PathBuf::new();
        for _ in common..left.len() {
            path.push("..");
        }
        for component in &right[common..] {
            path.push(component.as_os_str());
        }
        path
    };
    let mut copied =
        archon_workflow::acceptance_sibling_paths::discover(root, &[command.to_string()])
            .into_iter()
            .filter_map(|sibling| {
                let path = relative(&sibling.alias);
                let mut destination = vec![copied_root.to_string()];
                for component in path.components() {
                    match component {
                        std::path::Component::ParentDir => {
                            destination.pop();
                        }
                        std::path::Component::Normal(name) => {
                            destination.push(name.to_string_lossy().into_owned());
                        }
                        std::path::Component::CurDir => {}
                        _ => return None,
                    }
                }
                let name = destination.first()?.as_str();
                let internal = archon_workflow::acceptance_scratch::SITE_INTERNAL_DIRECTORIES
                    .iter()
                    .any(|directory| {
                        *directory == name
                            && !(build_cache
                                && *directory
                                    == archon_workflow::acceptance_scratch::SITE_TARGET_DIRECTORY)
                    });
                let repository_copy = name
                    == archon_workflow::acceptance_scratch::SITE_REPOSITORY_DIRECTORY
                    && sibling.source.starts_with(repository);
                (!internal || repository_copy).then_some((path, sibling.source))
            })
            .collect::<Vec<_>>();
    copied.sort();
    (cwd_key, copied)
}

pub(super) async fn run_layout(layout: &str, layout_index: usize) {
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
    async fn sites_pass(
        policy: &ScratchPolicy,
        commit: &str,
        base_contract: &AcceptanceContract,
        entries: &[(usize, AcceptanceCriterion)],
        scratch: &std::path::Path,
        index: usize,
    ) -> Vec<(usize, bool, String)> {
        let mut contract = base_contract.clone();
        contract.acceptance = entries.iter().map(|(_, entry)| entry.clone()).collect();
        contract.gap_policy.permitted_acceptance_ids = contract
            .acceptance
            .iter()
            .filter(|entry| entry.gap_permitted)
            .map(|entry| entry.id.clone())
            .collect();
        let chain_digest = archon_workflow::task_set_contract::content_digest(
            &serde_json::to_vec(&contract).unwrap(),
        );
        let references = entries
            .iter()
            .map(|(_, entry)| {
                let command = match &entry.check {
                    AcceptanceCheck::Command { command, .. } => command,
                    _ => unreachable!(),
                };
                FrozenCommandRef {
                    acceptance_id: entry.id.clone(),
                    kind: AcceptanceCommandKind::Command,
                    chain_digest: chain_digest.clone(),
                    command_digest: archon_workflow::task_set_contract::content_digest(
                        command.as_bytes(),
                    ),
                }
            })
            .collect::<Vec<_>>();
        let result = observe_commands_hooked(
            policy,
            commit,
            &contract,
            &chain_digest,
            &references,
            &scratch.join(format!("evidence-{index}")),
            archon_workflow::acceptance_scratch::ObserveControl::new(
                Arc::new(AtomicBool::new(false)),
                ObserveHooks::for_resource_limits(Default::default()),
            ),
        )
        .await
        .unwrap();
        entries
            .iter()
            .zip(&result.checks)
            .map(|((case_index, _), check)| {
                (
                    *case_index,
                    check.exit_code == Some(0),
                    format!("check={check:?}, errors={:?}", result.operational_errors),
                )
            })
            .collect()
    }
    let set = frozen_set(&[("AC-X-001", "true", true)]);
    for (cache_index, build_cache) in [false, true].into_iter().enumerate() {
        let base = set
            .project
            .path()
            .join("matrix")
            .join(format!("cache-{cache_index}"))
            .join(layout);
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
            if !project.join("alias").exists() {
                std::os::unix::fs::symlink(external.join("sub"), project.join("alias")).unwrap();
            }
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
        if !other_git.join(".git").exists() {
            git(&other_git, &["init", "-q"]);
            git(
                &other_git,
                &["config", "user.email", "test@example.invalid"],
            );
            git(&other_git, &["config", "user.name", "test"]);
            git(&other_git, &["config", "commit.gpgsign", "false"]);
            git(&other_git, &["add", "-f", "item"]);
            git(&other_git, &["commit", "-qm", "initial"]);
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
        let mut groups = std::collections::BTreeMap::new();
        let mut expected = Vec::with_capacity(commands.len());
        for (case_index, (command, cwd)) in commands.into_iter().enumerate() {
            let mut entry = set.contract().acceptance[0].clone();
            entry.check = AcceptanceCheck::Command {
                command: command.clone(),
                cwd,
            };
            let author_accepts =
                check_defects::check_defects(&set.contract(), entry.clone(), &scope).is_ok();
            expected.push((author_accepts, entry.check.clone()));
            groups
                .entry(path_matrix_helpers::copy_key(
                    &project,
                    &repository,
                    &command,
                    cwd,
                    build_cache,
                ))
                .or_insert_with(Vec::new)
                .push((case_index, entry));
        }
        let mut observed = vec![None; expected.len()];
        for (group_index, entries) in groups.into_values().enumerate() {
            let entries = entries
                .into_iter()
                .map(|(case_index, mut entry)| {
                    entry.id = format!("AC-X-{:03}", case_index + 1);
                    (case_index, entry)
                })
                .collect::<Vec<_>>();
            for (case_index, actual, observation) in sites_pass(
                &policy,
                &commit,
                &set.contract(),
                &entries,
                &set.project.path().join("observations"),
                (cache_index * 10 + layout_index) * 20 + group_index,
            )
            .await
            {
                observed[case_index] = Some((actual, observation));
            }
        }
        for (case_index, ((author_accepts, check), observed)) in
            expected.into_iter().zip(observed).enumerate()
        {
            let (observed, observation) = observed.expect("every case has one site result");
            assert_eq!(
                author_accepts, observed,
                "layout={layout}, case={case_index}, command={check:?}, {observation}"
            );
        }
    }
}

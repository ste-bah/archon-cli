use super::*;
use std::path::Component;

/// Static package selectors are checked even when an earlier new test stops the script.
pub(in crate::command::workflow_task_set) fn package_findings(
    contract: &AcceptanceContract,
    repository: &Path,
    project: &Path,
    base: &str,
    combined: bool,
) -> BTreeMap<String, String> {
    let mut findings: BTreeMap<String, Vec<(String, String)>> = BTreeMap::new();
    let Some(tree) = git(repository, &["ls-tree", "-r", "--name-only", base]) else {
        return BTreeMap::new();
    };
    let mut packages = Vec::new();
    let mut workspaces = BTreeMap::new();
    for path in tree.lines().filter(|path| path.ends_with("Cargo.toml")) {
        let Some(text) = git(repository, &["show", &format!("{base}:{path}")]) else {
            return BTreeMap::new();
        };
        let Ok(value) = toml::from_str::<toml::Value>(&text) else {
            return BTreeMap::new();
        };
        let directory = Path::new(path).parent().unwrap_or(Path::new(""));
        if let Some(workspace) = value.get("workspace") {
            workspaces.insert(directory.to_path_buf(), workspace.clone());
        }
        if let Some(name) = value
            .get("package")
            .and_then(|p| p.get("name"))
            .and_then(toml::Value::as_str)
        {
            packages.push((directory.to_path_buf(), name.to_string()));
        }
    }
    for entry in entries(contract) {
        let archon_workflow::task_set_contract::AcceptanceCheck::Command { command, cwd } =
            &entry.check
        else {
            continue;
        };
        let root = match cwd {
            archon_workflow::task_set_contract::TrustedCwd::RepoRoot => repository,
            archon_workflow::task_set_contract::TrustedCwd::ProjectRoot => project,
        };
        for (program, args, directories) in
            super::super::executability::test_inventory::located_shell_commands(command)
        {
            if program != "cargo"
                || !args.first().is_some_and(|arg| {
                    matches!(
                        arg.as_str(),
                        "test"
                            | "build"
                            | "check"
                            | "run"
                            | "bench"
                            | "doc"
                            | "clippy"
                            | "rustc"
                            | "rustdoc"
                            | "tree"
                            | "clean"
                    )
                })
            {
                continue;
            }
            let Some(directories) = directories else {
                continue;
            };
            let mut cwd = root.to_path_buf();
            for directory in directories {
                cwd.push(directory);
            }
            let mut copies = archon_workflow::acceptance_scratch::repository_copy_roots(
                repository, project, combined,
            );
            // Prefer the specific root when the live project and repository are nested.
            copies.sort_by_key(|copy| std::cmp::Reverse(copy.components().count()));
            let Some(cwd) = copies.iter().find_map(|copy| package_cwd(&cwd, copy)) else {
                continue;
            };
            let mut cwd = PathBuf::from(cwd);
            if let Some(manifest) = args.iter().enumerate().find_map(|(i, arg)| {
                arg.strip_prefix("--manifest-path=")
                    .map(str::to_string)
                    .or_else(|| {
                        (arg == "--manifest-path")
                            .then(|| args.get(i + 1).cloned())
                            .flatten()
                    })
            }) {
                let Some(manifest) = normalized(&cwd.join(manifest)) else {
                    continue;
                };
                cwd = Path::new(&manifest)
                    .parent()
                    .unwrap_or(Path::new(""))
                    .to_path_buf();
            }
            let cwd = cwd.as_path();
            let workspace = workspaces
                .keys()
                .filter(|path| cwd.starts_with(path))
                .max_by_key(|path| path.components().count())
                .map(PathBuf::as_path)
                .or_else(|| {
                    packages
                        .iter()
                        .map(|(path, _)| path)
                        .filter(|path| cwd.starts_with(path))
                        .max_by_key(|path| path.components().count())
                        .map(PathBuf::as_path)
                })
                .unwrap_or(cwd);
            let members: Vec<_> = packages
                .iter()
                .filter(|(path, _)| {
                    if path == workspace {
                        return true;
                    }
                    let Some(relative) = path.strip_prefix(workspace).ok() else {
                        return false;
                    };
                    let Some(config) = workspaces.get(workspace) else {
                        return false;
                    };
                    let matches = |key: &str| {
                        config
                            .get(key)
                            .and_then(toml::Value::as_array)
                            .is_some_and(|patterns| {
                                patterns
                                    .iter()
                                    .filter_map(toml::Value::as_str)
                                    .any(|pattern| {
                                        globset::Glob::new(pattern).ok().is_some_and(|glob| {
                                            glob.compile_matcher().is_match(relative)
                                        })
                                    })
                            })
                    };
                    matches("members") && !matches("exclude")
                })
                .collect();
            let mut real: Vec<_> = members.iter().map(|(_, name)| name.as_str()).collect();
            real.sort();
            real.dedup();
            for name in args
                .iter()
                .enumerate()
                .skip(1)
                .take_while(|(_, arg)| *arg != "--")
                .filter_map(|(index, arg)| {
                    if matches!(arg.as_str(), "-p" | "--package") {
                        args.get(index + 1).map(String::as_str)
                    } else {
                        arg.strip_prefix("--package=")
                            .or_else(|| arg.strip_prefix("-p").filter(|s| !s.is_empty()))
                    }
                })
            {
                if name.contains(['$', '*', '?', '@', ':']) || real.contains(&name) {
                    continue;
                }
                let mistaken = members.iter().find(|(path, real)| {
                    path.starts_with(workspace)
                        && real != name
                        && (if path.as_os_str().is_empty() {
                            repository.file_name()
                        } else {
                            path.file_name()
                        })
                        .is_some_and(|directory| directory == name)
                });
                if let Some((path, real)) = mistaken {
                    let dir = if path.as_os_str().is_empty() {
                        repository
                            .file_name()
                            .unwrap()
                            .to_string_lossy()
                            .into_owned()
                    } else {
                        archon_shell::paths::stored_text(&path)
                    };
                    let detail =
                        format!("`-p {name}` names directory {dir}; its package is named {real}");
                    let details = findings.entry(entry.id.clone()).or_insert_with(Vec::new);
                    if !details.iter().any(|(seen, _)| seen == name) {
                        details.push((name.to_string(), detail));
                    }
                }
            }
        }
    }
    findings
        .into_iter()
        .map(|(id, details)| {
            let text = details
                .into_iter()
                .map(|(_, detail)| detail)
                .collect::<Vec<_>>()
                .join("; ");
            let finding = format!("check '{id}': cannot pass as written: {text}");
            (id, finding)
        })
        .collect()
}

/// Reduce literal shell directory changes before inspecting base-tree manifests.
fn package_cwd(path: &Path, repository: &Path) -> Option<PathBuf> {
    let mut resolved = PathBuf::new();
    for part in path.components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir => {
                if !resolved.pop() {
                    return None;
                }
            }
            part => resolved.push(part.as_os_str()),
        }
    }
    resolved
        .strip_prefix(repository)
        .ok()
        .map(Path::to_path_buf)
}

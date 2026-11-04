//! Base-tree evidence for an invocation that cannot locate its own setup.
use super::*;
use std::path::Component;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Missing {
    Package,
    Test,
    Bin,
    Path,
}

fn missing(output: &str) -> Option<(Missing, String)> {
    for (kind, pattern) in [
        (
            Missing::Package,
            r#"package ID specification [`'\"]([^`'\"]+)[`'\"] did not match any packages"#,
        ),
        (
            Missing::Test,
            r#"no test target named [`'\"]([^`'\"]+)[`'\"]"#,
        ),
        (
            Missing::Bin,
            r#"no bin target named [`'\"]([^`'\"]+)[`'\"]"#,
        ),
        (Missing::Path, r#"file or directory not found: ([^\n]+)"#),
        (
            Missing::Path,
            r#"manifest path [`'\"]([^`'\"]+)[`'\"] does not exist"#,
        ),
    ] {
        // Patterns are toolchain grammar, never names from a particular check.
        if let Some(capture) = regex::Regex::new(pattern)
            .expect("static pattern")
            .captures(output)
        {
            return Some((
                kind,
                capture[1].trim().trim_matches(['\'', '"', '`']).to_string(),
            ));
        }
    }
    output
        .contains("could not find `Cargo.toml`")
        .then(|| (Missing::Path, "Cargo.toml".into()))
}

fn git(repository: &Path, args: &[&str]) -> Option<String> {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(repository)
        .args(args)
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}

fn normalized(path: &Path) -> Option<String> {
    let mut parts = Vec::new();
    for part in path.components() {
        match part {
            Component::Normal(part) => parts.push(part.to_str()?),
            Component::CurDir => {}
            Component::ParentDir => {
                parts.pop()?;
            }
            _ => return None,
        }
    }
    Some(parts.join("/"))
}

/// `None` means the base could not be inspected: never manufacture absence.
fn exists(
    repository: &Path,
    commit: &str,
    kind: Missing,
    name: &str,
    command: &str,
    cwd: &Path,
) -> Option<bool> {
    let paths = git(repository, &["ls-tree", "-r", "--name-only", commit])?;
    let paths: Vec<_> = paths.lines().collect();
    if kind == Missing::Path {
        let mut cwd = cwd.to_path_buf();
        for invocation in super::super::executability::test_inventory::shell_commands(command) {
            if invocation.0 == "cd" {
                let path = invocation.1.first()?;
                if path.contains(['$', '~', '`']) || path == "-" {
                    return None;
                }
                cwd.push(path);
            }
        }
        let path = normalized(&cwd.join(name.split("::").next()?))?;
        return Some(
            paths
                .iter()
                .any(|entry| *entry == path || entry.starts_with(&format!("{path}/"))),
        );
    }
    let commands = super::super::executability::test_inventory::shell_commands(command);
    let selected = commands
        .iter()
        .flat_map(|(_, args)| args.windows(2))
        .find(|args| matches!(args[0].as_str(), "-p" | "--package"))
        .map(|args| args[1].as_str());
    for manifest in paths.iter().filter(|path| path.ends_with("Cargo.toml")) {
        let text = git(repository, &["show", &format!("{commit}:{manifest}")])?;
        let value: toml::Value = toml::from_str(&text).ok()?;
        let package = value
            .get("package")
            .and_then(|p| p.get("name"))
            .and_then(toml::Value::as_str);
        if kind == Missing::Package && package == Some(name) {
            return Some(true);
        }
        if selected.is_some() && package != selected {
            continue;
        }
        let section = match kind {
            Missing::Test => "test",
            Missing::Bin => "bin",
            _ => continue,
        };
        if value
            .get(section)
            .and_then(toml::Value::as_array)
            .is_some_and(|targets| {
                targets
                    .iter()
                    .any(|target| target.get("name").and_then(toml::Value::as_str) == Some(name))
            })
        {
            return Some(true);
        }
        let directory = Path::new(manifest).parent()?;
        let implicit = match kind {
            Missing::Test => directory.join(format!("tests/{name}.rs")),
            Missing::Bin => directory.join(format!("src/bin/{name}.rs")),
            _ => unreachable!(),
        };
        let main = implicit.with_extension("").join("main.rs");
        if paths
            .iter()
            .any(|path| Path::new(path) == implicit || Path::new(path) == main)
            || (kind == Missing::Bin
                && package == Some(name)
                && paths
                    .iter()
                    .any(|path| Path::new(path) == directory.join("src/main.rs")))
        {
            return Some(true);
        }
    }
    Some(false)
}

/// Absent setup is deterministic unless a covered requirement asks to add it.
/// In that case the model sees the absence and the precise requirement together.
pub(super) fn diagnose(
    candidate: &mut Candidate,
    contract: &AcceptanceContract,
    requirements: &BTreeMap<String, String>,
    repository: &Path,
    project: &Path,
    base: &str,
) -> Option<(String, String)> {
    let entry = entries(contract).find(|entry| entry.id == candidate.id)?;
    let (command, cwd) = match &entry.check {
        archon_workflow::task_set_contract::AcceptanceCheck::Command { command, cwd } => {
            (command.as_str(), cwd)
        }
        _ => return None,
    };
    let output = format!(
        "{}\n{}",
        candidate.evidence.stderr, candidate.evidence.stdout
    );
    let (kind, name) = missing(&output)?;
    let base_exists = match cwd {
        archon_workflow::task_set_contract::TrustedCwd::RepoRoot => {
            exists(repository, base, kind, &name, command, Path::new(""))
        }
        archon_workflow::task_set_contract::TrustedCwd::ProjectRoot => {
            match project.strip_prefix(repository) {
                Ok(cwd) => exists(repository, base, kind, &name, command, cwd),
                Err(_) if kind == Missing::Path => Some(project.join(&name).exists()),
                Err(_) => exists(repository, base, kind, &name, command, Path::new("")),
            }
        }
    };
    if base_exists? {
        return None;
    }
    let requirement = entry
        .covers
        .iter()
        .filter_map(|id| requirements.get(id).map(|text| (id, text)))
        .find(|(_, text)| asks_to_add(text, &name));
    if let Some((id, text)) = requirement {
        candidate.shown["base_setup"] = serde_json::json!({"fact": format!("{name} is absent at base; requirement {id} says to add it"), "requirement": text});
        candidate.key = verdict_key(
            INSTRUCTION,
            serde_json::json!([candidate.key, candidate.shown["base_setup"]]),
        );
        return None;
    }
    // New test and binary targets need not be named in behavioral requirements.
    // Their absence is evidence for the judge, never a deterministic setup refutation.
    if matches!(kind, Missing::Test | Missing::Bin)
        || (kind == Missing::Path && command.contains("pytest"))
        || asks_to_add(&entry.criterion, &name)
    {
        candidate.shown["base_setup"] = serde_json::json!({"fact": format!("{name} is absent at base; judge whether the work must add it")});
        candidate.key = verdict_key(
            INSTRUCTION,
            serde_json::json!([candidate.key, candidate.shown["base_setup"]]),
        );
        return None;
    }
    let kind = match kind {
        Missing::Package => "package",
        Missing::Test => "test target",
        Missing::Bin => "binary target",
        Missing::Path => "path",
    };
    Some((
        evidence::inert(&format!(
            "the check cannot locate its own {kind} `{name}`, which is absent at base"
        )),
        evidence::inert(&format!(
            "no covered requirement asks to add `{name}`; name an existing project, manifest, package, target or test file"
        )),
    ))
}

#[path = "workflow_acceptance_requirement_addition.rs"]
mod addition;
use addition::asks_to_add;

/// Static package selectors are checked even when an earlier new test stops the script.
pub(in crate::command::workflow_task_set) fn package_findings(
    contract: &AcceptanceContract,
    repository: &Path,
    project: &Path,
    base: &str,
) -> BTreeMap<String, String> {
    let mut findings = BTreeMap::new();
    let Some(tree) = git(repository, &["ls-tree", "-r", "--name-only", base]) else {
        return findings;
    };
    let mut packages = Vec::new();
    let mut workspaces = BTreeMap::new();
    for path in tree.lines().filter(|path| path.ends_with("Cargo.toml")) {
        let Some(text) = git(repository, &["show", &format!("{base}:{path}")]) else {
            return findings;
        };
        let Ok(value) = toml::from_str::<toml::Value>(&text) else {
            return findings;
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
                || !args
                    .first()
                    .is_some_and(|arg| matches!(arg.as_str(), "test" | "build"))
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
            let Some(cwd) = package_cwd(&cwd, repository) else {
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
                    findings.insert(entry.id.clone(), format!("check '{}': cannot pass as written: `-p {name}` names directory {dir}; its package is named {real}", entry.id));
                }
            }
        }
    }
    findings
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

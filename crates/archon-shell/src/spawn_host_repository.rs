//! Remember repository selection in host memory, before handing a tree to an agent.
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

#[derive(Clone)]
struct Repository {
    git: PathBuf,
    work: PathBuf,
    common: PathBuf,
    config_sources: Vec<PathBuf>,
}
static REPOSITORIES: OnceLock<Mutex<BTreeMap<PathBuf, Repository>>> = OnceLock::new();
fn repositories() -> &'static Mutex<BTreeMap<PathBuf, Repository>> {
    REPOSITORIES.get_or_init(Default::default)
}

static FAILURE: Mutex<Option<String>> = Mutex::new(None);
pub(super) fn boundary_status() -> Result<(), String> {
    match FAILURE.lock().map_err(|e| e.to_string())?.as_ref() {
        Some(reason) => Err(reason.clone()),
        None => Ok(()),
    }
}
pub(super) fn register(root: &Path) -> Result<(), String> {
    let result = capture(root);
    if let Err(reason) = &result {
        if let Ok(mut failure) = FAILURE.lock() {
            *failure = Some(reason.clone());
        }
    }
    result
}
fn capture(root: &Path) -> Result<(), String> {
    let root = root.canonicalize().map_err(|e| e.to_string())?;
    let mut repositories = repositories().lock().map_err(|e| e.to_string())?;
    if repositories.contains_key(&root) {
        return Ok(());
    }
    let mut command = super::command("git");
    let output = command
        .current_dir(&root)
        .args([
            "rev-parse",
            "--path-format=absolute",
            "--absolute-git-dir",
            "--show-toplevel",
            "--git-common-dir",
        ])
        .output()
        .map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err(format!(
            "host repository capture failed for {}: {}",
            root.display(),
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    let text = String::from_utf8(output.stdout).map_err(|e| e.to_string())?;
    let mut lines = text.lines();
    let git: PathBuf = lines.next().ok_or("missing host Git directory")?.into();
    let work: PathBuf = lines.next().ok_or("missing host work tree")?.into();
    let common = lines
        .next()
        .ok_or("missing host common Git directory")?
        .into();
    let config_sources = config_sources(&work, &git)?;
    let repository = Repository {
        git,
        work,
        common,
        config_sources,
    };
    if repositories.len() >= 256 {
        return Err("repository registration limit reached for this run".into());
    }
    repositories.insert(repository.work.clone(), repository.clone());
    repositories.insert(root, repository);
    Ok(())
}

pub(super) fn command(root: &Path) -> std::process::Command {
    let captured = root
        .canonicalize()
        .ok()
        .and_then(|root| repositories().lock().ok()?.get(&root).cloned());
    let mut command = super::command("git");
    if let Some(repository) = captured {
        command
            .env("GIT_DIR", repository.git)
            .env("GIT_WORK_TREE", repository.work)
            .env("GIT_COMMON_DIR", repository.common);
    }
    command.current_dir(root);
    command
}

pub(super) fn protected_paths() -> Vec<PathBuf> {
    repositories()
        .lock()
        .map(|repositories| {
            repositories
                .values()
                .flat_map(|repo| {
                    #[cfg(not(target_os = "linux"))]
                    let mut paths = vec![
                        repo.git.join("config"),
                        repo.git.join("config.worktree"),
                        repo.git.join("commondir"),
                        repo.common.join("config"),
                    ];
                    #[cfg(target_os = "linux")]
                    let mut paths = vec![repo.common.join("config")];
                    #[cfg(target_os = "linux")]
                    if repo.git.join("config.worktree").exists() {
                        paths.push(repo.git.join("config.worktree"));
                    }
                    let pointer = repo.work.join(".git");
                    if !pointer.is_dir()
                        || pointer
                            .symlink_metadata()
                            .is_ok_and(|metadata| metadata.file_type().is_symlink())
                    {
                        paths.push(pointer);
                    }
                    paths.extend(repo.config_sources.clone());
                    paths
                })
                .collect()
        })
        .unwrap_or_default()
}

// Includes can live outside .git; seal the declared paths as well as every
// included file Git actually read. Otherwise an agent could plant filter or
// diff-driver commands without touching the repository's main config file.
fn config_sources(root: &Path, git_dir: &Path) -> Result<Vec<PathBuf>, String> {
    let extension = super::command("git")
        .current_dir(root)
        .args([
            "config",
            "--local",
            "--includes",
            "--bool",
            "--get",
            "extensions.worktreeConfig",
        ])
        .output()
        .map_err(|e| e.to_string())?;
    let worktree = match extension.status.code() {
        Some(0) => String::from_utf8_lossy(&extension.stdout).trim() == "true",
        Some(1) => false,
        _ => {
            return Err(format!(
                "host worktree-config extension lookup failed: {}",
                String::from_utf8_lossy(&extension.stderr)
            ));
        }
    };
    let mut sources = Vec::new();
    #[cfg(target_os = "linux")]
    if worktree {
        // Track absent sources too; preparation reports paths it cannot seal.
        sources.push(git_dir.join("config.worktree"));
    }
    for scope in ["--local", "--worktree"] {
        if scope == "--worktree" && (!worktree || !git_dir.join("config.worktree").exists()) {
            continue;
        }
        let output = super::command("git")
            .current_dir(root)
            .args([
                "config",
                scope,
                "--includes",
                "--show-origin",
                "--null",
                "--list",
            ])
            .output()
            .map_err(|e| e.to_string())?;
        if !output.status.success() {
            return Err(format!(
                "host repository configuration capture failed: {}",
                String::from_utf8_lossy(&output.stderr)
            ));
        }
        let records: Vec<_> = output.stdout.split(|b| *b == 0).collect();
        for pair in records.chunks_exact(2) {
            let origin = std::str::from_utf8(pair[0]).map_err(|e| e.to_string())?;
            let Some(file) = origin.strip_prefix("file:") else {
                continue;
            };
            let file = root.join(file);
            sources.push(file.clone());
            let entry = std::str::from_utf8(pair[1]).map_err(|e| e.to_string())?;
            if let Some((key, value)) = entry.split_once('\n') {
                if key == "include.path" || key.starts_with("includeif.") && key.ends_with(".path")
                {
                    let path = if let Some(relative) = value.strip_prefix("~/") {
                        crate::credential_paths::home().join(relative)
                    } else if value.starts_with("%(prefix)") || value.starts_with('~') {
                        file.parent().unwrap_or(root).join(value)
                    } else {
                        file.parent().unwrap_or(root).join(value)
                    };
                    sources.push(path);
                }
            }
        }
    }
    sources.sort();
    sources.dedup();
    Ok(sources)
}

pub(super) fn clear() {
    if let Ok(mut failure) = FAILURE.lock() {
        *failure = None;
    }
    if let Ok(mut repositories) = repositories().lock() {
        repositories.clear();
    }
}

#[cfg(any(target_os = "linux", target_os = "macos", all(test, unix)))]
pub(super) fn work_roots() -> Vec<PathBuf> {
    repositories()
        .lock()
        .map(|entries| entries.values().map(|entry| entry.work.clone()).collect())
        .unwrap_or_default()
}

use std::path::Path;

/// The directories (repo-relative; empty for the workspace root) of the
/// package the command names, or of every package when it names none.
pub(super) fn package_dirs_for(repo_root: &Path, command: &str) -> Vec<String> {
    let wanted = super::super::test_baseline_parse::cargo_package(command);
    let mut dirs = Vec::new();
    for dir in package_dirs(repo_root) {
        let manifest = repo_root.join(&dir).join("Cargo.toml");
        let Ok(text) = std::fs::read_to_string(&manifest) else {
            continue;
        };
        let name = manifest_package_name(&text);
        let matches = match (&wanted, name.as_deref()) {
            (Some(wanted), Some(name)) => wanted == name,
            (Some(wanted), None) => dir.rsplit('/').next() == Some(wanted.as_str()),
            (None, _) => true,
        };
        if matches {
            dirs.push(dir);
        }
    }
    dirs
}

/// The workspace root itself, then every directory one and two levels down
/// that holds a `Cargo.toml` — `crates/<name>/`, `<name>/` — sorted.
fn package_dirs(repo_root: &Path) -> Vec<String> {
    let mut dirs = vec![String::new()];
    let Ok(top) = std::fs::read_dir(repo_root) else {
        return dirs;
    };
    for entry in top.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if name.starts_with('.') || name == "target" {
            continue;
        }
        if path.join("Cargo.toml").is_file() {
            dirs.push(name.to_string());
        }
        let Ok(nested) = std::fs::read_dir(&path) else {
            continue;
        };
        for child in nested.flatten() {
            let child_path = child.path();
            if child_path.is_dir()
                && child_path.join("Cargo.toml").is_file()
                && let Some(child_name) = child_path.file_name().and_then(|n| n.to_str())
            {
                dirs.push(format!("{name}/{child_name}"));
            }
        }
    }
    dirs.sort();
    dirs.dedup();
    dirs
}

/// The `name = "..."` of a manifest's `[package]` table, read by line so a
/// workspace-only manifest (no package) answers `None`.
fn manifest_package_name(text: &str) -> Option<String> {
    let mut in_package = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_package = line == "[package]";
            continue;
        }
        if in_package && let Some(rest) = line.strip_prefix("name") {
            let rest = rest.trim_start();
            if let Some(value) = rest.strip_prefix('=') {
                return Some(value.trim().trim_matches('"').to_string());
            }
        }
    }
    None
}

/// The `src` directories (repo-relative) the command's package(s) hold.
pub(super) fn package_source_roots(repo_root: &Path, command: &str) -> Vec<String> {
    package_dirs_for(repo_root, command)
        .into_iter()
        .map(|dir| {
            if dir.is_empty() {
                "src".to_string()
            } else {
                format!("{dir}/src")
            }
        })
        .filter(|src| repo_root.join(src).is_dir())
        .collect()
}

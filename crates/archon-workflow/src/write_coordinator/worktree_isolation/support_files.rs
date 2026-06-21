use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use super::{IsolationError, run_git};

pub(super) fn capture_untracked(
    canonical_root: &Path,
    wanted: &BTreeSet<&str>,
    max_file_bytes: u64,
) -> Result<BTreeMap<String, Vec<u8>>, IsolationError> {
    let listing = run_git(
        &["ls-files", "--others", "--exclude-standard", "-z"],
        canonical_root,
    )?
    .stdout;
    let mut files = BTreeMap::new();
    for path in split_nul_paths(&listing) {
        let wanted_file = wanted.contains(path.as_str());
        if blocked_untracked(&path) {
            if wanted_file {
                return Err(IsolationError::UnsafeUntrackedFile { path });
            }
            continue;
        }
        if !wanted_file && !support_file(&path) {
            continue;
        }
        let abs = canonical_root.join(&path);
        let meta = std::fs::symlink_metadata(&abs)?;
        if !meta.file_type().is_file() {
            if wanted_file {
                return Err(IsolationError::UnsafeUntrackedFile { path });
            }
            continue;
        }
        let size = meta.len();
        if size > max_file_bytes {
            if wanted_file {
                return Err(IsolationError::FileTooLarge { path, size });
            }
            continue;
        }
        files.insert(path, std::fs::read(abs)?);
    }
    Ok(files)
}

pub(super) fn fingerprintable(path: &str) -> bool {
    !blocked_untracked(path)
}

fn split_nul_paths(bytes: &[u8]) -> impl Iterator<Item = String> + '_ {
    bytes
        .split(|byte| *byte == 0)
        .filter(|raw| !raw.is_empty())
        .map(|raw| String::from_utf8_lossy(raw).into_owned())
}

fn blocked_untracked(path: &str) -> bool {
    hidden_workflow_state(path)
        || generated_tree(path)
        || secret_like(path)
        || path_component(path, ".git")
}

fn hidden_workflow_state(path: &str) -> bool {
    path == ".archon" || path.starts_with(".archon/")
}

fn generated_tree(path: &str) -> bool {
    [
        "target",
        "node_modules",
        "dist",
        "build",
        ".next",
        ".turbo",
        "coverage",
        "__pycache__",
        ".pytest_cache",
        ".mypy_cache",
        ".gradle",
    ]
    .iter()
    .any(|component| path_component(path, component))
}

fn secret_like(path: &str) -> bool {
    let name = file_name(path).to_ascii_lowercase();
    name == ".env"
        || name.starts_with(".env.")
        || name.ends_with(".pem")
        || name.ends_with(".key")
        || name.ends_with(".p12")
        || name.ends_with(".pfx")
        || name == "id_rsa"
        || name == "id_dsa"
        || name == "id_ed25519"
        || name.contains("credential")
        || name.contains("secret")
}

fn support_file(path: &str) -> bool {
    source_extension(path) || support_name(path)
}

fn source_extension(path: &str) -> bool {
    let Some((_, ext)) = path.rsplit_once('.') else {
        return false;
    };
    matches!(
        ext,
        "c" | "cc"
            | "cpp"
            | "cs"
            | "css"
            | "go"
            | "gradle"
            | "h"
            | "hpp"
            | "html"
            | "java"
            | "js"
            | "jsx"
            | "json"
            | "kt"
            | "kts"
            | "mjs"
            | "py"
            | "pyi"
            | "rs"
            | "scss"
            | "sh"
            | "sql"
            | "swift"
            | "toml"
            | "ts"
            | "tsx"
            | "vue"
            | "xml"
            | "yaml"
            | "yml"
    )
}

fn support_name(path: &str) -> bool {
    matches!(
        file_name(path),
        "Cargo.lock"
            | "Cargo.toml"
            | "Dockerfile"
            | "Gemfile"
            | "Gemfile.lock"
            | "Justfile"
            | "Makefile"
            | "build.gradle"
            | "build.gradle.kts"
            | "gradle.properties"
            | "gradlew"
            | "package-lock.json"
            | "package.json"
            | "pnpm-lock.yaml"
            | "poetry.lock"
            | "pyproject.toml"
            | "requirements.txt"
            | "settings.gradle"
            | "settings.gradle.kts"
            | "setup.cfg"
            | "setup.py"
            | "tox.ini"
            | "tsconfig.json"
            | "yarn.lock"
    )
}

fn path_component(path: &str, needle: &str) -> bool {
    path.split('/').any(|part| part == needle)
}

fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

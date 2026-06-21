use std::path::Path;

use crate::write_coordinator::patch_manifest::{CapturedPatch, PatchError, capture_patch};
use crate::write_coordinator::write_plan::{NormalizedPath, WritePlan, normalize_target};
use crate::write_coordinator::{WriteCoordinatorConfig, coordinator::FanoutError};

use super::ItemState;

pub(super) fn capture_with_target_adoption(
    cfg: &WriteCoordinatorConfig,
    wave_items_len: usize,
    it: &ItemState<'_>,
) -> Result<(CapturedPatch, WritePlan), FanoutError> {
    let mut active_plan = it.plan.clone();
    let mut discovered = Vec::new();
    loop {
        match capture_patch(&it.workspace, &active_plan.target_files, &it.baseline) {
            Ok(captured) => return Ok((captured, active_plan)),
            Err(PatchError::UndeclaredWrite { path }) => {
                if discovered.len() >= cfg.max_dynamic_target_adoptions {
                    return Err(adoption_limit_error(
                        path,
                        discovered.len(),
                        cfg.max_dynamic_target_adoptions,
                    ));
                }
                if !adopt_undeclared_path(cfg, wave_items_len, &mut active_plan, &path) {
                    return Err(FanoutError::Patch(PatchError::UndeclaredWrite { path }));
                }
                discovered.push(path);
            }
            Err(err) => return Err(FanoutError::Patch(err)),
        }
    }
}

fn adoption_limit_error(path: String, adopted: usize, max: usize) -> FanoutError {
    FanoutError::Patch(PatchError::DynamicTargetAdoptionLimit { path, adopted, max })
}

fn adopt_undeclared_path(
    cfg: &WriteCoordinatorConfig,
    wave_items_len: usize,
    plan: &mut WritePlan,
    path: &str,
) -> bool {
    if wave_items_len != 1 || !safe_discovered_file(plan, path, cfg.max_file_bytes) {
        return false;
    }
    let Ok(normalized) = normalize_target(path, &plan.canonical_root) else {
        return false;
    };
    push_target_once(plan, normalized)
}

fn push_target_once(plan: &mut WritePlan, target: NormalizedPath) -> bool {
    if plan.target_files.contains(&target) {
        return true;
    }
    plan.target_files.push(target);
    true
}

fn safe_discovered_file(plan: &WritePlan, path: &str, max_file_bytes: u64) -> bool {
    safe_repo_path(path)
        && safe_file_kind(path)
        && safe_live_file(&plan.isolated_root.join(path), max_file_bytes)
}

fn safe_live_file(path: &Path, max_file_bytes: u64) -> bool {
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return false;
    };
    meta.file_type().is_file() && !meta.file_type().is_symlink() && meta.len() <= max_file_bytes
}

fn safe_repo_path(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with('/')
        && !path.contains('\0')
        && !path_component(path, ".archon")
        && !path_component(path, ".git")
        && !generated_tree(path)
        && !secret_like(path)
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

fn safe_file_kind(path: &str) -> bool {
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
            | "Makefile"
            | "build.gradle"
            | "build.gradle.kts"
            | "gradle.properties"
            | "gradlew"
            | "package-lock.json"
            | "package.json"
            | "pnpm-lock.yaml"
            | "pyproject.toml"
            | "requirements.txt"
            | "settings.gradle"
            | "settings.gradle.kts"
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adoption_limit_error_names_path_count_and_limit() {
        let err = adoption_limit_error("src/new.rs".to_string(), 2, 2).to_string();

        assert!(err.contains("dynamic target adoption limit reached"));
        assert!(err.contains("src/new.rs"));
        assert!(err.contains("2 adopted"));
        assert!(err.contains("max 2"));
    }
}

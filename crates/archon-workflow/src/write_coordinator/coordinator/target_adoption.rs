use std::collections::BTreeSet;
use std::path::Path;

use crate::write_coordinator::patch_manifest::{CapturedPatch, PatchError, capture_patch};
use crate::write_coordinator::write_plan::{NormalizedPath, WritePlan, normalize_target};
use crate::write_coordinator::{WriteCoordinatorConfig, coordinator::FanoutError};

use super::ItemState;

/// Capture the branch's patch, adopting undeclared files it legitimately
/// needed rather than throwing the whole patch away.
///
/// `claimed_by_others` is every path some *other* item in this wave declares.
/// Adoption used to be refused outright whenever the wave held more than one
/// item, which is far broader than the hazard it was guarding: two branches
/// adopting the same file and colliding on merge. A file no other item claims
/// cannot collide, so the wave's size is not the question — ownership is.
///
/// The blanket rule cost real work. A live branch needed exactly
/// one undeclared file, `src/command/trading_data_tests.rs`, in a fifteen-item
/// wave. Adoption was refused on item count alone and all thirty-one files of
/// correct, compiling work were discarded, with an adoption budget of sixty-four
/// left untouched.
pub(super) fn capture_with_target_adoption(
    cfg: &WriteCoordinatorConfig,
    claimed_by_others: &BTreeSet<NormalizedPath>,
    it: &ItemState<'_>,
) -> Result<(CapturedPatch, WritePlan), FanoutError> {
    let mut active_plan = it.plan.clone();
    let mut discovered = Vec::new();
    loop {
        let mut active_workspace = it.workspace.clone();
        active_workspace.plan = active_plan.clone();
        match capture_patch(&active_workspace, &active_plan.target_files, &it.baseline) {
            Ok(captured) => return Ok((captured, active_plan)),
            Err(PatchError::UndeclaredWrite { path }) => {
                if discovered.len() >= cfg.max_dynamic_target_adoptions {
                    return Err(adoption_limit_error(
                        path,
                        discovered.len(),
                        cfg.max_dynamic_target_adoptions,
                    ));
                }
                if !adopt_undeclared_path(cfg, claimed_by_others, &mut active_plan, &path) {
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
    claimed_by_others: &BTreeSet<NormalizedPath>,
    plan: &mut WritePlan,
    path: &str,
) -> bool {
    if !safe_discovered_file(plan, path, cfg.max_file_bytes) {
        return false;
    }
    let Ok(normalized) = normalize_target(path, &plan.canonical_root) else {
        return false;
    };
    // Another item in this wave owns it: adopting would put two branches on the
    // same file, which is the collision the old item-count rule was really for.
    if claimed_by_others.contains(&normalized) {
        return false;
    }
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
    // Only the fixture below needs this; importing it at module scope left a
    // dead import in every non-test build.
    use super::*;
    use crate::write_coordinator::write_plan::TargetFilesSource;

    fn plan_with(root: &Path, targets: &[&str]) -> WritePlan {
        let mut plan = WritePlan {
            run_id: "run".to_string(),
            stage_id: "stage".to_string(),
            item_id: "impl-item".to_string(),
            canonical_root: root.to_path_buf(),
            isolated_root: root.to_path_buf(),
            target_files: Vec::new(),
            target_dir_scopes: Vec::new(),
            target_files_source: TargetFilesSource::Item,
            read_context_files: Vec::new(),
            verify_inputs: Vec::new(),
            baseline_id: "baseline".to_string(),
            workspace_boundary_required: false,
            resource_keys: Default::default(),
        };
        for target in targets {
            let normalized = normalize_target(target, root).expect("normalize");
            plan.target_files.push(normalized);
        }
        plan
    }

    fn touch(root: &Path, rel: &str) {
        let path = root.join(rel);
        std::fs::create_dir_all(path.parent().expect("parent")).expect("dirs");
        std::fs::write(path, "// live file\n").expect("write");
    }

    /// The live failure: TASK-TDL-020 needed one undeclared file in a wave of
    /// fifteen. Adoption was refused on item count alone and all thirty-one
    /// files of correct work were discarded with 64 adoptions unused.
    #[test]
    fn a_file_no_other_item_owns_is_adopted_in_a_multi_item_wave() {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path();
        touch(root, "src/command/trading_data_tests.rs");
        let cfg = WriteCoordinatorConfig::default();
        let mut plan = plan_with(root, &["src/command/trading_data.rs"]);
        // Other items in the wave own unrelated files.
        let claimed = ["crates/other/src/lib.rs", "src/elsewhere.rs"]
            .iter()
            .map(|p| normalize_target(p, root).expect("normalize"))
            .collect::<BTreeSet<_>>();

        let adopted = adopt_undeclared_path(
            &cfg,
            &claimed,
            &mut plan,
            "src/command/trading_data_tests.rs",
        );

        assert!(
            adopted,
            "an unclaimed file must be adoptable regardless of wave size"
        );
        assert!(
            plan.target_files
                .contains(&normalize_target("src/command/trading_data_tests.rs", root).unwrap())
        );
    }

    /// The hazard the old item-count rule was really guarding: two branches
    /// adopting the same file and colliding when the wave merges.
    #[test]
    fn a_file_another_item_owns_is_never_adopted() {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path();
        touch(root, "src/shared.rs");
        let cfg = WriteCoordinatorConfig::default();
        let mut plan = plan_with(root, &["src/mine.rs"]);
        let claimed = std::iter::once(normalize_target("src/shared.rs", root).expect("normalize"))
            .collect::<BTreeSet<_>>();

        assert!(!adopt_undeclared_path(
            &cfg,
            &claimed,
            &mut plan,
            "src/shared.rs"
        ));
        assert_eq!(plan.target_files.len(), 1, "the plan must be unchanged");
    }

    /// Safety predicates still bind: adoption never reaches secrets, dotfiles,
    /// generated trees, or paths outside the repository.
    #[test]
    fn unsafe_paths_are_still_refused() {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path();
        for rel in [
            ".env",
            "target/generated.rs",
            ".git/config",
            "secret_token.rs",
        ] {
            touch(root, rel);
        }
        let cfg = WriteCoordinatorConfig::default();
        let claimed = BTreeSet::new();

        for rel in [
            ".env",
            "target/generated.rs",
            ".git/config",
            "secret_token.rs",
            "/etc/passwd",
            "does/not/exist.rs",
        ] {
            let mut plan = plan_with(root, &["src/mine.rs"]);
            assert!(
                !adopt_undeclared_path(&cfg, &claimed, &mut plan, rel),
                "must refuse {rel}"
            );
        }
    }

    #[test]
    fn adoption_limit_error_names_path_count_and_limit() {
        let err = adoption_limit_error("src/new.rs".to_string(), 2, 2).to_string();

        assert!(err.contains("dynamic target adoption limit reached"));
        assert!(err.contains("src/new.rs"));
        assert!(err.contains("2 adopted"));
        assert!(err.contains("max 2"));
    }
}

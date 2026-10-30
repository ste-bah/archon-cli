use std::path::{Path, PathBuf};

pub const SITE_REPOSITORY_DIRECTORY: &str = "repo";
pub const SITE_PROJECT_DIRECTORY: &str = "project";
pub const SITE_INTERNAL_DIRECTORIES: &[&str] = &[
    SITE_REPOSITORY_DIRECTORY,
    SITE_PROJECT_DIRECTORY,
    "home",
    "tmp",
    "cargo-home",
    "project-baseline",
    "process-groups",
    "target",
];
pub const SITE_HOME_DIRECTORY: &str = SITE_INTERNAL_DIRECTORIES[2];
pub const SITE_TEMP_DIRECTORY: &str = SITE_INTERNAL_DIRECTORIES[3];
pub const SITE_CARGO_HOME_DIRECTORY: &str = SITE_INTERNAL_DIRECTORIES[4];
pub const SITE_BASELINE_DIRECTORY: &str = SITE_INTERNAL_DIRECTORIES[5];
pub const SITE_PROCESS_GROUPS_DIRECTORY: &str = SITE_INTERNAL_DIRECTORIES[6];
pub const SITE_TARGET_DIRECTORY: &str = SITE_INTERNAL_DIRECTORIES[7];

/// Facts about a path that the real one-check site would not make available.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SitePathRefusal {
    pub site_target: PathBuf,
    pub reason: Option<String>,
}

/// Predict where a path resolves in the scratch site built for this one check.
/// This shares the site's root names, sibling discovery, aliases, and root
/// refusal rule, so callers can explain the same decision the site makes.
pub fn site_path_alone(
    project: &Path,
    repository: &Path,
    check: &str,
    cwd: crate::task_set_contract::TrustedCwd,
    named: &Path,
    build_cache: bool,
) -> Result<(), SitePathRefusal> {
    let project = archon_shell::paths::canonicalize(project).map_err(|error| SitePathRefusal {
        site_target: PathBuf::from("<site>"),
        reason: Some(format!("project root cannot be resolved: {error}")),
    })?;
    let repository =
        archon_shell::paths::canonicalize(repository).map_err(|error| SitePathRefusal {
            site_target: PathBuf::from("<site>"),
            reason: Some(format!("repository root cannot be resolved: {error}")),
        })?;
    let (root, site_root) = match cwd {
        crate::task_set_contract::TrustedCwd::ProjectRoot => {
            (&project, PathBuf::from(SITE_PROJECT_DIRECTORY))
        }
        crate::task_set_contract::TrustedCwd::RepoRoot => {
            (&repository, PathBuf::from(SITE_REPOSITORY_DIRECTORY))
        }
    };
    let site_cwd = PathBuf::from("<site>").join(&site_root);
    let site_target = lexical_site_path(&site_cwd.join(named));
    let discovered = crate::acceptance_sibling_paths::discover(root, &[check.to_string()]);
    if let Some(reason) = super::siblings::refused_sibling(&project, &repository, check, cwd) {
        let site_target = refused_named_path(check, &reason)
            .map(|path| lexical_site_path(&site_cwd.join(path)))
            .unwrap_or(site_target);
        return Err(SitePathRefusal {
            site_target,
            reason: Some(reason),
        });
    }
    let internal_target =
        !site_target.starts_with(&site_cwd) && is_site_internal_target(&site_target, build_cache);
    if internal_target
        && repository_copy_target(
            &site_target,
            root,
            &site_cwd,
            &discovered,
            &repository,
            named,
        )
    {
        return Ok(());
    }
    if internal_target {
        return Err(SitePathRefusal {
            reason: Some(format!(
                "the site uses `{}` for its own files and does not copy the sibling there",
                site_target
                    .components()
                    .nth(1)
                    .map(|component| component.as_os_str().to_string_lossy())
                    .unwrap_or_default()
            )),
            site_target,
        });
    }
    if site_target.starts_with(&site_cwd) {
        return if lexical_target_stays_inside(root, named) {
            Ok(())
        } else {
            Err(SitePathRefusal {
                site_target,
                reason: Some(format!("the path leaves the {}", root_name(cwd))),
            })
        };
    }
    for sibling in &discovered {
        let alias =
            lexical_site_path(&site_cwd.join(super::siblings::relative(root, &sibling.alias)));
        if site_target == alias || site_target.starts_with(&alias) {
            return Ok(());
        }
    }
    Err(SitePathRefusal {
        site_target,
        reason: None,
    })
}

fn repository_copy_target(
    site_target: &Path,
    root: &Path,
    site_cwd: &Path,
    discovered: &[crate::acceptance_sibling_paths::SiblingSource],
    repository: &Path,
    named: &Path,
) -> bool {
    let site_repository = PathBuf::from("<site>").join(SITE_REPOSITORY_DIRECTORY);
    let discovered_copy = discovered.iter().any(|sibling| {
        let alias =
            lexical_site_path(&site_cwd.join(super::siblings::relative(root, &sibling.alias)));
        sibling.source.starts_with(repository)
            && alias == site_repository.join(sibling.source.strip_prefix(repository).unwrap())
            && site_target.starts_with(&alias)
    });
    let live_target = lexical_site_path(&root.join(named));
    let repository_copy = live_target
        .strip_prefix(repository)
        .ok()
        .is_some_and(|relative| site_target.starts_with(&site_repository.join(relative)));
    discovered_copy || repository_copy
}

fn refused_named_path(check: &str, reason: &str) -> Option<PathBuf> {
    let alias = reason.split_once(": ")?.1.split_once(" resolves to ")?.0;
    let alias = Path::new(alias);
    crate::acceptance_sibling_paths::scan(check)
        .into_iter()
        .map(|named| named.path)
        .find(|path| path.starts_with(alias))
}

fn root_name(cwd: crate::task_set_contract::TrustedCwd) -> &'static str {
    match cwd {
        crate::task_set_contract::TrustedCwd::ProjectRoot => "project root",
        crate::task_set_contract::TrustedCwd::RepoRoot => "repository root",
    }
}

/// The site's own top-level directories, as the site makes them: with a
/// build cache the build target lives in the cache, not under the site.
pub fn site_internal_directories(build_cache: bool) -> Vec<&'static str> {
    SITE_INTERNAL_DIRECTORIES
        .iter()
        .copied()
        .filter(|name| is_site_internal_directory(name, build_cache))
        .collect()
}

pub(crate) fn is_site_internal_directory(name: &str, build_cache: bool) -> bool {
    SITE_INTERNAL_DIRECTORIES
        .iter()
        .filter(|directory| !(build_cache && **directory == SITE_TARGET_DIRECTORY))
        .any(|directory| *directory == name)
}

fn is_site_internal_target(target: &Path, build_cache: bool) -> bool {
    let Some(mut components) = target.strip_prefix("<site>").ok().map(Path::components) else {
        return false;
    };
    let Some(std::path::Component::Normal(name)) = components.next() else {
        return false;
    };
    is_site_internal_directory(&name.to_string_lossy(), build_cache)
}

fn lexical_target_stays_inside(root: &Path, named: &Path) -> bool {
    let mut current = root.to_path_buf();
    let root = lexical_site_path(root);
    for component in named.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                if current == root {
                    return false;
                }
                current.pop();
            }
            std::path::Component::Normal(part) => current.push(part),
            std::path::Component::RootDir | std::path::Component::Prefix(_) => return false,
        }
        if !lexical_site_path(&current).starts_with(&root) {
            return false;
        }
    }
    true
}

pub(crate) fn site_alias_leaves_live_root(
    root: &Path,
    cwd: crate::task_set_contract::TrustedCwd,
    named: &Path,
) -> bool {
    let site_root = match cwd {
        crate::task_set_contract::TrustedCwd::ProjectRoot => SITE_PROJECT_DIRECTORY,
        crate::task_set_contract::TrustedCwd::RepoRoot => SITE_REPOSITORY_DIRECTORY,
    };
    let site_cwd = PathBuf::from("<site>").join(site_root);
    lexical_site_path(&site_cwd.join(named)).starts_with(&site_cwd)
        && !lexical_target_stays_inside(root, named)
}

fn lexical_site_path(path: &Path) -> PathBuf {
    let mut result = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                result.pop();
            }
            other => result.push(other.as_os_str()),
        }
    }
    result
}

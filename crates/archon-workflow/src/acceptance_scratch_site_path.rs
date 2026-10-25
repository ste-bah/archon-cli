use std::path::{Path, PathBuf};

pub const SITE_REPOSITORY_DIRECTORY: &str = "repo";
pub const SITE_PROJECT_DIRECTORY: &str = "project";

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
    if !site_target.starts_with(&site_cwd)
        && is_site_internal_target(&site_target)
        && !repository_copy_target(&site_target, root, &site_cwd, &discovered, &repository)
    {
        return Err(SitePathRefusal {
            site_target,
            reason: None,
        });
    }
    if site_target.starts_with(&site_cwd) {
        let live_target = root.join(named);
        return match archon_shell::paths::canonicalize(&live_target) {
            Ok(target) if target.starts_with(root) => Ok(()),
            Ok(_) => Err(SitePathRefusal {
                site_target,
                reason: None,
            }),
            Err(_) => Ok(()),
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
) -> bool {
    let site_repository = PathBuf::from("<site>").join(SITE_REPOSITORY_DIRECTORY);
    site_target.starts_with(&site_repository)
        && discovered.iter().any(|sibling| {
            let alias =
                lexical_site_path(&site_cwd.join(super::siblings::relative(root, &sibling.alias)));
            (site_target == alias || site_target.starts_with(&alias))
                && sibling.source.starts_with(repository)
        })
}

fn refused_named_path(check: &str, reason: &str) -> Option<PathBuf> {
    let alias = reason.split_once(": ")?.1.split_once(" resolves to ")?.0;
    let alias = Path::new(alias);
    crate::acceptance_sibling_paths::scan(check)
        .into_iter()
        .map(|named| named.path)
        .find(|path| path.starts_with(alias))
}

fn is_site_internal_target(target: &Path) -> bool {
    let Some(mut components) = target.strip_prefix("<site>").ok().map(Path::components) else {
        return false;
    };
    let Some(std::path::Component::Normal(name)) = components.next() else {
        return false;
    };
    [
        SITE_REPOSITORY_DIRECTORY,
        SITE_PROJECT_DIRECTORY,
        "home",
        "tmp",
        "cargo-home",
        "target",
        "project-baseline",
    ]
    .contains(&name.to_string_lossy().as_ref())
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

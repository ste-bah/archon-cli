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
    let project = archon_shell::paths::canonicalize(project).unwrap_or_else(|_| project.into());
    let repository =
        archon_shell::paths::canonicalize(repository).unwrap_or_else(|_| repository.into());
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
        return Err(SitePathRefusal {
            site_target,
            reason: Some(reason),
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
            let relative_target = site_target
                .strip_prefix(&alias)
                .unwrap_or_else(|_| Path::new(""));
            if sibling_contains(sibling.source.as_path(), relative_target) {
                return Ok(());
            }
        }
    }
    Err(SitePathRefusal {
        site_target,
        reason: None,
    })
}

fn sibling_contains(source: &Path, relative_target: &Path) -> bool {
    let target = source.join(relative_target);
    if !target.exists() {
        return false;
    }
    if !source.join(".git").exists() || relative_target.as_os_str().is_empty() {
        return true;
    }
    let object = format!("HEAD:{}", relative_target.to_string_lossy());
    archon_shell::spawn::command("git")
        .arg("-C")
        .arg(source)
        .args(["cat-file", "-e"])
        .arg(object)
        .output()
        .is_ok_and(|output| output.status.success())
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

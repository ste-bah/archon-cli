use super::*;

pub(super) fn is_project_or_ancestor(source: &Path, project: &Path) -> bool {
    project.starts_with(source)
}

pub(super) fn site_destination_first_component(
    copied_root: &Path,
    relative: &Path,
) -> Option<String> {
    let mut components = vec![copied_root.file_name()?.to_string_lossy().into_owned()];
    for component in relative.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                components.pop();
            }
            std::path::Component::Normal(name) => {
                components.push(name.to_string_lossy().into_owned());
            }
            std::path::Component::RootDir | std::path::Component::Prefix(_) => return None,
        }
    }
    components.into_iter().next()
}

pub(super) fn git_head(path: &Path) -> WorkflowResult<String> {
    let output = archon_shell::spawn::git_command(path)
        .args(["rev-parse", "HEAD"])
        .output()
        .map_err(|error| WorkflowError::io(path, error))?;
    if !output.status.success() {
        return Err(super::invalid(format!(
            "git rev-parse HEAD in {}: {}",
            path.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

pub(crate) fn relative(from: &Path, to: &Path) -> PathBuf {
    let left = from.components().collect::<Vec<_>>();
    let right = to.components().collect::<Vec<_>>();
    let common = left.iter().zip(&right).take_while(|(a, b)| a == b).count();
    let mut path = PathBuf::new();
    for _ in common..left.len() {
        path.push("..");
    }
    for component in &right[common..] {
        path.push(component.as_os_str());
    }
    path
}

pub(super) fn setup_failure(
    root: &Path,
    cwd: crate::task_set_contract::TrustedCwd,
    check: &str,
) -> Option<(String, Option<super::faults::FaultCause>)> {
    for named in crate::acceptance_sibling_paths::scan(check) {
        if super::site_path::site_alias_leaves_live_root(root, cwd, &named.path) {
            let reason = match cwd {
                crate::task_set_contract::TrustedCwd::ProjectRoot => "project root",
                crate::task_set_contract::TrustedCwd::RepoRoot => "repository root",
            };
            return Some((
                format!(
                    "the path leaves the {reason}: {}",
                    archon_shell::paths::stored_text(&named.path)
                ),
                Some(super::faults::FaultCause::SiteAliasRefused),
            ));
        }
    }
    if let Some(reason) = crate::acceptance_sibling_paths::missing_root(root, check) {
        return Some((
            format!("the check {reason}"),
            Some(super::faults::FaultCause::SiblingShapeRefused),
        ));
    }
    None
}

#[cfg(test)]
#[path = "acceptance_scratch_sibling_setup_tests.rs"]
mod setup_tests;

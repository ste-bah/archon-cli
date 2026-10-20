use super::verdict::Context;
use super::{AcceptanceContract, CheckResult, executed_text};

/// Special identity for a selected missing-setup-path failure. Other
/// no-verdict failures keep the legacy exact-text identity.
pub(super) fn setup_path_signature(
    contract: &AcceptanceContract,
    id: &str,
    result: &CheckResult,
    selected_why: &str,
    at: &Context,
) -> Option<String> {
    let entry = (contract.acceptance.iter())
        .chain(&contract.supplementary)
        .find(|entry| entry.id == id);
    let (_, text) = executed_text(entry?)?;
    let why = super::verdict::setup_path::failure(text, result, at)?;
    if why != selected_why {
        return None;
    }
    let path = why.split_once("it names the path `")?.1.split_once('`')?.0;
    let cwd = match &entry?.check {
        archon_workflow::task_set_contract::AcceptanceCheck::Command { cwd, .. } => match cwd {
            archon_workflow::task_set_contract::TrustedCwd::ProjectRoot => "project_root",
            archon_workflow::task_set_contract::TrustedCwd::RepoRoot => "repo_root",
        },
        _ => "selected_root",
    };
    Some(format!("setup-path\0{cwd}\0{}", normalize_path(path)))
}

pub(super) fn normalize_path(path: &str) -> String {
    use std::path::Component;
    let mut parts: Vec<String> = Vec::new();
    let absolute = std::path::Path::new(path).is_absolute();
    for component in std::path::Path::new(path).components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if parts.last().is_some_and(|part| part != "..") {
                    parts.pop();
                } else if !std::path::Path::new(path).is_absolute() {
                    parts.push("..".into());
                }
            }
            Component::Normal(part) => parts.push(part.to_string_lossy().into_owned()),
            Component::RootDir | Component::Prefix(_) => {}
        }
    }
    let normalized = parts.join("/");
    if absolute {
        format!("/{normalized}")
    } else {
        normalized
    }
}

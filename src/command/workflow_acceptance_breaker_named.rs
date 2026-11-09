//! Literal file operands only; directory collection leaves the view intact.
use super::*;
use archon_workflow::base_snapshot::snapshot_path_key;
use archon_workflow::task_set_contract::TrustedCwd;
pub(super) fn check_named_keys(
    entry: &AcceptanceCriterion,
    repository: &Path,
    project: &Path,
    view: &BaseView,
) -> Vec<String> {
    from_keys(entry, repository, project, &view.file_keys)
}
pub(super) fn from_keys(
    entry: &AcceptanceCriterion,
    repository: &Path,
    project: &Path,
    files: &BTreeSet<String>,
) -> Vec<String> {
    let AcceptanceCheck::Command { command, cwd } = &entry.check else {
        return vec![];
    };
    let cwd = if *cwd == TrustedCwd::RepoRoot {
        repository
    } else {
        project
    };
    let mut roots = vec![cwd.to_path_buf(), repository.into(), project.into()];
    let mut words = Vec::new();
    for c in verdict_shell::simple_commands(command) {
        for word in std::iter::once(c.word)
            .chain(c.args)
            .filter(|w| !verdict_shell::expands(w))
        {
            let value = word
                .split_once('=')
                .filter(|(_, v)| word.starts_with('-') && !v.is_empty())
                .map_or(word.as_str(), |(_, v)| v);
            let operand = PathBuf::from(value.replace('\\', "/"));
            let paths: Vec<_> = roots.iter().map(|r| lexical(r.join(&operand))).collect();
            for p in paths {
                let key = snapshot_path_key(repository, &p.to_string_lossy());
                if files.contains(&key) {
                    words.push(key);
                } else if files.iter().any(|k| k.starts_with(&format!("{key}/"))) {
                    roots.push(p);
                }
            }
        }
    }
    words.sort();
    words.dedup();
    words
}
pub(super) fn exclude(view: &mut BaseView, keys: &[String]) -> Result<(), String> {
    for key in keys {
        if view.files.remove(key).is_some() {
            std::fs::remove_file(
                view.root()
                    .join(archon_workflow::base_snapshot::key_path(key)),
            )
            .map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

fn lexical(path: PathBuf) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    archon_shell::paths::plain(out)
}

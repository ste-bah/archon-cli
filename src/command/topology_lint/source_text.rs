use super::{LintSource, Path, PathBuf};

pub(super) fn describe(source: &LintSource) -> String {
    match source {
        LintSource::TaskFile(path) => {
            format!("task file {}", archon_shell::paths::stored_text(path))
        }
        LintSource::Tasks(path) => {
            format!("task directory {}", archon_shell::paths::stored_text(path))
        }
        LintSource::Spec(path) => {
            format!("workflow spec {}", archon_shell::paths::stored_text(path))
        }
        LintSource::Graph(id) => format!("recorded graph {id}"),
    }
}
pub(super) fn absolute(cwd: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    }
}

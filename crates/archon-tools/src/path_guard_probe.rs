//! A host-side answer to "may an agent holding this context read that
//! path", given by the guard the agent's own tools would consult.
//!
//! `Read`, `Glob` and `Grep` all resolve their path through
//! `path_guard::resolve_existing_path`; this is that function, exported so a
//! host can ask the question before it dispatches an agent rather than
//! learning the answer hours later from the agent's tool errors. Nothing is
//! widened: the probe reads nothing and grants nothing, it only reports what
//! the guard would decide for `ctx` as it stands.

use std::path::{Path, PathBuf};

use crate::tool::ToolContext;

/// The canonical path an agent holding `ctx` would be handed for `path`, or
/// the exact refusal text its `Read` of `path` would have returned.
pub fn probe_read_access(path: &Path, ctx: &ToolContext) -> Result<PathBuf, String> {
    crate::path_guard::resolve_existing_path(&path.display().to_string(), ctx)
}

#[cfg(test)]
mod path_guard_probe_tests {
    use super::*;

    fn context(working_dir: &Path, extra_dirs: Vec<PathBuf>) -> ToolContext {
        ToolContext {
            working_dir: working_dir.to_path_buf(),
            extra_dirs,
            ..ToolContext::default()
        }
    }

    #[test]
    fn path_guard_probe_refuses_a_root_missing_from_the_allowed_roots() {
        let temp = tempfile::tempdir().unwrap();
        let project = temp.path().join("project");
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::create_dir_all(&repo).unwrap();

        let refusal = probe_read_access(&repo, &context(&project, Vec::new()))
            .expect_err("the repository is not under the project");
        assert!(refusal.contains("outside allowed directories"), "{refusal}");
        assert!(refusal.contains(&project.canonicalize().unwrap().display().to_string()), "{refusal}");
    }

    #[test]
    fn path_guard_probe_admits_a_root_listed_as_an_extra_dir() {
        let temp = tempfile::tempdir().unwrap();
        let project = temp.path().join("project");
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::create_dir_all(repo.join("src")).unwrap();

        let ctx = context(&project, vec![repo.clone()]);
        assert_eq!(probe_read_access(&repo, &ctx).unwrap(), repo.canonicalize().unwrap());
        assert!(probe_read_access(&repo.join("src"), &ctx).is_ok());
        assert!(probe_read_access(&project, &ctx).is_ok(), "the working dir stays readable");
    }
}

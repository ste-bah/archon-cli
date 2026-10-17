//! Copy relative sibling inputs named by authorized checks. A Git sibling is
//! cloned at HEAD, so uncommitted changes are not seen; the copied HEAD is
//! verified and recorded. A non-Git sibling is copied from its current files,
//! excluding build-output directories below its root, and its evidence has no
//! commit.
use super::*;

const OMIT: &[&str] = &[".git", "target", "node_modules"];

impl super::ScratchRoots {
    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn project(&self) -> &Path {
        &self.project
    }

    pub fn repository(&self) -> &Path {
        &self.repository
    }

    pub fn target(&self) -> PathBuf {
        self.target.clone()
    }

    pub(crate) fn sibling_revisions(&self) -> &[crate::acceptance_sibling_paths::SiblingRevision] {
        &self.sibling_revisions
    }
}

pub(super) fn copy_external(
    project: &Path,
    repository: &Path,
    copied_project: &Path,
    copied_repository: &Path,
    checks: &[(String, crate::task_set_contract::TrustedCwd)],
    remaining: &mut u64,
) -> WorkflowResult<Vec<crate::acceptance_sibling_paths::SiblingRevision>> {
    let project = archon_shell::paths::canonicalize(project)
        .map_err(|error| WorkflowError::io(project, error))?;
    let repository = archon_shell::paths::canonicalize(repository)
        .map_err(|error| WorkflowError::io(repository, error))?;
    let mut siblings = Vec::new();
    for (check, cwd) in checks {
        let (root, copied_root) = match cwd {
            crate::task_set_contract::TrustedCwd::ProjectRoot => (&project, copied_project),
            crate::task_set_contract::TrustedCwd::RepoRoot => (&repository, copied_repository),
        };
        for sibling in crate::acceptance_sibling_paths::discover(root, std::slice::from_ref(check))
        {
            let path = relative(root, &sibling.alias);
            siblings.push((copied_root.join(&path), path, sibling.source));
        }
    }
    siblings.sort();
    siblings.dedup_by(|left, right| left.0 == right.0);
    let mut evidence = Vec::new();
    for (destination, relative, source) in siblings {
        let existed = destination.exists();
        let commit = if source.join(".git").exists() {
            let head = git_head(&source)?;
            if existed {
                let copied_head = git_head(&destination)?;
                let current_head = git_head(&source)?;
                if copied_head != head || current_head != head {
                    return Err(invalid(format!(
                        "sibling revision mismatch at {} (expected {head}, copied {copied_head}, current {current_head})",
                        relative.display()
                    )));
                }
                Some(head)
            } else {
                if let Some(parent) = destination.parent() {
                    std::fs::create_dir_all(parent)
                        .map_err(|error| WorkflowError::io(parent, error))?;
                }
                let output = archon_shell::spawn::command("git")
                    .args([
                        "-c",
                        "core.hooksPath=/dev/null",
                        "clone",
                        "--quiet",
                        "--shared",
                        "--",
                    ])
                    .arg(
                        archon_shell::paths::try_plain(source.clone())
                            .map_err(|e| WorkflowError::io(&source, e))?,
                    )
                    .arg(
                        archon_shell::paths::try_plain(destination.clone())
                            .map_err(|e| WorkflowError::io(&destination, e))?,
                    )
                    .output()
                    .map_err(|error| WorkflowError::io(&source, error))?;
                if !output.status.success() {
                    return Err(invalid(format!(
                        "git clone {}: {}",
                        source.display(),
                        String::from_utf8_lossy(&output.stderr).trim()
                    )));
                }
                let copied_head = git_head(&destination)?;
                let current_head = git_head(&source)?;
                if copied_head != head || current_head != head {
                    return Err(invalid(format!(
                        "sibling revision changed while copying {} (expected {head}, copied {copied_head}, current {current_head})",
                        relative.display()
                    )));
                }
                Some(head)
            }
        } else if existed {
            None
        } else {
            if let Some(parent) = destination.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|error| WorkflowError::io(parent, error))?;
            }
            if source.is_file() {
                if let Some(parent) = destination.parent() {
                    std::fs::create_dir_all(parent)
                        .map_err(|error| WorkflowError::io(parent, error))?;
                }
                super::io::copy_tree(&source, &destination, remaining, true)?;
            } else {
                copy_directory(&source, &source, &destination, remaining)?;
            }
            None
        };
        evidence.push(crate::acceptance_sibling_paths::SiblingRevision {
            path: relative,
            commit,
        });
        if !existed && source.join(".git").exists() {
            *remaining = remaining
                .checked_sub(tree_bytes(&destination)?)
                .ok_or_else(|| invalid("scratch copy exceeds configured size limit"))?;
        }
    }
    Ok(evidence)
}

fn git_head(path: &Path) -> WorkflowResult<String> {
    let output = archon_shell::spawn::command("git")
        .args(["-C"])
        .arg(
            archon_shell::paths::try_plain(path.to_path_buf())
                .map_err(|e| WorkflowError::io(path, e))?,
        )
        .args(["rev-parse", "HEAD"])
        .output()
        .map_err(|error| WorkflowError::io(path, error))?;
    if !output.status.success() {
        return Err(invalid(format!(
            "git rev-parse HEAD in {}: {}",
            path.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn relative(from: &Path, to: &Path) -> PathBuf {
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

fn copy_directory(
    root: &Path,
    source: &Path,
    dest: &Path,
    remaining: &mut u64,
) -> WorkflowResult<()> {
    let name = source.file_name().unwrap_or_default();
    // A named sibling root is needed whole; skip build-output folders inside it.
    if source != root && OMIT.iter().any(|skip| name == *skip) {
        return Ok(());
    }
    let metadata =
        std::fs::symlink_metadata(source).map_err(|error| WorkflowError::io(source, error))?;
    let target = dest.join(source.strip_prefix(root).unwrap_or(Path::new("")));
    if metadata.is_dir() {
        std::fs::create_dir_all(&target).map_err(|error| WorkflowError::io(&target, error))?;
        for entry in std::fs::read_dir(source).map_err(|error| WorkflowError::io(source, error))? {
            let entry = entry.map_err(|error| WorkflowError::io(source, error))?;
            copy_directory(root, &entry.path(), dest, remaining)?;
        }
    } else if metadata.is_file() {
        super::io::copy_tree(source, &target, remaining, true)?;
    }
    Ok(())
}

fn tree_bytes(root: &Path) -> WorkflowResult<u64> {
    let mut total = 0u64;
    let mut stack = vec![root.to_path_buf()];
    while let Some(path) = stack.pop() {
        for entry in std::fs::read_dir(&path).map_err(|error| WorkflowError::io(&path, error))? {
            let entry = entry.map_err(|error| WorkflowError::io(&path, error))?;
            let child = entry.path();
            let meta = entry
                .metadata()
                .map_err(|error| WorkflowError::io(&child, error))?;
            if meta.is_dir() {
                stack.push(child);
            } else if meta.is_file() {
                total = total.saturating_add(meta.len());
            }
        }
    }
    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run_git(path: &Path, args: &[&str]) {
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(path)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn configured_copy_keeps_nested_tracked_sibling_and_omits_build_output() {
        // EXPECTED: `../a/b` runs tracked status 23 from the configured copy;
        // its untracked target tree is absent.
        let root = tempfile::tempdir().unwrap();
        let project = root.path().join("project");
        let copied = root.path().join("scratch/project");
        let sibling = root.path().join("a/b");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::create_dir_all(&sibling).unwrap();
        std::fs::create_dir_all(&copied).unwrap();
        std::fs::write(
            sibling.join("check.sh"),
            "#!/bin/sh\n# current committed revision\nexit 23\n",
        )
        .unwrap();
        run_git(&sibling, &["init", "-q"]);
        run_git(&sibling, &["config", "user.email", "test@example.invalid"]);
        run_git(&sibling, &["config", "user.name", "test"]);
        run_git(&sibling, &["add", "check.sh"]);
        run_git(&sibling, &["commit", "-q", "-m", "tracked"]);
        std::fs::write(sibling.join("check.sh"), "#!/bin/sh\nexit 23\n").unwrap();
        run_git(&sibling, &["add", "check.sh"]);
        run_git(&sibling, &["commit", "-q", "-m", "current head"]);
        let expected_head = git_head(&sibling).unwrap();
        std::fs::create_dir_all(sibling.join("target/debug")).unwrap();
        std::fs::write(sibling.join("target/debug/output"), b"untracked").unwrap();
        let mut remaining = u64::MAX;
        let evidence = copy_external(
            &project,
            &project,
            &copied,
            &copied,
            &[(
                "cd ../a/b && sh check.sh".into(),
                crate::task_set_contract::TrustedCwd::ProjectRoot,
            )],
            &mut remaining,
        )
        .unwrap();
        #[cfg(unix)]
        let command = std::process::Command::new("/bin/sh")
            .args(["-c", "cd ../a/b && sh check.sh"])
            .current_dir(&copied)
            .output()
            .unwrap();
        #[cfg(unix)]
        {
            println!(
                "EVIDENCE #420 scratch-sibling BEFORE: sibling absent; AFTER: tracked sibling status={:?}; target copied={}",
                command.status.code(),
                copied.parent().unwrap().join("a/b/target").exists()
            );
            assert_eq!(command.status.code(), Some(23));
        }
        assert!(!copied.parent().unwrap().join("a/b/target").exists());
        let copied_head = git_head(&copied.parent().unwrap().join("a/b")).unwrap();
        println!(
            "EVIDENCE #420 scratch-sibling-revision BEFORE: old revision could be copied; AFTER: path=../a/b, source={expected_head}, copied={copied_head}"
        );
        assert_eq!(copied_head, expected_head);
        assert_eq!(evidence[0].path, PathBuf::from("../a/b"));
        assert_eq!(evidence[0].commit.as_deref(), Some(expected_head.as_str()));
    }

    #[test]
    fn a_check_without_an_outside_path_keeps_the_scratch_parent_unchanged() {
        let root = tempfile::tempdir().unwrap();
        let project = root.path().join("project");
        let copied = root.path().join("scratch/project");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::create_dir_all(&copied).unwrap();
        let before = std::fs::read_dir(copied.parent().unwrap()).unwrap().count();
        let mut remaining = u64::MAX;
        copy_external(
            &project,
            &project,
            &copied,
            &copied,
            &[(
                "cargo test".into(),
                crate::task_set_contract::TrustedCwd::ProjectRoot,
            )],
            &mut remaining,
        )
        .unwrap();
        let after = std::fs::read_dir(copied.parent().unwrap()).unwrap().count();
        assert_eq!(before, after);
    }

    #[test]
    fn sibling_discovery_parses_quotes_and_ignores_prose() {
        // EXPECTED before running: the scanner returns the quoted manifest
        // sibling and does not treat an echo argument as a required path.
        assert_eq!(
            crate::acceptance_sibling_paths::scan(
                "cargo test --manifest-path=\"../repo/Cargo.toml\""
            )
            .into_iter()
            .map(|s| s.path)
            .collect::<Vec<_>>(),
            vec![PathBuf::from("../repo/Cargo.toml")]
        );
        assert!(!crate::acceptance_sibling_paths::scan("echo \"../missing\"")[0].required);
    }

    #[test]
    fn copies_a_standalone_sibling_file_and_omits_nested_build_output() {
        // EXPECTED before implementation: the file is copied in-place while
        // nested target and node_modules trees are omitted.
        let root = tempfile::tempdir().unwrap();
        let project = root.path().join("project");
        let copied = root.path().join("scratch/project");
        let sibling = root.path().join("sibling");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::create_dir_all(&copied).unwrap();
        std::fs::write(root.path().join("note.txt"), "standalone file").unwrap();
        std::fs::create_dir_all(sibling.join("target/debug")).unwrap();
        std::fs::create_dir_all(sibling.join("node_modules/pkg")).unwrap();
        std::fs::write(sibling.join("note.txt"), "real file").unwrap();
        std::fs::write(sibling.join("target/debug/output"), "build").unwrap();
        std::fs::write(sibling.join("node_modules/pkg/output"), "dependency").unwrap();
        let mut remaining = u64::MAX;
        let evidence = copy_external(
            &project,
            &project,
            &copied,
            &copied,
            &[(
                "echo ../note.txt ../sibling".into(),
                crate::task_set_contract::TrustedCwd::ProjectRoot,
            )],
            &mut remaining,
        )
        .unwrap();
        let copied_sibling = copied.parent().unwrap().join("sibling");
        assert_eq!(
            std::fs::read_to_string(copied.parent().unwrap().join("note.txt")).unwrap(),
            "standalone file"
        );
        assert_eq!(
            std::fs::read_to_string(copied_sibling.join("note.txt")).unwrap(),
            "real file"
        );
        assert!(!copied_sibling.join("target").exists());
        assert!(!copied_sibling.join("node_modules").exists());
        assert!(
            evidence
                .iter()
                .any(|revision| revision.path == PathBuf::from("../note.txt"))
        );
        assert!(
            evidence
                .iter()
                .any(|revision| revision.path == PathBuf::from("../sibling"))
        );
        assert_eq!(
            evidence
                .iter()
                .find(|revision| revision.path == PathBuf::from("../sibling"))
                .unwrap()
                .commit,
            None
        );
    }

    #[test]
    fn a_sibling_root_named_target_is_copied() {
        // EXPECTED: the check names ../target itself, so the root is copied;
        // only target directories below that root are omitted.
        let root = tempfile::tempdir().unwrap();
        let project = root.path().join("project");
        let copied = root.path().join("scratch/project");
        let sibling = root.path().join("target");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::create_dir_all(&copied).unwrap();
        std::fs::create_dir_all(sibling.join("target/debug")).unwrap();
        std::fs::write(sibling.join("needed.txt"), "current target root").unwrap();
        std::fs::write(sibling.join("target/debug/output"), "nested build").unwrap();
        let mut remaining = u64::MAX;
        copy_external(
            &project,
            &project,
            &copied,
            &copied,
            &[(
                "cat ../target/needed.txt".into(),
                crate::task_set_contract::TrustedCwd::ProjectRoot,
            )],
            &mut remaining,
        )
        .unwrap();
        let copied_target = copied.parent().unwrap().join("target");
        assert_eq!(
            std::fs::read_to_string(copied_target.join("needed.txt")).unwrap(),
            "current target root"
        );
        assert!(!copied_target.join("target").exists());
    }
}

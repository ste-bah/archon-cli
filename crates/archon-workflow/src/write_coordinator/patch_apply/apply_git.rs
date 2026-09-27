use std::path::Path;

use crate::write_coordinator::worktree_isolation::{IsolationError, run_git, run_git_with_stdin};

pub(super) fn apply_patch(
    canonical_root: &Path,
    patch_path: &str,
    changed_files: &[String],
) -> Result<(), IsolationError> {
    // Rust can read Windows long evidence paths that Git's file API rejects.
    // Keep the durable record where it belongs and send the same bytes on retries.
    let patch = std::fs::read(patch_path)?;
    match run_git_with_stdin(
        &["apply", "--whitespace=nowarn", "-"],
        canonical_root,
        &patch,
    ) {
        Ok(output) => Ok(output).map(|_| ()),
        Err(first) if has_staged_targets(canonical_root, changed_files) => Err(first),
        Err(first) => run_git_with_stdin(
            &["apply", "--3way", "--whitespace=nowarn", "-"],
            canonical_root,
            &patch,
        )
        .map(|_| ())
        .map_err(|second| prefer_apply_error(first, second)),
    }
}

fn has_staged_targets(canonical_root: &Path, changed_files: &[String]) -> bool {
    if changed_files.is_empty() {
        return false;
    }
    let mut args: Vec<&str> = vec!["diff", "--cached", "--name-only", "--"];
    args.extend(changed_files.iter().map(String::as_str));
    run_git(&args, canonical_root)
        .map(|out| !out.stdout.is_empty())
        .unwrap_or(true)
}

fn prefer_apply_error(first: IsolationError, second: IsolationError) -> IsolationError {
    let second_text = second.to_string();
    if second_text.contains("does not match index") {
        first
    } else {
        second
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_patch_at_a_long_evidence_path_applies_without_renaming_the_record() {
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("repo");
        std::fs::create_dir(&repo).unwrap();
        run_git(&["init", "-q"], &repo).unwrap();
        std::fs::write(repo.join("owned.txt"), "before\n").unwrap();
        run_git(&["add", "owned.txt"], &repo).unwrap();
        run_git(
            &[
                "-c",
                "user.name=test",
                "-c",
                "user.email=test@example.invalid",
                "commit",
                "-qm",
                "baseline",
            ],
            &repo,
        )
        .unwrap();
        std::fs::write(repo.join("owned.txt"), "after\n").unwrap();
        let patch = run_git(&["diff", "--binary", "HEAD", "--", "owned.txt"], &repo)
            .unwrap()
            .stdout;
        run_git(&["checkout", "--", "owned.txt"], &repo).unwrap();
        let mut evidence = temp.path().join("evidence");
        while evidence.to_string_lossy().len() < 280 {
            evidence = evidence.join("long-call-and-item-identity");
        }
        std::fs::create_dir_all(&evidence).unwrap();
        let file = evidence.join("item.patch");
        std::fs::write(&file, &patch).unwrap();
        apply_patch(&repo, file.to_str().unwrap(), &["owned.txt".into()]).unwrap();
        assert_eq!(
            std::fs::read_to_string(repo.join("owned.txt")).unwrap(),
            "after\n"
        );
        assert_eq!(std::fs::read(&file).unwrap(), patch);
    }
}

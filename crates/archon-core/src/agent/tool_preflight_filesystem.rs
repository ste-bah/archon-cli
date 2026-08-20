//! The filesystem baseline a mutating tool call is measured against.
//!
//! Every tool whose `WorkingTreeEffect` is `DeclaredPaths` or `Arbitrary` —
//! which is most of them, `Bash` and `FileWrite` included — walks and hashes
//! the working tree before it runs and again after, so the plan record can say
//! which files it actually touched.
//!
//! That walk must not see build output. It originally used a bare `WalkDir`
//! over the whole directory: in this repository that is 181,364 files and
//! 278 GB, nearly all of it `target/`, hashed twice per tool call. The turn did
//! not fail, it stopped — no output, no error, one core busy, forever. Files
//! git ignores are not part of the tree anyone audits, so they are skipped, and
//! what remains here is roughly five thousand files.

use std::io::Read;

use sha2::{Digest, Sha256};

use super::tool_types::{FileSystemObservation, ObservedEntry};
use super::*;

/// Entries one baseline will observe before giving up.
///
/// The ignore rules are what keep the walk small, so this only fires on a tree
/// that is genuinely enormous *after* they are applied. It exists because the
/// failure it replaces was a silent hang: a ceiling turns that into a sentence
/// naming the directory, which is something a person can act on.
const MAX_OBSERVED_ENTRIES: usize = 100_000;

impl Agent {
    pub(super) fn observe_filesystem_before_mutation(
        &self,
        effect: archon_tools::tool::WorkingTreeEffect,
    ) -> Result<Option<FileSystemObservation>, String> {
        effect
            .requires_filesystem_observation()
            .then(|| filesystem_observation(&self.config.working_dir, true, MAX_OBSERVED_ENTRIES))
            .transpose()
    }

    pub(super) fn changed_files_after_mutation(
        &self,
        before: &FileSystemObservation,
    ) -> Result<Vec<String>, String> {
        let after = filesystem_observation(&before.root, true, MAX_OBSERVED_ENTRIES)?;
        let mut changed = after
            .entries
            .iter()
            .filter(|(path, entry)| before.entries.get(*path) != Some(*entry))
            .map(|(path, _)| observed_relative_display(path))
            .collect::<Result<Vec<_>, _>>()?;
        changed.extend(
            before
                .entries
                .keys()
                .filter(|path| !after.entries.contains_key(*path))
                .map(|path| observed_relative_display(path))
                .collect::<Result<Vec<_>, _>>()?,
        );
        changed.sort();
        changed.dedup();
        Ok(changed)
    }
}

fn filesystem_observation(
    root: &std::path::Path,
    reject_unsafe_symlinks: bool,
    max_entries: usize,
) -> Result<FileSystemObservation, String> {
    let canonical_root = root.canonicalize().map_err(|error| {
        format!(
            "cannot resolve working directory {}: {error}",
            root.display()
        )
    })?;
    let mut entries = std::collections::BTreeMap::new();
    for entry in ignored_aware_walk(root) {
        let entry = entry.map_err(|error| format!("cannot walk {}: {error}", root.display()))?;
        let path = entry.path();
        if path == root {
            continue;
        }
        if entries.len() >= max_entries {
            return Err(format!(
                "{} holds more than {max_entries} files that git does not ignore, so a \
                 per-tool-call baseline of it cannot be taken; add the generated directories to \
                 .gitignore",
                root.display()
            ));
        }
        let relative = path
            .strip_prefix(root)
            .map_err(|error| format!("cannot relativize {}: {error}", path.display()))?
            .to_path_buf();
        // `ignore` reports no file type only for stdin, which cannot appear here.
        let Some(file_type) = entry.file_type() else {
            continue;
        };
        let observed = if file_type.is_file() {
            ObservedEntry::File(observed_file(path)?)
        } else if file_type.is_dir() {
            ObservedEntry::Directory
        } else if file_type.is_symlink() {
            let target = std::fs::read_link(path)
                .map_err(|error| format!("cannot read symlink {}: {error}", path.display()))?;
            if reject_unsafe_symlinks {
                validate_symlink_target(&canonical_root, path, &target)?;
            }
            ObservedEntry::Symlink(target)
        } else {
            continue;
        };
        entries.insert(relative, observed);
    }
    Ok(FileSystemObservation {
        root: root.to_path_buf(),
        entries,
    })
}

/// Walk `root`, skipping what git ignores and the `.git` directory itself.
///
/// Hidden entries are kept: `.archon/`, `.github/` and `.cargo/` are working
/// tree, and a tool that edits one has changed the repository. `.git` is pruned
/// because it churns on its own — an index refresh between the two baselines
/// would be reported as a file the tool touched.
///
/// Ignore files are read from `root` downwards only. Parent directories and the
/// developer's global excludes are deliberately not consulted: what counts as
/// working tree is a property of the repository, not of the machine, and two
/// people running the same tool must record the same changed files.
fn ignored_aware_walk(root: &std::path::Path) -> ignore::Walk {
    ignore::WalkBuilder::new(root)
        .hidden(false)
        .parents(false)
        .git_global(false)
        .git_ignore(true)
        .git_exclude(true)
        // Honour `.gitignore` even when the directory is not a checkout, so a
        // temporary tree behaves the same way a repository does.
        .require_git(false)
        .follow_links(false)
        .filter_entry(|entry| entry.file_name() != ".git")
        .build()
}

fn observed_relative_display(path: &std::path::Path) -> Result<String, String> {
    path.to_str()
        .map(|path| path.replace('\\', "/"))
        .ok_or_else(|| {
            format!(
                "non-UTF-8 path cannot be durably recorded: {}",
                path.display()
            )
        })
}

fn validate_symlink_target(
    canonical_root: &std::path::Path,
    link: &std::path::Path,
    target: &std::path::Path,
) -> Result<(), String> {
    let resolved = link
        .parent()
        .unwrap_or(canonical_root)
        .join(target)
        .canonicalize()
        .map_err(|error| format!("cannot resolve symlink {}: {error}", link.display()))?;
    if !resolved.starts_with(canonical_root) {
        return Err(format!(
            "symlink {} resolves outside working directory: {}",
            link.display(),
            resolved.display()
        ));
    }
    Ok(())
}

fn observed_file(path: &std::path::Path) -> Result<String, String> {
    let mut file = std::fs::File::open(path)
        .map_err(|error| format!("cannot open {}: {error}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 8192];
    loop {
        let bytes = file
            .read(&mut buffer)
            .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
        if bytes == 0 {
            break;
        }
        hasher.update(&buffer[..bytes]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

#[cfg(test)]
mod tests {
    //! What the baseline may and may not see.
    //!
    //! There were no tests here at all, which is how a walk that hashed every
    //! byte of `target/` twice per tool call reached a release: nothing ever
    //! asserted what the walk was allowed to look at.

    use super::*;
    use std::path::Path;

    /// A tree with one source file and one build artefact the ignore file
    /// excludes.
    fn tree() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("temp dir");
        let root = dir.path();
        std::fs::write(root.join(".gitignore"), "target/\n*.log\n").expect("gitignore");
        std::fs::write(root.join("main.rs"), "fn main() {}").expect("source");
        std::fs::create_dir_all(root.join("target/debug")).expect("target");
        std::fs::write(root.join("target/debug/app.exe"), vec![0_u8; 4096]).expect("artefact");
        std::fs::write(root.join("build.log"), "noise").expect("log");
        dir
    }

    fn observe(root: &Path) -> FileSystemObservation {
        filesystem_observation(root, true, MAX_OBSERVED_ENTRIES).expect("observation")
    }

    fn observed(observation: &FileSystemObservation, relative: &str) -> bool {
        observation.entries.contains_key(Path::new(relative))
    }

    /// The defect this module was built to stop: `target/` is 278 GB in this
    /// repository, and hashing it before and after every tool call did not slow
    /// the turn down, it stopped it.
    #[test]
    fn what_git_ignores_is_not_hashed() {
        let dir = tree();
        let seen = observe(dir.path());

        assert!(observed(&seen, "main.rs"), "source file must be observed");
        assert!(
            !observed(&seen, "target/debug/app.exe"),
            "an ignored build artefact was hashed"
        );
        assert!(!observed(&seen, "build.log"), "an ignored log was hashed");
    }

    /// Git's own directory rewrites itself between the two baselines, so
    /// observing it reports index churn as files the tool changed.
    #[test]
    fn the_git_directory_is_never_observed() {
        let dir = tree();
        std::fs::create_dir_all(dir.path().join(".git")).expect("git dir");
        std::fs::write(dir.path().join(".git/index"), "0").expect("index");

        assert!(!observed(&observe(dir.path()), ".git/index"));
    }

    /// Hidden is not the same as ignored. Editing `.github/workflows/ci.yml`
    /// changes the repository and has to be recorded.
    #[test]
    fn hidden_working_tree_files_are_observed() {
        let dir = tree();
        std::fs::create_dir_all(dir.path().join(".github/workflows")).expect("workflows");
        std::fs::write(dir.path().join(".github/workflows/ci.yml"), "on: push").expect("ci");

        let seen = observe(dir.path());
        assert!(observed(&seen, ".github/workflows/ci.yml"));
        assert!(observed(&seen, ".gitignore"), "the ignore file is tree too");
    }

    /// Skipping ignored paths must not cost the module its actual job.
    #[test]
    fn an_edit_to_a_tracked_file_still_changes_the_observation() {
        let dir = tree();
        let before = observe(dir.path());
        std::fs::write(dir.path().join("main.rs"), "fn main() { todo!() }").expect("edit");
        let after = observe(dir.path());

        assert_ne!(
            before.entries.get(Path::new("main.rs")),
            after.entries.get(Path::new("main.rs")),
            "an edited file hashed the same before and after"
        );
    }

    /// Writing into an ignored directory is invisible, which is the point: it
    /// is not a change to the working tree anyone reviews.
    #[test]
    fn writing_into_an_ignored_directory_changes_nothing() {
        let dir = tree();
        let before = observe(dir.path());
        std::fs::write(dir.path().join("target/debug/app.exe"), vec![1_u8; 8192]).expect("rebuild");

        assert_eq!(before.entries, observe(dir.path()).entries);
    }

    /// The hang this replaced was silent. A tree that is still too large after
    /// the ignore rules must say so.
    #[test]
    fn an_unmanageably_large_tree_is_reported_rather_than_walked() {
        let dir = tree();
        // Two observable files already (`.gitignore`, `main.rs`); a third puts
        // the tree over a ceiling of two.
        std::fs::write(dir.path().join("lib.rs"), "pub fn f() {}").expect("second source");

        // Matched rather than `expect_err`: the success value is a map of every
        // hash in the tree and has no business being formatted into a panic.
        let Err(error) = filesystem_observation(dir.path(), true, 2) else {
            panic!("a tree over the ceiling was walked instead of refused");
        };

        assert!(error.contains("more than 2 files"), "got {error}");
        assert!(error.contains(".gitignore"), "no remedy offered: {error}");
    }
}

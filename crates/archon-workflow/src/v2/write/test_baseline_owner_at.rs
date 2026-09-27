//! Issue-118: which file a failing test lived in AT A COMMIT.
//!
//! [`super::test_baseline_owner::test_file`] answers from the working tree,
//! which is right while the verifier's tree is the working tree. A record
//! read later -- the second residual pass, the final gate -- must answer for
//! the tree the verifier judged, whatever landed since: a module added or
//! moved afterwards must not change which file a recorded failure names. The
//! module walk is the same (longest prefix first, `<path>.rs`,
//! `<path>/mod.rs`, the `#[path]` sibling `<parent>_tests.rs`, and
//! `<pkg>/tests/<name>` for an integration target), read against the files
//! git holds at `commit`. Package directories are read from the working
//! tree's manifests; a package whose `src` holds no file at `commit` is not
//! one there.

use std::collections::BTreeSet;
use std::path::Path;

use super::focused_test_targets::{Selection, selection};
use super::test_baseline_owner::package_dirs_for;

/// The repo-relative file `test_id` lived in at `commit` (the working tree
/// when `None` or unreadable), when exactly one package the command names
/// reaches one.
pub(crate) fn test_file_at(
    root: &Path,
    commit: Option<&str>,
    command: &str,
    test_id: &str,
) -> Option<String> {
    let files = crate::v2::script::residual_patterns::tree_files(root, commit);
    let segments: Vec<&str> = test_id.split("::").collect();
    let (_, modules) = segments.split_last()?;
    let mut found: BTreeSet<String> = BTreeSet::new();
    for dir in package_dirs_for(root, command) {
        let base = |sub: &str| {
            if dir.is_empty() {
                sub.to_string()
            } else {
                format!("{dir}/{sub}")
            }
        };
        let hit = match selection(command) {
            Selection::IntegrationTest(name) => {
                let stem = base(&format!("tests/{name}"));
                prefix_in(&files, &stem, modules).or_else(|| {
                    [format!("{stem}.rs"), format!("{stem}/main.rs")]
                        .into_iter()
                        .find(|candidate| files.contains(candidate))
                })
            }
            _ => {
                let src = base("src");
                prefix_in(&files, &src, modules).or_else(|| crate_root_in(&files, &src, modules))
            }
        };
        found.extend(hit);
    }
    (found.len() == 1).then(|| found.into_iter().next().unwrap_or_default())
}

/// The longest prefix of `modules` that reaches a file under `src`.
fn prefix_in(files: &BTreeSet<String>, src: &str, modules: &[&str]) -> Option<String> {
    for len in (1..=modules.len()).rev() {
        let prefix = &modules[..len];
        let joined = prefix.join("/");
        let mut candidates = vec![
            format!("{src}/{joined}.rs"),
            format!("{src}/{joined}/mod.rs"),
        ];
        if prefix[len - 1] == "tests" && len >= 2 {
            candidates.push(format!("{src}/{}_tests.rs", prefix[..len - 1].join("/")));
        }
        if let Some(hit) = candidates.into_iter().find(|c| files.contains(c)) {
            return Some(hit);
        }
    }
    None
}

/// An inline `tests` module at the crate root lives in the root file.
fn crate_root_in(files: &BTreeSet<String>, src: &str, modules: &[&str]) -> Option<String> {
    if modules.is_empty() || !modules.iter().all(|module| *module == "tests") {
        return None;
    }
    ["lib.rs", "main.rs"]
        .iter()
        .map(|root| format!("{src}/{root}"))
        .find(|c| files.contains(c))
}

/// The file of the module that declares the test's own module, at `commit`:
/// for `a::b::t` (test `t` in module `a::b`), the file module `a` lives in.
/// `None` when the test's module is a crate or target root, or the parent
/// resolves to the test's own file.
pub(crate) fn parent_module_file_at(
    root: &Path,
    commit: Option<&str>,
    command: &str,
    test_id: &str,
) -> Option<String> {
    let segments: Vec<&str> = test_id.split("::").collect();
    if segments.len() < 3 {
        return None;
    }
    let parent = segments[..segments.len() - 2].join("::");
    let own = test_file_at(root, commit, command, test_id);
    test_file_at(root, commit, command, &format!("{parent}::parent"))
        .filter(|parent| Some(parent) != own.as_ref())
}

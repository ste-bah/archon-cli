//! What a failing check implicates, split from `acceptance_routing` for
//! size: the repository files its failure output locates and every file the
//! landing it regressed at changed, less the check's own sources.

use std::collections::BTreeSet;
use std::path::Path;

use super::super::acceptance_signals::failure_locations;
use super::super::script::residual_paths::is_repo_file;

/// Whether `file` is the check's own source rather than what it checks,
/// read from the frozen command's structure alone: the program it runs, the
/// first argument after the program (a script or test an interpreter or
/// runner executes), or a flag's value naming it by path or stem (a test or
/// script target, `--flag name`). A file the command only reads or searches
/// (a pattern comes first) is what the check inspects, never excluded. A
/// remediation fixes the implementation; it is never handed the check.
pub(super) fn own_source(file: &str, command: &str) -> bool {
    let stem = Path::new(file)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let names = |token: &str| {
        let token = token.trim_matches(|c| c == '\'' || c == '"');
        token.strip_prefix("./").unwrap_or(token) == file
    };
    let mut tokens = command
        .split_whitespace()
        .skip_while(|token| token.contains('=') && !token.starts_with('-'));
    let Some(program) = tokens.next() else {
        return false;
    };
    if names(program) {
        return true;
    }
    let rest: Vec<&str> = tokens.collect();
    if rest
        .iter()
        .find(|token| !token.starts_with('-'))
        .is_some_and(|t| names(t))
    {
        return true;
    }
    rest.windows(2).any(|pair| {
        pair[0].starts_with('-')
            && !pair[1].starts_with('-')
            && (names(pair[1]) || (stem.len() >= 3 && pair[1] == stem))
    })
}

/// The files one check implicates: its output's failure locations (stderr
/// first), every one of them, then every file its blamed landing changed; less the
/// check's own sources, which are recorded as unwritable. The second list
/// is the implicated files the landing changed.
pub(super) fn implicated(
    check: &super::super::acceptance_stage::AcceptanceCheckRecordV1,
    root: &Path,
    command: &str,
    own: &mut Vec<(String, String)>,
) -> (Vec<String>, BTreeSet<String>) {
    let why = "the check's own source, never the remediation's to change";
    let mut files = Vec::new();
    // Every failure location is routed: a cap left the seventh file and
    // later ungranted, so the unit could not write the one that broke.
    let located = failure_locations(&check.stderr_tail, root, usize::MAX)
        .into_iter()
        .chain(failure_locations(&check.stdout_tail, root, usize::MAX));
    for file in located {
        if files.contains(&file) {
            continue;
        }
        if own_source(&file, command) {
            own.push((file, why.into()));
        } else {
            files.push(file);
        }
    }
    let mut landed = BTreeSet::new();
    if let Some(regression) = &check.regressed_by {
        for file in &regression.changed_files {
            // A file the landing deleted is implicated too: restoring it
            // may be the fix.
            let present = is_repo_file(root, file) || deleted_repo_path(root, file);
            if !present || own.iter().any(|(own, _)| own == file) {
                continue;
            }
            if own_source(file, command) {
                own.push((file.clone(), why.into()));
                continue;
            }
            landed.insert(file.clone());
            if !files.contains(file) {
                files.push(file.clone());
            }
        }
    }
    (files, landed)
}

/// Whether `relative` names a path that does not exist but would lie inside
/// `root`: a plain relative path whose nearest existing ancestor resolves
/// inside the repository (a file a landing deleted).
fn deleted_repo_path(root: &Path, relative: &str) -> bool {
    let path = Path::new(relative);
    let plain = path
        .components()
        .all(|part| matches!(part, std::path::Component::Normal(_)));
    if !plain || relative.is_empty() || std::fs::symlink_metadata(root.join(path)).is_ok() {
        return false;
    }
    let Ok(base) = root.canonicalize().map(archon_shell::paths::plain) else {
        return false;
    };
    (root.join(path).ancestors().skip(1))
        .find(|ancestor| ancestor.exists())
        .and_then(|ancestor| ancestor.canonicalize().map(archon_shell::paths::plain).ok())
        .is_some_and(|ancestor| ancestor.starts_with(&base))
}

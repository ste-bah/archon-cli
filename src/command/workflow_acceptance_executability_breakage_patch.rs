//! Byte-preserving modify-only patch application in an isolated tree.
use super::*;
use archon_workflow::base_snapshot::key_path;
use archon_workflow::breakage_proof::RunOutcome;
use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::time::{Duration, SystemTime};
pub(super) fn query(
    copy: &HermeticCopy,
    args: &[&OsStr],
) -> Result<std::process::Output, RunOutcome> {
    let git = copy
        .isolated_git
        .as_ref()
        .ok_or_else(|| decide::unrun("host_io", "isolated operation unavailable"))?;
    let top = copy.root.join("repo");
    if [&copy.root, &top]
        .into_iter()
        .any(|p| !p.symlink_metadata().is_ok_and(|m| m.is_dir()))
    {
        return Err(decide::unrun(
            "host_io",
            "disposable root is no longer a directory",
        ));
    }
    let mut all = vec![OsStr::new("-C"), top.as_os_str()];
    all.extend_from_slice(args);
    git.output(&all).map_err(host)
}
pub(super) fn host(e: Unrun) -> RunOutcome {
    use crate::command::workflow_host_probe_fault::UnrunCause;
    decide::unrun(
        match e.2 {
            Some(UnrunCause::BaselineUnavailable) => "baseline_unavailable",
            Some(UnrunCause::BaselineFetchFailed) => "baseline_fetch_failed",
            Some(UnrunCause::SnapshotTampered) => "snapshot_tampered",
            None => "host_io",
        },
        &e.0,
    )
}
fn refused(code: &str, detail: &[u8]) -> RunOutcome {
    RunOutcome::Inapplicable {
        code: code.into(),
        detail: String::from_utf8_lossy(detail).into_owned(),
    }
}
fn apply_args(
    copy: &HermeticCopy,
    file: &Path,
    flags: &[&str],
) -> Result<std::process::Output, RunOutcome> {
    let mut args = vec![OsStr::new("apply"), OsStr::new("--whitespace=nowarn")];
    args.extend(flags.iter().map(OsStr::new));
    let directory = format!("--directory={}", relative_key(&copy.prefix));
    if !copy.prefix.as_os_str().is_empty() {
        args.push(OsStr::new(&directory));
    }
    args.extend([OsStr::new("--"), file.as_os_str()]);
    query(copy, &args)
}
fn paths(bytes: &[u8]) -> BTreeSet<Vec<u8>> {
    bytes
        .split(|b| *b == 0)
        .filter(|p| !p.is_empty())
        .map(Vec::from)
        .collect()
}
fn remember(copy: &HermeticCopy, touched: &[String]) -> Result<(), RunOutcome> {
    let mut modified = copy
        .modified
        .lock()
        .map_err(|e| decide::unrun("host_io", &e.to_string()))?;
    for key in touched {
        match copy.repository.join(key_path(key)).symlink_metadata() {
            Ok(meta) if meta.is_file() => {
                let time = meta
                    .modified()
                    .map_err(|e| decide::unrun("mtime_not_set", &e.to_string()))?;
                modified
                    .entry(key.clone())
                    .and_modify(|old| *old = (*old).max(time))
                    .or_insert(time);
            }
            Err(e)
                if !matches!(
                    e.kind(),
                    std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
                ) =>
            {
                return Err(decide::unrun("mtime_not_set", &e.to_string()));
            }
            _ => {}
        }
    }
    Ok(())
}
pub(super) fn bump(copy: &HermeticCopy, touched: &[String]) -> Result<(), RunOutcome> {
    remember(copy, touched)?;
    let mut modified = copy
        .modified
        .lock()
        .map_err(|e| decide::unrun("host_io", &e.to_string()))?;
    for key in touched {
        let path = copy.repository.join(key_path(key));
        let result = (|| -> std::io::Result<()> {
            let file = std::fs::OpenOptions::new().write(true).open(path)?;
            let old = modified.get(key).copied().unwrap_or(SystemTime::UNIX_EPOCH);
            let next = old
                .checked_add(Duration::from_secs(2))
                .ok_or_else(|| std::io::Error::other("modification time outside range"))?
                .max(SystemTime::now());
            file.set_modified(next)?;
            modified.insert(key.clone(), next);
            Ok(())
        })();
        result.map_err(|e| decide::unrun("mtime_not_set", &e.to_string()))?;
    }
    Ok(())
}
pub(super) fn inspect(
    copy: &HermeticCopy,
    file: &Path,
    bytes: &[u8],
) -> Result<(Vec<String>, BTreeSet<Vec<u8>>), RunOutcome> {
    std::fs::write(file, bytes).map_err(|e| decide::unrun("host_io", &e.to_string()))?;
    let stat = apply_args(copy, file, &["--numstat", "-z", "--allow-empty"])?;
    if !stat.status.success() {
        return Err(refused(
            if stat.status.code() == Some(128)
                && String::from_utf8_lossy(&stat.stderr).contains("invalid path")
            {
                "patch_path_unsafe"
            } else {
                "patch_does_not_apply"
            },
            &stat.stderr,
        ));
    }
    let summary = apply_args(copy, file, &["--summary", "--allow-empty"])?;
    if !summary.status.success() {
        return Err(refused("patch_does_not_apply", &summary.stderr));
    }
    if !summary.stdout.is_empty() {
        return Err(refused("patch_not_modify_only", &summary.stdout));
    }
    let mut expected = BTreeSet::new();
    let mut changed = false;
    let mut touched = Vec::new();
    for row in stat.stdout.split(|b| *b == 0).filter(|p| !p.is_empty()) {
        let mut fields = row.splitn(3, |b| *b == b'\t');
        let add = fields.next().unwrap_or_default();
        let del = fields.next().unwrap_or_default();
        if add == b"-" || del == b"-" {
            return Err(refused("patch_binary", &[]));
        }
        changed |= add != b"0" || del != b"0";
        let name = fields
            .next()
            .ok_or_else(|| refused("patch_path_unsafe", row))?;
        let text = std::str::from_utf8(name).map_err(|_| refused("patch_path_unsafe", name))?;
        let prefix = relative_key(&copy.prefix);
        let key = if prefix.is_empty() {
            text
        } else {
            text.strip_prefix(&format!("{prefix}/"))
                .ok_or_else(|| refused("patch_outside_root", name))?
        };
        if archon_workflow::base_snapshot::outside_key(key) {
            return Err(refused("patch_path_unsafe", name));
        }
        let path = copy.repository.join(key_path(key));
        match path.symlink_metadata() {
            Ok(meta) if !meta.is_file() => return Err(refused("patch_path_unsafe", name)),
            Err(e)
                if !matches!(
                    e.kind(),
                    std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
                ) =>
            {
                return Err(decide::unrun("host_io", &e.to_string()));
            }
            _ => {}
        }
        let relative = key_path(key);
        if relative.ancestors().skip(1).any(|p| {
            copy.repository
                .join(p)
                .symlink_metadata()
                .is_ok_and(|m| m.file_type().is_symlink())
        }) {
            return Err(refused("patch_path_unsafe", name));
        }
        touched.push(key.into());
        expected.insert(name.to_vec());
    }
    if !changed {
        return Err(refused("patch_no_change", &[]));
    }
    remember(copy, &touched)?;
    Ok((touched, expected))
}
pub(super) fn apply(
    copy: &HermeticCopy,
    file: &Path,
    bytes: &[u8],
) -> Result<Vec<String>, RunOutcome> {
    let (touched, expected) = inspect(copy, file, bytes)?;
    let applied = apply_args(copy, file, &[])?;
    if !applied.status.success() {
        return Err(refused(
            if applied.status.code() == Some(128)
                && String::from_utf8_lossy(&applied.stderr).contains("invalid path")
            {
                "patch_path_unsafe"
            } else {
                "patch_does_not_apply"
            },
            &applied.stderr,
        ));
    }
    let diff = query(
        copy,
        &[
            "diff".as_ref(),
            "--name-only".as_ref(),
            "-z".as_ref(),
            "--no-renames".as_ref(),
            "--no-ext-diff".as_ref(),
        ],
    )?;
    if !diff.status.success() || paths(&diff.stdout) != expected {
        return Err(refused("apply_unverified", &diff.stderr));
    }
    bump(copy, &touched)?;
    Ok(touched)
}
fn clean(copy: &HermeticCopy) -> Result<bool, RunOutcome> {
    let result = query(
        copy,
        &[
            "diff".as_ref(),
            "--quiet".as_ref(),
            "--no-ext-diff".as_ref(),
        ],
    )?;
    match result.status.code() {
        Some(0) => Ok(true),
        Some(1) => Ok(false),
        _ => Err(decide::unrun(
            "host_io",
            &String::from_utf8_lossy(&result.stderr),
        )),
    }
}
pub(super) fn restore(
    copy: &HermeticCopy,
    file: &Path,
    touched: &[String],
    inventory: &BTreeSet<PathBuf>,
) -> Result<u32, RunOutcome> {
    remember(copy, touched)?;
    let mut restored = 0;
    for delay in [0, 200, 1000, 3000] {
        if delay > 0 {
            std::thread::sleep(Duration::from_millis(delay));
        }
        if !touched.is_empty() {
            let _ = apply_args(copy, file, &["-R"]);
        }
        if !clean(copy).unwrap_or(false) {
            let checkout = query(
                copy,
                &[
                    "checkout".as_ref(),
                    "--quiet".as_ref(),
                    "--".as_ref(),
                    ".".as_ref(),
                ],
            );
            if checkout.is_ok_and(|o| o.status.success()) {
                restored = 1;
            }
        }
        if clean(copy).unwrap_or(false) {
            cleanup(copy, inventory)?;
            bump(copy, touched)?;
            return Ok(restored);
        }
    }
    Err(decide::unrun(
        "copy_poisoned",
        "tracked state could not be restored",
    ))
}
fn walk(root: &Path, result: &mut BTreeSet<PathBuf>) -> std::io::Result<()> {
    if !root.symlink_metadata()?.is_dir() {
        return Err(std::io::Error::other(
            "inventory root is no longer a directory",
        ));
    }
    for item in std::fs::read_dir(root)? {
        let item = item?;
        let path = item.path();
        result.insert(path.clone());
        if item.file_type()?.is_dir() {
            walk(&path, result)?;
        }
    }
    Ok(())
}
pub(super) fn inventory(copy: &HermeticCopy) -> Result<BTreeSet<PathBuf>, RunOutcome> {
    let result = query(
        copy,
        &[
            "ls-files".as_ref(),
            "-z".as_ref(),
            "--others".as_ref(),
            "--directory".as_ref(),
        ],
    )?;
    if !result.status.success() {
        return Err(decide::unrun(
            "host_io",
            &String::from_utf8_lossy(&result.stderr),
        ));
    }
    let top = copy.root.join("repo");
    if [&copy.root, &top]
        .into_iter()
        .any(|p| !p.symlink_metadata().is_ok_and(|m| m.is_dir()))
    {
        return Err(decide::unrun(
            "host_io",
            "disposable root is no longer a directory",
        ));
    }
    let mut all = BTreeSet::new();
    for name in paths(&result.stdout) {
        let text =
            std::str::from_utf8(&name).map_err(|e| decide::unrun("host_io", &e.to_string()))?;
        let path = top.join(key_path(text));
        if path.symlink_metadata().is_ok_and(|m| m.is_dir()) {
            walk(&path, &mut all).map_err(|e| decide::unrun("host_io", &e.to_string()))?;
        }
        all.insert(path);
    }
    if !copy.project.starts_with(top) {
        walk(&copy.project, &mut all).map_err(|e| decide::unrun("host_io", &e.to_string()))?;
    }
    Ok(all)
}
pub(super) fn cleanup(copy: &HermeticCopy, before: &BTreeSet<PathBuf>) -> Result<(), RunOutcome> {
    let now = inventory(copy)?;
    let mut extra: Vec<_> = now.difference(before).collect();
    extra.sort_by_key(|p| std::cmp::Reverse(p.components().count()));
    for path in extra {
        let meta = path
            .symlink_metadata()
            .map_err(|e| decide::unrun("host_io", &e.to_string()))?;
        let result = if meta.file_type().is_symlink() {
            std::fs::remove_dir_all(path)
        } else if meta.is_dir() {
            std::fs::remove_dir(path)
        } else {
            std::fs::remove_file(path)
        };
        result.map_err(|e| decide::unrun("host_io", &e.to_string()))?;
    }
    Ok(())
}

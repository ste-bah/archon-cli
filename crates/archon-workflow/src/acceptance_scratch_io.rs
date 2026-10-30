use super::*;
use std::io::Read;

pub(super) fn read(path: &Path) -> WorkflowResult<Vec<u8>> {
    let mut opts = std::fs::OpenOptions::new();
    opts.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let mut file = opts.open(path).map_err(|e| WorkflowError::io(path, e))?;
    if !file
        .metadata()
        .map_err(|e| WorkflowError::io(path, e))?
        .is_file()
    {
        return Err(invalid("snapshot input is not a regular file"));
    }
    let mut bytes = Vec::new();
    let mut chunk = [0; 65536];
    loop {
        control::check()?;
        let n = file
            .read(&mut chunk)
            .map_err(|e| WorkflowError::io(path, e))?;
        if n == 0 {
            break;
        }
        bytes.extend_from_slice(&chunk[..n]);
    }
    Ok(bytes)
}
pub(super) fn copy_tree(
    source: &Path,
    dest: &Path,
    remaining: &mut u64,
    exclude_git: bool,
) -> WorkflowResult<()> {
    copy_tree_with_cause(source, dest, remaining, exclude_git, false)
}

pub(super) fn copy_sibling_tree(
    source: &Path,
    dest: &Path,
    remaining: &mut u64,
    exclude_git: bool,
) -> WorkflowResult<()> {
    copy_tree_with_cause(source, dest, remaining, exclude_git, true)
}

fn copy_tree_with_cause(
    source: &Path,
    dest: &Path,
    remaining: &mut u64,
    exclude_git: bool,
    typed_sibling: bool,
) -> WorkflowResult<()> {
    copy_tree_inner(
        source,
        dest,
        remaining,
        CopyOptions {
            exclude_git,
            cache: false,
            exclude_freeze_owned: false,
            typed_sibling,
        },
        Path::new(""),
    )
}
/// Copy a task root without freeze-owned artifacts, which are not check inputs.
pub(super) fn copy_task_root(
    source: &Path,
    dest: &Path,
    project_relative: &Path,
    task_root: &Path,
    (excludes, nested): (&[PathBuf], Option<&Path>),
    remaining: &mut u64,
) -> WorkflowResult<()> {
    copy_task_root_inner(
        source,
        dest,
        remaining,
        project_relative,
        task_root,
        (excludes, nested),
    )
}

fn copy_task_root_inner(
    source: &Path,
    dest: &Path,
    remaining: &mut u64,
    relative: &Path,
    task_root: &Path,
    (excludes, nested): (&[PathBuf], Option<&Path>),
) -> WorkflowResult<()> {
    control::check()?;
    if super::acceptance_input_excluded(relative, excludes, Some(task_root))
        || super::inputs::nested_repository_skipped(relative, nested)
    {
        return Ok(());
    }
    let meta = std::fs::symlink_metadata(source).map_err(|e| WorkflowError::io(source, e))?;
    if meta.is_dir() {
        if let Ok(existing) = dest.symlink_metadata()
            && (!existing.is_dir() || existing.file_type().is_symlink())
        {
            return Err(invalid("scratch input directory collision"));
        }
        std::fs::create_dir_all(dest).map_err(|e| WorkflowError::io(dest, e))?;
        for item in std::fs::read_dir(source).map_err(|e| WorkflowError::io(source, e))? {
            let item = item.map_err(|e| WorkflowError::io(source, e))?;
            copy_task_root_inner(
                &item.path(),
                &dest.join(item.file_name()),
                remaining,
                &relative.join(item.file_name()),
                task_root,
                (excludes, nested),
            )?;
        }
    } else if meta.is_file() {
        if meta.len() > *remaining {
            return Err(invalid("scratch copy exceeds configured size limit"));
        }
        let bytes = read(source)?;
        if let Ok(existing) = dest.symlink_metadata() {
            if !existing.is_file() || read(dest)? != bytes {
                return Err(invalid(format!(
                    "nonidentical scratch path collision: {} already holds a copy from an earlier overlay source and {} differs from it",
                    dest.display(),
                    source.display()
                )));
            }
            return Ok(());
        }
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).map_err(|e| WorkflowError::io(parent, e))?;
        }
        std::fs::write(dest, &bytes).map_err(|e| WorkflowError::io(dest, e))?;
        std::fs::set_permissions(dest, meta.permissions())
            .map_err(|e| WorkflowError::io(dest, e))?;
        *remaining -= bytes.len() as u64;
    } else {
        return Err(invalid(format!(
            "nonregular or symlink snapshot input refused: {}",
            source.display()
        )));
    }
    Ok(())
}
/// The caller selects only registry/git cache subtrees, never Cargo home itself.
pub(super) fn copy_cache(source: &Path, dest: &Path, remaining: &mut u64) -> WorkflowResult<()> {
    copy_tree_inner(
        source,
        dest,
        remaining,
        CopyOptions {
            exclude_git: false,
            cache: true,
            exclude_freeze_owned: false,
            typed_sibling: false,
        },
        Path::new(""),
    )
}

#[derive(Clone, Copy)]
struct CopyOptions {
    exclude_git: bool,
    cache: bool,
    exclude_freeze_owned: bool,
    typed_sibling: bool,
}

fn copy_tree_inner(
    source: &Path,
    dest: &Path,
    remaining: &mut u64,
    options: CopyOptions,
    relative: &Path,
) -> WorkflowResult<()> {
    if options.cache
        && source.components().any(|part| {
            matches!(
                part.as_os_str().to_str(),
                Some("credentials" | "credentials.toml" | ".env")
            )
        })
    {
        return Err(invalid(
            "credential file cannot be exported from Cargo cache",
        ));
    }
    control::check()?;
    let meta = std::fs::symlink_metadata(source).map_err(|e| WorkflowError::io(source, e))?;
    if meta.is_dir() {
        if let Ok(existing) = dest.symlink_metadata()
            && (!existing.is_dir() || existing.file_type().is_symlink())
        {
            return Err(copy_shape_error(
                options.typed_sibling,
                "scratch input directory collision",
            ));
        }
        std::fs::create_dir_all(dest).map_err(|e| WorkflowError::io(dest, e))?;
        for item in std::fs::read_dir(source).map_err(|e| WorkflowError::io(source, e))? {
            let item = item.map_err(|e| WorkflowError::io(source, e))?;
            let name = item.file_name();
            let child_relative = relative.join(&name);
            if options.exclude_git && name == ".git" {
                continue;
            }
            if options.exclude_freeze_owned
                && super::acceptance_input_excluded(&child_relative, &[], Some(Path::new("")))
            {
                continue;
            }
            copy_tree_inner(
                &item.path(),
                &dest.join(name),
                remaining,
                options,
                &child_relative,
            )?;
        }
    } else if meta.is_file() {
        if meta.len() > *remaining {
            return Err(copy_size_error(options.typed_sibling));
        }
        let bytes = read(source)?;
        if let Ok(existing) = dest.symlink_metadata() {
            if !existing.is_file() || read(dest)? != bytes {
                let message = format!(
                    "nonidentical scratch path collision: {} already holds a copy from an earlier overlay source and {} differs from it",
                    dest.display(),
                    source.display()
                );
                return Err(copy_shape_error(options.typed_sibling, &message));
            }
            return Ok(());
        }
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).map_err(|e| WorkflowError::io(parent, e))?;
        }
        std::fs::write(dest, &bytes).map_err(|e| WorkflowError::io(dest, e))?;
        std::fs::set_permissions(dest, meta.permissions())
            .map_err(|e| WorkflowError::io(dest, e))?;
        *remaining -= bytes.len() as u64;
    } else {
        let message = format!(
            "nonregular or symlink snapshot input refused: {}",
            source.display()
        );
        // Refusing a symlink or special file in host-supplied snapshot data
        // does not prove the check authored that input.
        return Err(invalid(message));
    }
    Ok(())
}

fn copy_size_error(typed_sibling: bool) -> WorkflowError {
    if typed_sibling {
        WorkflowError::ScratchCopyTooLarge
    } else {
        invalid("scratch copy exceeds configured size limit")
    }
}

fn copy_shape_error(typed_sibling: bool, message: &str) -> WorkflowError {
    if typed_sibling {
        WorkflowError::ScratchCopyShape(message.to_owned())
    } else {
        invalid(message)
    }
}

pub(super) fn readonly(path: &Path) -> WorkflowResult<()> {
    control::check()?;
    let meta = std::fs::symlink_metadata(path).map_err(|e| WorkflowError::io(path, e))?;
    if meta.is_dir() {
        for item in std::fs::read_dir(path).map_err(|e| WorkflowError::io(path, e))? {
            readonly(&item.map_err(|e| WorkflowError::io(path, e))?.path())?;
        }
    }
    let mut permissions = meta.permissions();
    permissions.set_readonly(true);
    std::fs::set_permissions(path, permissions).map_err(|e| WorkflowError::io(path, e))
}
#[path = "acceptance_scratch_io_remove.rs"]
mod remove;
pub(super) use remove::remove_owned_tree;
#[cfg(test)]
pub(super) use remove::tests as remove_tests;
/// Exact byte inventory, including symbolic link targets without following them.
pub fn inventory(root: &Path) -> WorkflowResult<BTreeMap<String, String>> {
    inventory_with(root, &mut |path, _| {
        Ok(crate::task_set_contract::content_digest(&read(path)?))
    })
}
/// [`inventory`] with each regular file's content digest supplied by
/// `digest` (given the path and its unfollowed metadata), so a caller may
/// answer from a verified earlier hash (`acceptance_scratch_seal`). The
/// entries and their format are exactly [`inventory`]'s.
pub(super) fn inventory_with(
    root: &Path,
    digest: &mut dyn FnMut(&Path, &std::fs::Metadata) -> WorkflowResult<String>,
) -> WorkflowResult<BTreeMap<String, String>> {
    fn visit(
        root: &Path,
        path: &Path,
        out: &mut BTreeMap<String, String>,
        digest: &mut dyn FnMut(&Path, &std::fs::Metadata) -> WorkflowResult<String>,
    ) -> WorkflowResult<()> {
        control::check()?;
        let meta = std::fs::symlink_metadata(path).map_err(|e| WorkflowError::io(path, e))?;
        let key = super::relative_key(
            path.strip_prefix(root)
                .map_err(|_| invalid("inventory escaped root"))?,
        );
        let value = if meta.file_type().is_symlink() {
            format!(
                "link:{}",
                std::fs::read_link(path)
                    .map_err(|e| WorkflowError::io(path, e))?
                    .display()
            )
        } else if meta.is_dir() {
            "directory".into()
        } else if meta.is_file() {
            format!("file:{}:{}", meta.len(), digest(path, &meta)?)
        } else {
            return Err(invalid(format!(
                "nonregular live inventory path: {}",
                path.display()
            )));
        };
        out.insert(key, value);
        if meta.is_dir() {
            for item in std::fs::read_dir(path).map_err(|e| WorkflowError::io(path, e))? {
                visit(
                    root,
                    &item.map_err(|e| WorkflowError::io(path, e))?.path(),
                    out,
                    digest,
                )?;
            }
        }
        Ok(())
    }
    let mut result = BTreeMap::new();
    visit(root, root, &mut result, digest)?;
    Ok(result)
}
/// Host setting values replaced in child output, longest first.
/// Issue 544: `bytes` with each secret value replaced by a stable indexed
/// placeholder (`<secret#1>`, `<secret#2>`, ...) instead of one shared
/// marker, so distinct secrets stay distinct and no secret value is kept or
/// hashed. Indexes follow the secret set's own order, which is the same for
/// every observation made under the same bindings. (Sentry's message-based
/// grouping replaces variable values with placeholders before it groups.)
pub(super) fn placeholders(
    host_environment: &BTreeMap<String, String>,
    bytes: &[u8],
    truncated: bool,
) -> Vec<u8> {
    let secrets = archon_observability::secret_values::SecretValues::for_evidence(
        host_environment.values().map(String::as_str),
    );
    let mut output = bytes.to_vec();
    for (index, value) in secrets.iter().enumerate() {
        let needle = value.as_bytes();
        let marker = format!("<secret#{}>", index + 1);
        let mut clean = Vec::with_capacity(output.len());
        let mut at = 0;
        while at < output.len() {
            if output[at..].starts_with(needle) {
                clean.extend_from_slice(marker.as_bytes());
                at += needle.len();
            } else {
                clean.push(output[at]);
                at += 1;
            }
        }
        // A stream the collector cut can end inside a secret: its prefix is
        // the secret too, by the rule `redact` uses.
        for n in (1..if truncated {
            needle.len().min(clean.len() + 1)
        } else {
            1
        })
            .rev()
        {
            if clean.ends_with(&needle[..n]) {
                clean.truncate(clean.len() - n);
                clean.extend_from_slice(marker.as_bytes());
                break;
            }
        }
        output = clean;
    }
    output
}

pub(super) fn redact(
    host_environment: &BTreeMap<String, String>,
    bytes: &[u8],
    truncated: bool,
) -> Vec<u8> {
    let mut output = bytes.to_vec();
    let secrets = archon_observability::secret_values::SecretValues::for_evidence(
        host_environment.values().map(String::as_str),
    );
    for value in secrets.iter() {
        let needle = value.as_bytes();
        let mut clean = Vec::new();
        let mut at = 0;
        while at < output.len() {
            if output[at..].starts_with(needle) {
                clean.extend_from_slice(b"[REDACTED]");
                at += needle.len();
            } else {
                clean.push(output[at]);
                at += 1;
            }
        }
        // Only a stream actually capped by the collector can end with a
        // partial secret. Complete output must retain coincidental suffixes.
        for n in (1..if truncated {
            needle.len().min(clean.len() + 1)
        } else {
            1
        })
            .rev()
        {
            if clean.ends_with(&needle[..n]) {
                clean.truncate(clean.len() - n);
                clean.extend_from_slice(b"[REDACTED]");
                break;
            }
        }
        output = clean;
    }
    output
}

#[cfg(test)]
#[path = "acceptance_scratch_redaction_tests.rs"]
mod redaction_tests;

#[cfg(test)]
#[test]
fn task_root_copy_omits_freeze_owned_contract_and_lock() {
    let source = tempfile::tempdir().unwrap();
    let destination = tempfile::tempdir().unwrap();
    let contract = crate::task_set_contract::ACCEPTANCE_CONTRACT_FILE;
    let lock = crate::task_set_contract::ACCEPTANCE_LOCK_FILE;
    std::fs::write(source.path().join(contract), b"contract").unwrap();
    std::fs::write(source.path().join(lock), b"lock").unwrap();
    std::fs::write(source.path().join("input.json"), b"input").unwrap();
    std::fs::write(
        source
            .path()
            .join(crate::task_set_contract::TASK_SKELETON_FILE),
        b"skeleton",
    )
    .unwrap();
    std::fs::write(
        source
            .path()
            .join(crate::task_set_contract::TASK_SKELETON_LOCK_FILE),
        b"skeleton lock",
    )
    .unwrap();
    std::fs::write(source.path().join("TASK-EXAMPLE-001.md"), b"landed body").unwrap();
    std::fs::write(source.path().join(".decompose.log"), b"pipeline log").unwrap();

    let mut remaining = u64::MAX;
    copy_task_root(
        source.path(),
        destination.path(),
        Path::new(""),
        Path::new(""),
        (&[], None),
        &mut remaining,
    )
    .unwrap();

    assert!(!destination.path().join(contract).exists());
    assert!(!destination.path().join(lock).exists());
    assert!(
        !destination
            .path()
            .join(crate::task_set_contract::TASK_SKELETON_FILE)
            .exists()
    );
    assert!(
        !destination
            .path()
            .join(crate::task_set_contract::TASK_SKELETON_LOCK_FILE)
            .exists()
    );
    assert!(!destination.path().join("TASK-EXAMPLE-001.md").exists());
    assert!(!destination.path().join(".decompose.log").exists());
    assert!(std::fs::read(destination.path().join("acceptance-contract.json")).is_err());
    assert_eq!(
        std::fs::read(destination.path().join("input.json")).unwrap(),
        b"input"
    );
}

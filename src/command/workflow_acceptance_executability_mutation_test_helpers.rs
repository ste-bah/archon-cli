use super::*;

/// Candidate data paths `entry` names, relative to its working directory.
pub(crate) fn named_inputs(
    entry: &AcceptanceCriterion,
    project: &Path,
    repository: &Path,
) -> Vec<String> {
    input_discovery(entry, project, repository).eligible
}

#[cfg(unix)]
pub(crate) fn run_with_host_mutation(dir: &Path, script: &str) -> std::process::Output {
    use std::io::Write;
    use std::os::unix::fs::MetadataExt;
    use std::process::Stdio;

    let (nonce, entries, live) = decode_test_plan(script);
    if nonce.is_empty() {
        return archon_shell::spawn::command(archon_shell::resolve_posix_shell())
            .arg("-s")
            .current_dir(dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .and_then(|mut child| {
                child.stdin.take().unwrap().write_all(script.as_bytes())?;
                child.wait_with_output()
            })
            .unwrap();
    }
    let tag = format!("archon-mutated-{}", &nonce[..12]);
    sweep_test_leftovers(dir);
    let canonical_dir = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());
    if live.iter().any(|live| canonical_dir.starts_with(live)) {
        use std::os::unix::process::ExitStatusExt;
        return std::process::Output {
            status: std::process::ExitStatus::from_raw(97 << 8),
            stdout: Vec::new(),
            stderr: format!(
                "archon-mutation-guard-{nonce}: refusing to move inputs in a live root\n"
            )
            .into_bytes(),
        };
    }
    let mut records = Vec::new();
    let mut moved_markers = Vec::new();
    for (index, (path, root)) in entries.iter().enumerate() {
        let root_input = root.clone();
        let root = if root.as_os_str().is_empty() {
            canonical_dir.clone()
        } else {
            root.canonicalize().unwrap_or_else(|_| root.clone())
        };
        let target = absolute_from_selected_root(dir, path, &root_input, root.as_path());
        let aside = PathBuf::from(format!("{}.{}", target.display(), tag));
        let root_identity = std::fs::metadata(&root)
            .ok()
            .map(|meta| (meta.dev(), meta.ino()));
        let state = if !safe_test_path(&root, &target) {
            "refused: symlink component".to_string()
        } else {
            match std::fs::symlink_metadata(&target) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    "absent at the selected root".into()
                }
                Err(error) => format!("move failed: {error}"),
                Ok(_) => match std::fs::rename(&target, &aside) {
                    Ok(()) => {
                        moved_markers.push(index);
                        if root.as_os_str().is_empty() {
                            "moved".into()
                        } else {
                            "moved in the site copy".into()
                        }
                    }
                    Err(error) => format!("move failed: {error}"),
                },
            }
        };
        records.push((index, target, aside, root, root_identity, state, None));
    }
    for record in &mut records {
        if record.5.starts_with("moved") {
            record.6 = Some(test_dir_state(record.1.parent().unwrap()));
        }
    }
    let mut command = archon_shell::spawn::command(archon_shell::resolve_posix_shell());
    let mut child = command
        .arg("-s")
        .current_dir(dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(script.as_bytes())
        .unwrap();
    let mut output = child.wait_with_output().unwrap();
    for (index, target, _, _, _, state, _) in &records {
        if state.starts_with("moved") && std::fs::symlink_metadata(target).is_ok() {
            output.stderr.extend_from_slice(
                format!("archon-mutation-check-recreated-{nonce}: {index}\n").as_bytes(),
            );
        }
    }
    for index in &moved_markers {
        output
            .stderr
            .extend_from_slice(format!("archon-mutation-moved-{nonce}: {index}\n").as_bytes());
    }
    let mut all_safe = true;
    for (index, target, aside, root, identity, move_state, parent_state) in
        records.into_iter().rev()
    {
        output.stderr.extend_from_slice(
            format!("archon-mutation-move-state-{nonce}: {index}:{move_state}\n").as_bytes(),
        );
        if !move_state.starts_with("moved") {
            continue;
        }
        if output.status.code().is_none() {
            all_safe = false;
            output.stdout.extend_from_slice(
                format!("archon-mutation-state-{nonce}: {index}:unsafe-to-restore: teardown did not confirm the process tree gone\n").as_bytes(),
            );
            continue;
        }
        let current_parent_state = test_dir_state(target.parent().unwrap());
        let Some(current_parent_state) = current_parent_state else {
            all_safe = false;
            output.stderr.extend_from_slice(
                format!("archon-mutation-state-{nonce}: {index}:unsafe-to-restore: input parent is unavailable\n").as_bytes(),
            );
            output.stdout.extend_from_slice(
                format!("archon-mutation-state-{nonce}: {index}:unsafe-to-restore: input parent is unavailable\n").as_bytes(),
            );
            continue;
        };
        let parent_changed = parent_state
            .flatten()
            .as_deref()
            .is_some_and(|before| before != current_parent_state);
        let root_still_same = std::fs::metadata(&root)
            .ok()
            .map(|meta| (meta.dev(), meta.ino()))
            == identity;
        if parent_changed {
            output.stderr.extend_from_slice(
                format!("archon-mutation-recreated-{nonce}: {index}\n").as_bytes(),
            );
        }
        let state = if !root_still_same {
            all_safe = false;
            output.stderr.extend_from_slice(
                format!("archon-mutation-state-{nonce}: {index}:unsafe-to-restore: selected root identity changed\n").as_bytes(),
            );
            "unsafe-to-restore".to_string()
        } else if std::fs::symlink_metadata(&target).is_ok() {
            all_safe = false;
            output.stderr.extend_from_slice(
                format!("archon-mutation-generated-{nonce}: {index}\n").as_bytes(),
            );
            output.stderr.extend_from_slice(
                format!("archon-mutation-recreated-{nonce}: {index}\n").as_bytes(),
            );
            "original-recreated".into()
        } else if !safe_test_parent(&root, &target) || !safe_test_parent(&root, &aside) {
            all_safe = false;
            output.stderr.extend_from_slice(
                format!("archon-mutation-state-{nonce}: {index}:unsafe-to-restore: unsafe restore path\n").as_bytes(),
            );
            "unsafe-to-restore".to_string()
        } else {
            match std::fs::rename(&aside, &target) {
                Ok(()) => "restored".into(),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    "aside-missing".into()
                }
                Err(_) => {
                    all_safe = false;
                    output.stderr.extend_from_slice(
                        format!("archon-mutation-state-{nonce}: {index}:unsafe-to-restore: restore move failed\n").as_bytes(),
                    );
                    "unsafe-to-restore".into()
                }
            }
        };
        let state = if parent_changed && state == "restored" {
            all_safe = false;
            "original-recreated".into()
        } else {
            state
        };
        output.stdout.extend_from_slice(
            format!("archon-mutation-state-{nonce}: {index}:{state}\n").as_bytes(),
        );
    }
    if all_safe {
        output
            .stdout
            .extend_from_slice(format!("archon-mutation-restored-{nonce}\n").as_bytes());
    }
    output
}

#[cfg(unix)]
fn decode_test_plan(script: &str) -> (String, Vec<(PathBuf, PathBuf)>, Vec<PathBuf>) {
    let Some(header) = script
        .lines()
        .next()
        .and_then(|line| line.strip_prefix("# archon-test-mutation-v1:"))
    else {
        return (String::new(), Vec::new(), Vec::new());
    };
    let Some((nonce, rest)) = header.split_once(':') else {
        return (String::new(), Vec::new(), Vec::new());
    };
    let Some((path_field, live_field)) = rest.rsplit_once(':') else {
        return (String::new(), Vec::new(), Vec::new());
    };
    let entries = path_field
        .split(',')
        .filter_map(|entry| {
            let (path, root) = entry.split_once(':')?;
            Some((
                PathBuf::from(String::from_utf8(decode_hex(path)?).ok()?),
                PathBuf::from(String::from_utf8(decode_hex(root)?).ok()?),
            ))
        })
        .collect();
    let live = live_field
        .split(',')
        .filter_map(|path| {
            (!path.is_empty())
                .then(|| String::from_utf8(decode_hex(path)?).ok().map(PathBuf::from))
                .flatten()
        })
        .collect();
    (nonce.to_string(), entries, live)
}

#[cfg(unix)]
fn decode_hex(text: &str) -> Option<Vec<u8>> {
    text.as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let digit = |byte| match byte {
                b'0'..=b'9' => Some(byte - b'0'),
                b'a'..=b'f' => Some(byte - b'a' + 10),
                _ => None,
            };
            Some(digit(pair[0])? * 16 + digit(pair[1])?)
        })
        .collect()
}

#[cfg(unix)]
fn absolute_from(dir: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        // Match the canonical selected root while keeping all components
        // below it lexical so safe_test_path can still reject symlinks.
        dir.canonicalize()
            .unwrap_or_else(|_| dir.to_path_buf())
            .join(path)
    }
}

#[cfg(unix)]
fn absolute_from_selected_root(dir: &Path, path: &Path, root_input: &Path, root: &Path) -> PathBuf {
    if root == dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf()) {
        return absolute_from(dir, path);
    }
    if path.is_absolute() {
        if let Ok(relative) = path.strip_prefix(root_input) {
            return root.join(relative);
        }
        return path.to_path_buf();
    }
    root.join(path)
}

#[cfg(unix)]
fn safe_test_path(root: &Path, path: &Path) -> bool {
    if std::fs::symlink_metadata(root).is_ok_and(|meta| meta.file_type().is_symlink())
        || root.canonicalize().as_deref().ok() != Some(root)
    {
        return false;
    }
    let Ok(relative) = path.strip_prefix(root) else {
        return false;
    };
    let mut current = root.to_path_buf();
    for component in relative.components() {
        let std::path::Component::Normal(part) = component else {
            return false;
        };
        current.push(part);
        match std::fs::symlink_metadata(&current) {
            Ok(meta) if meta.file_type().is_symlink() => return false,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return true,
            Err(_) => return false,
            _ => {}
        }
    }
    true
}

#[cfg(unix)]
fn safe_test_parent(root: &Path, path: &Path) -> bool {
    let Ok(relative) = path.strip_prefix(root) else {
        return false;
    };
    let mut current = root.to_path_buf();
    for component in relative
        .components()
        .take(relative.components().count().saturating_sub(1))
    {
        let std::path::Component::Normal(part) = component else {
            return false;
        };
        current.push(part);
        match std::fs::symlink_metadata(&current) {
            Ok(meta) if meta.file_type().is_symlink() || !meta.is_dir() => return false,
            Err(_) => return false,
            _ => {}
        }
    }
    true
}

#[cfg(unix)]
fn test_dir_state(path: &Path) -> Option<String> {
    use std::os::unix::fs::MetadataExt;
    let metadata = std::fs::metadata(path).ok()?;
    Some(format!(
        "{}:{}:{}:{}:{}",
        metadata.mtime(),
        metadata.mtime_nsec(),
        metadata.ino(),
        metadata.ctime(),
        metadata.ctime_nsec()
    ))
}

#[cfg(unix)]
fn sweep_test_leftovers(root: &Path) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            sweep_test_leftovers(&path);
        } else if let Some(name) = path.file_name().and_then(|name| name.to_str())
            && let Some((base, _)) = name.split_once(".archon-mutated-")
        {
            let target = path.with_file_name(base);
            if std::fs::symlink_metadata(&target).is_err() {
                let _ = std::fs::rename(path, target);
            }
        }
    }
}

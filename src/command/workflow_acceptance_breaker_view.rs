//! A disposable view containing only recorded, eligible objects.
use super::*;
use archon_workflow::base_snapshot::{
    BaseSnapshot, SnapshotEntry, SnapshotKind, key_path, outside_key,
};
use std::collections::{BTreeMap, BTreeSet};
use std::io::{BufRead, BufReader, Read, Write};
use std::process::Stdio;
pub(crate) const MAX_VIEW_BYTES: usize = 4 * 1024 * 1024;
pub(crate) struct BaseView {
    root: PathBuf,
    pub(crate) files: BTreeMap<String, ViewFile>,
    pub(crate) skipped: Vec<String>,
    pub(crate) file_keys: BTreeSet<String>,
}
pub(crate) struct ViewFile {
    pub(crate) object: String,
    pub(crate) bytes: Vec<u8>,
}
impl BaseView {
    pub(crate) fn root(&self) -> &Path {
        &self.root
    }
    pub(crate) fn remove(self) -> Result<(), String> {
        owner::remove(&self.root)
    }
}
impl Drop for BaseView {
    fn drop(&mut self) {
        let _ = owner::remove(&self.root);
    }
}
fn error(e: impl std::fmt::Display) -> Unrun {
    Unrun::unknown(e.to_string())
}
pub(crate) fn materialise_view(
    parent: &Path,
    repository: &Path,
    snapshot: &BaseSnapshot,
    keys: &[(String, SnapshotKind)],
    exclude: &[String],
) -> Result<BaseView, Unrun> {
    if !snapshot.valid_digest() {
        return Err(error("invalid_snapshot"));
    }
    std::fs::create_dir_all(parent).map_err(error)?;
    let root = parent.join(format!("{COPY_PREFIX}view-{}", uuid::Uuid::new_v4()));
    owner::write(&root)?;
    let mut view = BaseView {
        root,
        files: BTreeMap::new(),
        skipped: vec![],
        file_keys: BTreeSet::new(),
    };
    // Private configuration is beside the readable view, never inside it.
    let config = tempfile::tempdir_in(parent).map_err(error)?;
    let isolated = base_git::IsolatedGit::new(config.path())?;
    let (mut objects, skipped) = object_map(&isolated, repository, snapshot, keys)?;
    view.skipped = skipped;
    view.file_keys = objects.keys().cloned().collect();
    objects.retain(|key, _| {
        !exclude
            .iter()
            .any(|e| key == e || key.strip_prefix(e).is_some_and(|s| s.starts_with('/')))
    });
    let mut child = isolated
        .command()
        .arg("-C")
        .arg(repository)
        .args(["cat-file", "--batch"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(error)?;
    let result = (|| {
        let mut input = child
            .stdin
            .take()
            .ok_or_else(|| error("object_input_missing"))?;
        let mut output = BufReader::new(
            child
                .stdout
                .take()
                .ok_or_else(|| error("object_output_missing"))?,
        );
        let mut total = 0usize;
        for (key, object) in objects {
            writeln!(input, "{object}").map_err(error)?;
            input.flush().map_err(error)?;
            let mut header = String::new();
            output.read_line(&mut header).map_err(error)?;
            let fields: Vec<_> = header.split_whitespace().collect();
            if fields.len() != 3 || fields[0] != object || fields[1] != "blob" {
                return Err(error("invalid_object_reply"));
            }
            let size: usize = fields[2].parse().map_err(error)?;
            total = total
                .checked_add(size)
                .ok_or_else(|| error("view_too_large"))?;
            if total > MAX_VIEW_BYTES {
                return Err(error("view_too_large"));
            }
            let mut bytes = vec![0; size];
            output.read_exact(&mut bytes).map_err(error)?;
            let mut end = [0];
            output.read_exact(&mut end).map_err(error)?;
            if end != [b'\n'] {
                return Err(error("invalid_object_reply"));
            }
            view.files.insert(key, ViewFile { object, bytes });
        }
        drop(input);
        Ok(())
    })();
    if result.is_err() {
        let _ = child.kill();
    }
    let status = child.wait().map_err(error)?;
    result?;
    if !status.success() {
        return Err(error("object_reader_failed"));
    }
    // All bytes and the cap are verified before writing anything to the view.
    std::fs::create_dir(&view.root).map_err(error)?;
    for (key, file) in &view.files {
        let path = view.root.join(key_path(key));
        std::fs::create_dir_all(path.parent().ok_or_else(|| error("view_parent_missing"))?)
            .map_err(error)?;
        std::fs::write(path, &file.bytes).map_err(error)?;
    }
    Ok(view)
}

fn object_map(
    isolated: &base_git::IsolatedGit,
    repository: &Path,
    snapshot: &BaseSnapshot,
    keys: &[(String, SnapshotKind)],
) -> Result<(BTreeMap<String, String>, Vec<String>), Unrun> {
    if !snapshot.valid_digest() {
        return Err(error("invalid_snapshot"));
    }
    let mut skipped = Vec::new();
    let mut objects = BTreeMap::new();
    for (key, kind) in keys {
        if outside_key(key) {
            return Err(error("invalid_view_key"));
        }
        let Some(SnapshotEntry::Tracked {
            object,
            kind: recorded,
            ..
        }) = snapshot.entries.get(key)
        else {
            return Err(error("view_object_missing"));
        };
        if kind != recorded {
            return Err(error("view_kind_mismatch"));
        }
        if object.is_empty() || !object.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(error("invalid_view_object"));
        }
        match kind {
            SnapshotKind::Blob => {
                objects.insert(key.clone(), object.clone());
            }
            SnapshotKind::Symlink => skipped.push(key.clone()),
            SnapshotKind::Tree => {
                let result = isolated.output(&[
                    "-C".as_ref(),
                    repository.as_os_str(),
                    "ls-tree".as_ref(),
                    "-r".as_ref(),
                    "-z".as_ref(),
                    "--full-tree".as_ref(),
                    object.as_ref(),
                ])?;
                if !result.status.success() {
                    return Err(error(String::from_utf8_lossy(&result.stderr)));
                }
                for row in result.stdout.split(|b| *b == 0).filter(|r| !r.is_empty()) {
                    let row = std::str::from_utf8(row).map_err(error)?;
                    let (header, path) = row
                        .split_once('\t')
                        .ok_or_else(|| error("invalid_object_entry"))?;
                    let fields: Vec<_> = header.split_whitespace().collect();
                    let path = format!("{key}/{path}");
                    if outside_key(&path) {
                        return Err(error("invalid_view_key"));
                    }
                    if fields.len() != 3 {
                        return Err(error("invalid_object_entry"));
                    }
                    if fields[1] == "blob" && fields[0] != "120000" {
                        objects.insert(path, fields[2].into());
                    } else {
                        skipped.push(path);
                    }
                }
            }
        }
    }
    Ok((objects, skipped))
}
pub(crate) fn view_file_keys(
    parent: &Path,
    repository: &Path,
    snapshot: &BaseSnapshot,
    keys: &[(String, SnapshotKind)],
) -> Result<BTreeSet<String>, Unrun> {
    std::fs::create_dir_all(parent).map_err(error)?;
    let config = tempfile::tempdir_in(parent).map_err(error)?;
    let isolated = base_git::IsolatedGit::new(config.path())?;
    Ok(object_map(&isolated, repository, snapshot, keys)?
        .0
        .into_keys()
        .collect())
}

//! Deliverable evidence anchored to one recorded tree.
use crate::repository_audit::ignored::ignored_among_checked;
use crate::repository_record::{RepositoryTree, normalize_relative};
use crate::task_set_contract::content_digest;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BaseSnapshot {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub landing_verified: BTreeMap<String, String>,
    pub commit: String,
    pub entries: BTreeMap<String, SnapshotEntry>,
    pub digest: String,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SnapshotKind {
    Blob,
    Tree,
    Symlink,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LiveState {
    Same,
    Modified,
    Missing,
    Unknown,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnverifiedReason {
    Ignored,
    Untracked,
    Submodule,
    OutsideRepository,
    CaseMismatch,
    IgnoreStatusUnknown,
    AddedAfterBase,
    Error,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum SnapshotEntry {
    Tracked {
        kind: SnapshotKind,
        object: String,
        live: LiveState,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        live_note: Option<String>,
    },
    Unverified {
        reason: UnverifiedReason,
        live_digest: Option<String>,
        live_note: Option<String>,
    },
    Absent,
}
impl BaseSnapshot {
    pub fn recomputed_digest(&self) -> String {
        let bytes = if self.landing_verified.is_empty() {
            serde_json::to_vec(&(&self.commit, &self.entries))
        } else {
            serde_json::to_vec(&(&self.commit, &self.entries, &self.landing_verified))
        };
        content_digest(&bytes.expect("serializable snapshot"))
    }
    pub fn valid_digest(&self) -> bool {
        self.digest == self.recomputed_digest()
    }
}
pub fn snapshot_key(tree: &RepositoryTree, path: &str) -> String {
    snapshot_path_key(tree.root(), path)
}
pub fn snapshot_path_key(root: &Path, path: &str) -> String {
    let candidate = PathBuf::from(path.replace('\\', "/"));
    if candidate.has_root() || candidate.is_absolute() || path.as_bytes().get(1) == Some(&b':') {
        return candidate
            .strip_prefix(root)
            .map(|p| normalize_relative(&p.to_string_lossy()))
            .unwrap_or_else(|_| path.into());
    }
    normalize_relative(path)
}
pub fn outside_key(key: &str) -> bool {
    let key = key.replace('\\', "/");
    key.is_empty()
        || key.split('/').any(|p| p == "..")
        || Path::new(&key).has_root()
        || key.as_bytes().get(1) == Some(&b':')
}
pub fn skip_copy_entry(key: &str, entry: &SnapshotEntry) -> bool {
    outside_key(key)
        || matches!(
            entry,
            SnapshotEntry::Unverified {
                reason: UnverifiedReason::OutsideRepository | UnverifiedReason::CaseMismatch,
                ..
            }
        )
}
pub fn key_path(key: &str) -> PathBuf {
    let mut path = PathBuf::new();
    for part in key.split('/').filter(|part| !part.is_empty()) {
        path.push(part);
    }
    path
}
fn query(root: &Path, args: &[&str]) -> Result<Vec<u8>, String> {
    let output = archon_shell::spawn::command("git")
        .arg("-C")
        .arg(root)
        .arg("--literal-pathspecs")
        .args(args)
        .output()
        .map_err(|e| e.to_string())?;
    if !output.status.success() {
        let cause = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(if cause.is_empty() {
            format!("{} exited with {}", args.join(" "), output.status)
        } else {
            cause
        });
    }
    Ok(output.stdout)
}
fn names(bytes: &[u8]) -> BTreeSet<String> {
    bytes
        .split(|b| *b == 0)
        .filter(|p| !p.is_empty())
        .map(|p| String::from_utf8_lossy(p).into_owned())
        .collect()
}
fn live_ancestor(root: &Path, key: &str) -> bool {
    let path = key_path(key);
    path.ancestors()
        .skip(1)
        .filter(|p| !p.as_os_str().is_empty())
        .any(|p| {
            root.join(p)
                .symlink_metadata()
                .is_ok_and(|m| m.file_type().is_symlink())
        })
}
fn unverified(
    reason: UnverifiedReason,
    path: Option<&Path>,
    note: Option<String>,
) -> SnapshotEntry {
    let (digest, digest_note) = path.map(live_digest).unwrap_or((None, None));
    SnapshotEntry::Unverified {
        reason,
        live_digest: digest,
        live_note: note.or(digest_note),
    }
}
/// Hash live evidence without following links, with a total one-GiB read bound.
pub fn live_digest(path: &Path) -> (Option<String>, Option<String>) {
    match digest_live(path) {
        Ok(digest) => (Some(digest), None),
        Err(error) => (None, Some(error)),
    }
}
#[path = "base_snapshot_digest.rs"]
mod digest;
use digest::digest_live;

pub fn snapshot_deliverables<I, S>(
    tree: &RepositoryTree,
    project_root: &Path,
    paths: I,
) -> Result<BaseSnapshot, String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let root = tree.root();
    let empty = tree.base_commit().trim().is_empty()
        || tree.base_commit() == crate::repository_record::UNBORN_BASE_COMMIT;
    if !empty {
        query(
            root,
            &[
                "cat-file",
                "-e",
                &format!("{}^{{commit}}", tree.base_commit()),
            ],
        )?;
    }
    let changes = query(
        root,
        &[
            "diff",
            "--name-only",
            "-z",
            "--no-ext-diff",
            "--no-renames",
            "--relative",
            if empty { "HEAD" } else { tree.base_commit() },
        ],
    );
    let staged = query(root, &["ls-files", "-z", "--stage"]);
    let indexed = staged
        .as_ref()
        .map(|b| {
            b.split(|v| *v == 0)
                .filter_map(|line| {
                    let text = String::from_utf8_lossy(line);
                    text.split_once('\t').map(|(_, path)| path.to_string())
                })
                .collect::<BTreeSet<_>>()
        })
        .map_err(|error| (*error).clone());
    let gitlinks = staged.as_ref().map(|b| {
        b.split(|v| *v == 0)
            .filter_map(|line| {
                let text = String::from_utf8_lossy(line);
                text.split_once('\t')
                    .filter(|(header, _)| header.starts_with("160000 "))
                    .map(|(_, path)| path.to_string())
            })
            .collect::<BTreeSet<_>>()
    });
    let headed =
        query(root, &["ls-tree", "-r", "-t", "-z", "--name-only", "HEAD"]).map(|b| names(&b));
    let changed = changes.as_ref().map(|b| names(b));
    let mut entries = BTreeMap::new();
    for path in paths {
        let raw = path.as_ref();
        let key = snapshot_key(tree, raw);
        if entries.contains_key(&key) {
            continue;
        }
        let invalid = outside_key(&key) || tree.relative_to_root(raw).is_none();
        let ancestor = tree
            .objects_at_base()
            .iter()
            .find(|(p, object)| key.starts_with(&format!("{p}/")) && object.kind != "tree");
        let entry = if invalid {
            unverified(UnverifiedReason::OutsideRepository, None, None)
        } else if let Some((_, object)) = ancestor {
            unverified(
                if object.mode == "160000" {
                    UnverifiedReason::Submodule
                } else {
                    UnverifiedReason::OutsideRepository
                },
                None,
                None,
            )
        } else if let Some(object) = tree.objects_at_base().get(&key) {
            if object.mode == "160000" {
                unverified(UnverifiedReason::Submodule, None, None)
            } else {
                let kind = match object.mode.as_str() {
                    "100644" | "100755" => Some(SnapshotKind::Blob),
                    "120000" => Some(SnapshotKind::Symlink),
                    "040000" => Some(SnapshotKind::Tree),
                    _ => None,
                };
                match kind {
                    None => unverified(
                        UnverifiedReason::Error,
                        None,
                        Some(format!("unsupported base mode {}", object.mode)),
                    ),
                    Some(kind) => {
                        let others = if kind == SnapshotKind::Tree {
                            query(
                                root,
                                &[
                                    "ls-files",
                                    "-z",
                                    "--others",
                                    "--exclude-standard",
                                    "--directory",
                                    "--",
                                    &key,
                                ],
                            )
                        } else {
                            Ok(Vec::new())
                        };
                        let path = root.join(key_path(&key));
                        let metadata = path.symlink_metadata();
                        let metadata_error = metadata
                            .as_ref()
                            .err()
                            .filter(|error| {
                                !matches!(
                                    error.kind(),
                                    std::io::ErrorKind::NotFound
                                        | std::io::ErrorKind::NotADirectory
                                )
                            })
                            .map(|error| format!("{}: {error}", path.display()));
                        let error = changes
                            .as_ref()
                            .err()
                            .or(others.as_ref().err())
                            .cloned()
                            .or(metadata_error);
                        let live = if error.is_some() {
                            LiveState::Unknown
                        } else if metadata.is_err() {
                            LiveState::Missing
                        } else if changed.as_ref().is_ok_and(|set| {
                            set.iter()
                                .any(|p| p == &key || p.starts_with(&format!("{key}/")))
                        }) || others.is_ok_and(|b| !b.is_empty())
                        {
                            LiveState::Modified
                        } else {
                            LiveState::Same
                        };
                        SnapshotEntry::Tracked {
                            kind,
                            object: object.object.clone(),
                            live,
                            live_note: error,
                        }
                    }
                }
            }
        } else if gitlinks.as_ref().is_ok_and(|paths| {
            paths
                .iter()
                .any(|path| path == &key || key.starts_with(&format!("{path}/")))
        }) {
            unverified(UnverifiedReason::Submodule, None, None)
        } else if live_ancestor(root, &key) || live_ancestor(project_root, &key) {
            unverified(
                UnverifiedReason::OutsideRepository,
                None,
                Some("live symbolic-link ancestor".into()),
            )
        } else {
            let repository_path = root.join(key_path(&key));
            let project_path = project_root.join(key_path(&key));
            let mut live = None;
            let mut presence_error = None;
            for path in [&repository_path, &project_path] {
                match path.symlink_metadata() {
                    Ok(_) => {
                        live = Some(path);
                        break;
                    }
                    Err(error)
                        if matches!(
                            error.kind(),
                            std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
                        ) => {}
                    Err(error) => {
                        presence_error = Some(format!("{}: {error}", path.display()));
                        break;
                    }
                }
            }
            if let Some(error) = presence_error {
                unverified(UnverifiedReason::Error, None, Some(error))
            } else if tree
                .paths_at_base()
                .iter()
                .any(|p| p.eq_ignore_ascii_case(&key))
            {
                unverified(
                    UnverifiedReason::CaseMismatch,
                    live.map(PathBuf::as_path),
                    None,
                )
            } else if let Some(path) = live {
                let Ok(relative) = path.strip_prefix(root) else {
                    entries.insert(
                        key,
                        unverified(UnverifiedReason::OutsideRepository, Some(path), None),
                    );
                    continue;
                };
                let live_key = normalize_relative(&relative.to_string_lossy());
                match ignored_among_checked(root, std::slice::from_ref(&live_key)) {
                    Err(error) => unverified(
                        UnverifiedReason::IgnoreStatusUnknown,
                        Some(path),
                        Some(error),
                    ),
                    Ok(ignored) if ignored.contains(&live_key) => {
                        unverified(UnverifiedReason::Ignored, Some(path), None)
                    }
                    Ok(_) if empty => unverified(UnverifiedReason::Untracked, Some(path), None),
                    Ok(_) => match indexed.as_ref().and_then(|index| {
                        headed.as_ref().map(|head| {
                            index
                                .iter()
                                .chain(head.iter())
                                .any(|p| p == &live_key || p.starts_with(&format!("{live_key}/")))
                        })
                    }) {
                        Ok(true) => unverified(UnverifiedReason::AddedAfterBase, Some(path), None),
                        Ok(false) => unverified(UnverifiedReason::Untracked, Some(path), None),
                        Err(error) => {
                            unverified(UnverifiedReason::Error, Some(path), Some(error.clone()))
                        }
                    },
                }
            } else {
                SnapshotEntry::Absent
            }
        };
        entries.insert(key, entry);
    }
    let mut snapshot = BaseSnapshot {
        landing_verified: Default::default(),
        commit: tree.base_commit().into(),
        entries,
        digest: String::new(),
    };
    snapshot.digest = snapshot.recomputed_digest();
    Ok(snapshot)
}

#[path = "base_snapshot_excludes.rs"]
mod excludes;
pub use excludes::snapshot_excludes;
pub fn base_snapshot_excludes(
    policy: &crate::acceptance_scratch::ScratchPolicy,
    snapshot: &BaseSnapshot,
) -> Vec<PathBuf> {
    snapshot_excludes(
        &policy.repository,
        &policy.project,
        policy.combined,
        snapshot,
    )
}

impl BaseSnapshot {
    pub fn landing_bytes(&self, repository: &Path, key: &str) -> Result<Vec<u8>, String> {
        if outside_key(key) {
            return Err("landing_key_outside".into());
        }
        let expected = self
            .landing_verified
            .get(key)
            .ok_or("landing_digest_missing")?;
        let path = repository.join(key_path(key));
        if !path
            .symlink_metadata()
            .is_ok_and(|m| m.is_file() && !m.file_type().is_symlink())
        {
            return Err("landing_not_file".into());
        }
        let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
        if content_digest(&bytes) != *expected {
            return Err("control_mismatch".into());
        }
        Ok(bytes)
    }
}

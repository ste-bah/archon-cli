//! Lexical ownership of roots that connect other tasks' artifacts.
use archon_workflow::base_snapshot::{
    BaseSnapshot, SnapshotEntry, SnapshotKind, snapshot_path_key,
};
use archon_workflow::v2::requirement_classes::RequirementEntry;
use std::path::Path;
fn word(text: &str, needle: &str) -> bool {
    text.match_indices(needle).any(|(at, _)| {
        let boundary = |c: char| !c.is_ascii_alphanumeric() && c != '_';
        text[..at].chars().next_back().is_none_or(boundary)
            && text[at + needle.len()..]
                .chars()
                .next()
                .is_none_or(boundary)
    })
}
pub(super) fn mark(entries: &mut [RequirementEntry], snapshot: &BaseSnapshot, repository: &Path) {
    let all: Vec<_> = entries
        .iter()
        .flat_map(|e| {
            e.deliverables.iter().map(|d| {
                (
                    e.task_id.clone(),
                    snapshot_path_key(repository, &d.artifact_path),
                )
            })
        })
        .collect();
    for entry in entries {
        for d in &mut entry.deliverables {
            let key = snapshot_path_key(repository, &d.artifact_path);
            let Some(SnapshotEntry::Tracked {
                kind: SnapshotKind::Blob,
                object,
                ..
            }) = snapshot.entries.get(&key)
            else {
                continue;
            };
            let Ok(text) = base_blob(repository, object) else {
                d.error.get_or_insert_with(|| "wiring_root_unread".into());
                continue;
            };
            if text.len() > 1024 * 1024 {
                d.error.get_or_insert_with(|| "wiring_root_unread".into());
                continue;
            }
            let file = Path::new(&key);
            let dir = file.parent().unwrap_or_else(|| Path::new(""));
            let Some(stem) = file
                .file_stem()
                .and_then(|s| s.to_str())
                .filter(|s| s.len() >= 2)
            else {
                continue;
            };
            let connects = all
                .iter()
                .filter(|(t, _)| t != &entry.task_id)
                .filter_map(|(_, path)| Path::new(path).strip_prefix(dir).ok())
                .filter(|p| p.components().count() > 0)
                .any(|p| {
                    p.file_stem()
                        .and_then(|s| s.to_str())
                        .into_iter()
                        .chain(
                            p.parent()
                                .into_iter()
                                .flat_map(|p| p.components())
                                .filter_map(|c| c.as_os_str().to_str()),
                        )
                        .any(|s| s.len() >= 2 && word(&text, s))
                });
            if !connects {
                continue;
            }
            let output = base_names(repository, &snapshot.commit, dir, stem);
            if output.is_ok_and(|names| {
                names
                    .lines()
                    .all(|p| p.split_once(':').map_or(p, |(_, p)| p) == key)
            }) {
                d.shared = true;
                d.wiring_root = true;
            }
        }
    }
}

fn base_blob(repository: &Path, object: &str) -> Result<String, String> {
    let size = crate::command::workflow_task_set::executability::hermetic::base_copy::output(
        repository,
        &["cat-file", "-s", object],
    )
    .map_err(|e| format!("{e:?}"))?;
    if size.trim().parse::<usize>().map_err(|e| e.to_string())? > 1024 * 1024 {
        return Err("wiring_root_unread".into());
    }
    crate::command::workflow_task_set::executability::hermetic::base_copy::output(
        repository,
        &["cat-file", "blob", object],
    )
    .map_err(|e| format!("{e:?}"))
}
fn base_names(repository: &Path, commit: &str, dir: &Path, stem: &str) -> Result<String, String> {
    match crate::command::workflow_task_set::executability::hermetic::base_copy::query(
        repository,
        &[
            "grep",
            "-l",
            "-w",
            "-F",
            "-e",
            stem,
            commit,
            "--",
            dir.to_str().filter(|s| !s.is_empty()).unwrap_or("."),
        ],
    ) {
        Ok(s) if s.status.success() => Ok(String::from_utf8_lossy(&s.stdout).into_owned()),
        Ok(s) if s.status.code() == Some(1) => Ok(String::new()),
        Ok(s) => Err(String::from_utf8_lossy(&s.stderr).into_owned()),
        Err(e) => Err(format!("{e:?}")),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn git(root: &Path, args: &[&str]) -> String {
        let out = archon_shell::spawn::command("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap().trim().into()
    }
    fn classify(
        root_text: &str,
        other: &str,
        peer: Option<&str>,
        same_task: bool,
        root_name: &str,
    ) -> RequirementEntry {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        git(root, &["init", "-q"]);
        git(root, &["config", "user.name", "test"]);
        git(root, &["config", "user.email", "test@example.invalid"]);
        for (path, text) in [(root_name, root_text), (other, "content")] {
            let p = root.join(path);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, text).unwrap();
        }
        if let Some(text) = peer {
            std::fs::write(root.join("tree/peer"), text).unwrap();
        }
        git(root, &["add", "."]);
        git(root, &["commit", "-qm", "base"]);
        let commit = git(root, &["rev-parse", "HEAD"]);
        let tree = archon_workflow::repository_record::RepositoryTree::load(
            &archon_workflow::repository_record::RepositoryRecordV1 {
                schema_version: 1,
                repository_root: root.display().to_string(),
                base_commit: commit,
                decomposition_run_id: "test".into(),
                recorded_at: "test".into(),
            },
        )
        .unwrap();
        let snapshot =
            archon_workflow::base_snapshot::snapshot_deliverables(&tree, root, [root_name, other])
                .unwrap();
        let entry = |task: &str, path: &str| {
            serde_json::from_value::<RequirementEntry>(serde_json::json!({"task_id":task,"id":"id","class":"candidate_done","reason":null,"fallback":"undecided","checks":{},"deliverables":[{"artifact_path":path,"at_base":true,"live_only":false,"shared":false,"text":false}],"text_item":false,"passing_check_without_deliverable":false})).unwrap()
        };
        let mut entries = vec![
            entry("owner", root_name),
            entry(if same_task { "owner" } else { "other" }, other),
        ];
        mark(&mut entries, &snapshot, root);
        entries.remove(0)
    }
    #[test]
    fn wiring_roots_use_words_and_other_task_ownership() {
        assert!(
            classify("leaf", "tree/leaf", None, false, "tree/root").deliverables[0].wiring_root
        );
        assert!(
            !classify("leaf", "tree/leaf", Some("root"), false, "tree/root").deliverables[0]
                .wiring_root
        );
        assert!(
            !classify("leaf", "tree/leaf", None, true, "tree/root").deliverables[0].wiring_root
        );
        assert!(
            !classify("leaf_extra", "tree/leaf", None, false, "tree/root").deliverables[0]
                .wiring_root
        );
        assert!(!classify("leaf", "tree/leaf", None, false, "tree/x").deliverables[0].wiring_root);
        assert!(
            classify("branch", "tree/branch/file", None, false, "tree/root").deliverables[0]
                .wiring_root
        );
    }
    #[test]
    fn oversized_wiring_root_is_reported_unread() {
        let entry = classify(
            &"leaf ".repeat(220000),
            "tree/leaf",
            None,
            false,
            "tree/root",
        );
        assert_eq!(
            entry.deliverables[0].error.as_deref(),
            Some("wiring_root_unread")
        );
        assert!(!entry.deliverables[0].wiring_root);
    }
}

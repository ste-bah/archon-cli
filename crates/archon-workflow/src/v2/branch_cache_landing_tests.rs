//! Issue-108: a landing stands on the tree the run's own later landings
//! left, and on nothing else.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use super::*;
use crate::v2::host_api::{WorkflowV2HostCall, WorkflowV2HostMethod, WorkflowV2HostOptions};
use crate::v2::result::WorkflowV2Result;
use crate::v2::result_store::WorkflowV2CallRecord;

const RUN: &str = "run-1";
const FILE: &str = "src/lib.rs";

struct Run {
    _temp: tempfile::TempDir,
    run_root: PathBuf,
    repo: PathBuf,
    store: WorkflowV2ResultStore,
}

fn run() -> Run {
    let temp = tempfile::tempdir().unwrap();
    let run_root = temp.path().join("run");
    let repo = temp.path().join("repo");
    std::fs::create_dir_all(repo.join("src")).unwrap();
    let store = WorkflowV2ResultStore::new(run_root.join("v2"));
    Run {
        _temp: temp,
        run_root,
        repo,
        store,
    }
}

fn hash(text: &str) -> String {
    blake3::hash(text.as_bytes()).to_hex().to_string()
}

impl Run {
    fn write(&self, text: Option<&str>) {
        let path = self.repo.join(FILE);
        match text {
            Some(text) => std::fs::write(path, text).unwrap(),
            None => {
                let _ = std::fs::remove_file(path);
            }
        }
    }

    /// A landed manifest of `stage` taking FILE from `pre` to `post` (None
    /// is no file), its call record started at `second`.
    fn land(
        &self,
        stage: &str,
        pre: Option<&str>,
        post: Option<&str>,
        second: Option<u32>,
    ) -> PatchManifest {
        self.land_as(RUN, stage, pre, post, second)
    }

    fn land_as(
        &self,
        run_id: &str,
        stage: &str,
        pre: Option<&str>,
        post: Option<&str>,
        second: Option<u32>,
    ) -> PatchManifest {
        let state = |text: Option<&str>, none: &str| text.map_or(none.to_string(), hash);
        let manifest = PatchManifest {
            schema: "test".into(),
            run_id: run_id.into(),
            stage_id: stage.into(),
            item_id: format!("{stage}-0"),
            baseline_commit: "base".into(),
            patch_path: PathBuf::from("p.patch"),
            declared_target_files: vec![FILE.into()],
            changed_files: if pre.is_some() && post.is_some() {
                vec![FILE.into()]
            } else {
                vec![]
            },
            created_files: if pre.is_none() {
                vec![FILE.into()]
            } else {
                vec![]
            },
            deleted_files: if post.is_none() {
                vec![FILE.into()]
            } else {
                vec![]
            },
            pre_hashes: BTreeMap::from([(FILE.to_string(), state(pre, "absent"))]),
            post_hashes: BTreeMap::from([(FILE.to_string(), state(post, "deleted"))]),
            verify_command: None,
            agent_artifact_path: None,
            status: ManifestStatus::Applied,
            skipped_ignored: BTreeMap::new(),
        };
        let dir = self
            .run_root
            .join(format!("write-coordination/stages/{stage}/manifests"));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join(format!("{stage}-0.json")),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        if let Some(second) = second {
            let call = WorkflowV2HostCall {
                id: stage.into(),
                method: WorkflowV2HostMethod::Fanout,
                write_mode: None,
                options: WorkflowV2HostOptions::default(),
            };
            let mut record = WorkflowV2CallRecord::new(
                RUN,
                call,
                1,
                "h".into(),
                WorkflowV2Result::accepted("landed"),
                vec![],
            );
            record.started_at = format!("2026-09-26T01:00:{second:02}.5+00:00");
            self.store.save_call_record(&record).unwrap();
        }
        manifest
    }

    fn holds(&self, manifest: &PatchManifest) -> Result<(), String> {
        landing_holds(&self.store, Path::new(&self.repo), manifest)
    }
}

#[test]
fn an_untouched_landing_stands() {
    let r = run();
    let one = r.land("fix-1", Some("x"), Some("a"), Some(1));
    r.write(Some("a"));
    assert_eq!(r.holds(&one), Ok(()));
}

#[test]
fn a_later_landing_of_the_run_over_the_same_file_leaves_both_standing() {
    let r = run();
    let one = r.land("fix-1", Some("x"), Some("a"), Some(1));
    let two = r.land("fix-2", Some("a"), Some("b"), Some(2));
    let three = r.land("fix-3", Some("b"), Some("c"), Some(3));
    r.write(Some("c"));
    for landing in [&one, &two, &three] {
        assert_eq!(r.holds(landing), Ok(()), "{}", landing.stage_id);
    }
}

#[test]
fn a_change_no_landing_of_the_run_left_refuses() {
    let r = run();
    let one = r.land("fix-1", Some("x"), Some("a"), Some(1));
    let two = r.land("fix-2", Some("a"), Some("b"), Some(2));
    r.write(Some("edited by hand"));
    assert!(r.holds(&one).is_err());
    assert!(r.holds(&two).is_err());
    r.write(None);
    assert!(r.holds(&two).is_err(), "an outside deletion refuses too");
}

#[test]
fn an_order_the_host_records_do_not_prove_refuses() {
    // Something outside the run changed the file between two landings.
    let r = run();
    let one = r.land("fix-1", Some("x"), Some("a"), Some(1));
    let two = r.land("fix-2", Some("edited by hand"), Some("b"), Some(2));
    r.write(Some("b"));
    assert!(
        r.holds(&one)
            .unwrap_err()
            .contains("no landing of this run left")
    );
    assert!(r.holds(&two).is_err());
    // Two landings both claim to follow it.
    let r = run();
    let one = r.land("fix-1", Some("x"), Some("a"), Some(1));
    r.land("fix-2", Some("a"), Some("b"), Some(2));
    r.land("fix-3", Some("a"), Some("c"), Some(3));
    r.write(Some("b"));
    assert!(r.holds(&one).unwrap_err().contains("not proven"));
}

/// Order is read from the manifests' contents, never from when a call
/// record says a call started: a resume rewrites a replayed call's record.
#[test]
fn a_chain_needs_no_call_record_and_ignores_record_times() {
    let r = run();
    let one = r.land("fix-1", Some("x"), Some("a"), Some(9));
    let two = r.land("fix-2", Some("a"), Some("b"), None);
    r.write(Some("b"));
    assert_eq!(r.holds(&one), Ok(()));
    assert_eq!(r.holds(&two), Ok(()));
}

#[test]
fn another_runs_landing_and_an_unrecorded_pre_state_prove_nothing() {
    let r = run();
    let one = r.land("fix-1", Some("x"), Some("a"), Some(1));
    r.land_as("other-run", "fix-2", Some("a"), Some("b"), Some(2));
    r.write(Some("b"));
    assert!(r.holds(&one).is_err());
    let r = run();
    let one = r.land("fix-1", Some("x"), Some("a"), Some(1));
    let mut two = r.land("fix-2", Some("a"), Some("b"), Some(2));
    two.pre_hashes.clear();
    let dir = r.run_root.join("write-coordination/stages/fix-2/manifests");
    std::fs::write(dir.join("fix-2-0.json"), serde_json::to_vec(&two).unwrap()).unwrap();
    r.write(Some("b"));
    assert!(
        r.holds(&one).is_err(),
        "a writer with no pre-hash cannot be ordered"
    );
}

#[test]
fn a_deletion_then_a_recreation_by_the_run_stands() {
    let r = run();
    let gone = r.land("fix-1", Some("x"), None, Some(1));
    let back = r.land("fix-2", None, Some("new"), Some(2));
    r.write(Some("new"));
    assert_eq!(r.holds(&gone), Ok(()));
    assert_eq!(r.holds(&back), Ok(()));
    r.write(None);
    assert!(r.holds(&back).is_err());
    assert!(
        r.holds(&gone).is_err(),
        "the run's last landing there recreated it"
    );
}

/// The run's own later landing decides, even when the tree was reverted by
/// hand to an earlier landing's state.
#[test]
fn a_revert_to_an_earlier_landing_after_a_later_one_refuses() {
    let r = run();
    let one = r.land("fix-1", Some("x"), Some("a"), Some(1));
    let two = r.land("fix-2", Some("a"), Some("b"), Some(2));
    r.write(Some("a"));
    assert!(r.holds(&one).is_err());
    assert!(r.holds(&two).is_err());
}

//! Issue-113 under Issue-108: a materialized project artifact stands on the
//! state the run's LAST copy there left, ordered by the host's own receipts.
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use super::*;
use crate::write_coordinator::{ManifestStatus, MaterializedDeliverable};

fn hash(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

struct Run {
    _dir: tempfile::TempDir,
    root: PathBuf,
    destination: PathBuf,
}

fn run() -> Run {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("project/.archon/workflows/run1");
    std::fs::create_dir_all(&root).unwrap();
    let destination = dir.path().join("project/.archon/lab/out.pine");
    std::fs::create_dir_all(destination.parent().unwrap()).unwrap();
    Run {
        _dir: dir,
        root,
        destination,
    }
}

/// A landed manifest of `stage` that copied `bytes` to the destination as
/// the run's `sequence`-th copy, persisted where the run keeps receipts and
/// -- as a decided landing does -- appended to the run's ledger.
fn landed(run: &Run, stage: &str, bytes: &[u8], sequence: u64) -> PatchManifest {
    let manifest = claimed(run, stage, bytes, sequence);
    let (path, receipt) = manifest.materialized.iter().next().unwrap();
    crate::write_coordinator::patch_apply::append_materializations(
        &run.root,
        stage,
        &format!("{stage}-0"),
        &[(path.clone(), receipt.clone())],
    )
    .unwrap();
    manifest
}

/// `landed`, without the ledger line: a manifest that only CLAIMS the copy.
fn claimed(run: &Run, stage: &str, bytes: &[u8], sequence: u64) -> PatchManifest {
    let manifest = PatchManifest {
        schema: crate::write_coordinator::patch_manifest::PATCH_MANIFEST_SCHEMA.into(),
        run_id: "run1".into(),
        stage_id: stage.into(),
        item_id: format!("{stage}-0"),
        baseline_commit: "base".into(),
        patch_path: run.root.join("unused.patch"),
        declared_target_files: vec![".archon/lab/out.pine".into()],
        changed_files: vec![],
        created_files: vec![],
        deleted_files: vec![],
        pre_hashes: BTreeMap::new(),
        post_hashes: BTreeMap::new(),
        verify_command: None,
        agent_artifact_path: None,
        status: ManifestStatus::SkippedIgnored,
        skipped_ignored: BTreeMap::new(),
        materialized: BTreeMap::from([(
            ".archon/lab/out.pine".to_string(),
            MaterializedDeliverable {
                destination: run.destination.display().to_string(),
                pre_hash: "absent".into(),
                post_hash: hash(bytes),
                sequence,
            },
        )]),
        materializable: Default::default(),
        destination_baselines: Default::default(),
        needs_attention: None,
    };
    let path = Path::new(&crate::v2::write::manifest_path_for(
        &run.root,
        stage,
        &format!("{stage}-0"),
    ))
    .to_path_buf();
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, serde_json::to_vec(&manifest).unwrap()).unwrap();
    manifest
}

#[test]
fn a_copy_that_still_holds_stands_and_an_edit_to_it_refuses() {
    let run = run();
    let round = landed(&run, "fix-1", b"regenerated", 1);
    std::fs::write(&run.destination, "regenerated").unwrap();
    assert_eq!(materialized_holds(&run.root, &round), Ok(()));

    std::fs::write(&run.destination, "edited outside the run").unwrap();
    let refused = materialized_holds(&run.root, &round).unwrap_err();
    assert!(refused.contains("fix-1/fix-1-0"), "{refused}");

    std::fs::remove_file(&run.destination).unwrap();
    assert!(
        materialized_holds(&run.root, &round).is_err(),
        "a deletion refuses"
    );
}

#[test]
fn an_earlier_round_stands_on_a_later_rounds_copy_and_only_on_it() {
    let run = run();
    let first = landed(&run, "fix-1", b"round one", 1);
    let second = landed(&run, "fix-2", b"round two", 2);
    std::fs::write(&run.destination, "round two").unwrap();
    assert_eq!(materialized_holds(&run.root, &first), Ok(()));
    assert_eq!(materialized_holds(&run.root, &second), Ok(()));

    // Back to round one's bytes: no landing of the run left them LAST, so
    // neither round stands on them.
    std::fs::write(&run.destination, "round one").unwrap();
    assert!(materialized_holds(&run.root, &first).is_err());
    assert!(materialized_holds(&run.root, &second).is_err());
}

#[test]
fn a_claim_the_ledger_lacks_refuses_and_an_unrecorded_landing_is_no_later_copy() {
    let run = run();
    let round = landed(&run, "fix-1", b"round one", 1);
    // A later landing that never reached the ledger (it failed) is not the
    // last copy: round one still stands on its own bytes.
    let _failed = claimed(&run, "fix-2", b"never landed", 2);
    std::fs::write(&run.destination, "round one").unwrap();
    assert_eq!(materialized_holds(&run.root, &round), Ok(()));
    // A manifest claiming a copy the ledger does not hold refuses.
    let forged = claimed(&run, "fix-3", b"round one", 3);
    assert!(materialized_holds(&run.root, &forged).is_err());
}

/// The false green: a later re-persist drops the manifest's receipts, but
/// the ledger still says the item placed a copy -- so its loss refuses, with
/// or without any manifest at all.
#[test]
fn a_manifest_that_lost_its_receipts_still_answers_for_the_ledger() {
    let run = run();
    let round = landed(&run, "fix-1", b"regenerated", 1);
    let mut repersisted = round.clone();
    repersisted.materialized.clear();
    std::fs::write(&run.destination, "regenerated").unwrap();
    assert_eq!(materialized_holds(&run.root, &repersisted), Ok(()));
    std::fs::remove_file(&run.destination).unwrap();
    assert!(materialized_holds(&run.root, &repersisted).is_err());
    assert!(landing_copies_hold(&run.root, "fix-1", "fix-1-0", None).is_err());
    // A ledger line that does not parse fails every check closed.
    std::fs::write(&run.destination, "regenerated").unwrap();
    let ledger = run.root.join("write-coordination/materializations.jsonl");
    let mut text = std::fs::read_to_string(&ledger).unwrap();
    text.push_str("not json\n");
    std::fs::write(&ledger, text).unwrap();
    assert!(materialized_holds(&run.root, &round).is_err());
}

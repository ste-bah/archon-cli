//! Issue-25: a wave's own outcome is never an unexpected change.
//!
//! Live on wf-719ff3b0, the three-refresh unexpected-change allowance was
//! spent by ordinary waves: `agents-7-0` created `providers/tradingview_store.rs`
//! under the directory scope of its declared `providers/mod.rs` — in the
//! manifest's `created_files` and in the wave commit, but with no post-hash,
//! so the post-apply audit called it unexpected; and after a pause interrupted
//! a post-apply audit, the next dispatch found the tree differing from the
//! audit state and charged the wave's own apply to the allowance again. These
//! drive the production write-wave seam and prove both paths read the apply
//! receipt the host wrote.
#[path = "support/write_wave_fixture.rs"]
mod support;

use archon_workflow::repository_audit::budget::{AuditPolicy, Limit};
use archon_workflow::repository_audit::receipts::{ApplyReceipt, read_apply_receipts};
use archon_workflow::repository_audit::runtime::{AuditRuntime, Snapshot};
use archon_workflow::*;
use std::collections::BTreeMap;
use support::{AuditScript, Edits, Fixture, git};

fn no_findings() -> AuditScript {
    AuditScript {
        flagged: vec![],
        dispositions: BTreeMap::new(),
    }
}

/// Every `repository_audit_started` detail of the run, in order.
fn started_events(f: &Fixture) -> Vec<serde_json::Value> {
    std::fs::read_to_string(f.store.events_path(&f.run))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap()["detail"].clone())
        .filter(|detail| detail["event"] == "repository_audit_started")
        .collect()
}

/// A run whose audit pauses on the first unexpected-change refresh, so a
/// misclassified wave fails loudly instead of counting quietly.
fn strict_audit(f: &Fixture) -> AuditRuntime {
    AuditRuntime::initialize(
        f.store.clone(),
        f.run.clone(),
        AuditPolicy {
            attempt_timeout_secs: Limit::Unlimited,
            total_time_secs: Limit::Unlimited,
            unexpected_change_refreshes: Limit::Finite(0),
        },
    )
    .unwrap()
}

/// The live shape: `src/providers/mod.rs` declared, its directory the scope,
/// `store.rs` created beside it and never declared.
fn providers_baseline(f: &Fixture) {
    std::fs::create_dir_all(f.repo.join("src/providers")).unwrap();
    std::fs::write(f.repo.join("src/providers/mod.rs"), "// providers\n").unwrap();
    git(&f.repo, &["add", "src/providers/mod.rs"]);
    git(&f.repo, &["commit", "-qm", "providers module"]);
}

fn providers_edits() -> Edits {
    Edits {
        files: vec![
            ("src/providers/mod.rs", "mod store;\n"),
            ("src/providers/store.rs", "pub fn store() {}\n"),
        ],
        report: vec!["src/providers/mod.rs", "src/providers/store.rs"],
        via_adapter: false,
    }
}

#[tokio::test]
async fn a_file_created_inside_the_scope_but_not_declared_is_the_waves_own_change() {
    let f = Fixture::new();
    providers_baseline(&f);
    let audit = strict_audit(&f);
    let (out, _) = f
        .wave_audited(
            "scoped",
            vec![(vec!["src/providers/mod.rs"], providers_edits())],
            Some(no_findings()),
        )
        .await;
    assert_eq!(out.status, WorkflowV2Status::Accepted, "{out:#?}");
    assert_eq!(
        git(&f.repo, &["show", "HEAD:src/providers/store.rs"]),
        "pub fn store() {}"
    );
    let manifest = f.manifest("scoped", "scoped-0");
    assert_eq!(
        manifest["declared_target_files"],
        serde_json::json!(["src/providers/mod.rs"])
    );
    assert!(
        manifest["created_files"]
            .as_array()
            .unwrap()
            .contains(&serde_json::json!("src/providers/store.rs")),
        "{manifest:#}"
    );
    assert!(
        manifest["post_hashes"]["src/providers/store.rs"].is_string(),
        "landed file has no post-hash: {manifest:#}"
    );
    let receipts = read_apply_receipts(&f.store, &f.run).unwrap();
    assert_eq!(receipts.len(), 1, "{receipts:#?}");
    assert_eq!(
        receipts[0].unexpected_paths,
        Vec::<String>::new(),
        "{receipts:#?}"
    );
    assert_eq!(receipts[0].call_id, "scoped");
    let started = started_events(&f);
    let post_apply = started
        .iter()
        .find(|d| d["trigger"] == "post_apply")
        .unwrap_or_else(|| panic!("{started:#?}"));
    assert_eq!(
        post_apply["unexpected_paths"],
        serde_json::json!([]),
        "{post_apply:#}"
    );
    assert!(
        started.iter().all(|d| d["trigger"] != "unexpected_change"),
        "{started:#?}"
    );
    assert_eq!(audit.state().unwrap().budget.unexpected_refreshes, 0);
}

#[tokio::test]
async fn a_dispatch_after_an_interrupted_post_apply_audit_is_post_apply() {
    let f = Fixture::new();
    let audit = strict_audit(&f);
    let paths = vec!["owned.txt".to_string()];
    let before = Snapshot::capture(&f.repo, &paths, &f.v2).unwrap();
    let (first, _) = f
        .wave_audited(
            "first",
            vec![(
                vec!["owned.txt"],
                Edits {
                    files: vec![("owned.txt", "first\n")],
                    report: vec!["owned.txt"],
                    via_adapter: false,
                },
            )],
            Some(no_findings()),
        )
        .await;
    assert_eq!(first.status, WorkflowV2Status::Accepted, "{first:#?}");
    let receipts = read_apply_receipts(&f.store, &f.run).unwrap();
    let [receipt]: [ApplyReceipt; 1] = receipts.try_into().unwrap();
    assert_eq!(receipt.before, before.identity);
    assert_eq!(
        audit.state().unwrap().snapshot.unwrap().identity,
        receipt.after
    );
    // The post-apply audit never completed: the audit state still holds the
    // pre-apply snapshot while the repository holds the wave's outcome.
    audit
        .update(|state| {
            state.snapshot = Some(before.clone());
            Ok(())
        })
        .unwrap();
    let (second, _) = f
        .wave_audited(
            "second",
            vec![(
                vec!["other.txt"],
                Edits {
                    files: vec![("other.txt", "second\n")],
                    report: vec!["other.txt"],
                    via_adapter: false,
                },
            )],
            Some(no_findings()),
        )
        .await;
    assert_eq!(second.status, WorkflowV2Status::Accepted, "{second:#?}");
    let started = started_events(&f);
    // The first wave's own post-apply audit also moved `before` to `after`;
    // the second wave's dispatch is the later one.
    let resumed = started
        .iter()
        .rev()
        .find(|d| d["previous_snapshot"] == before.identity && d["snapshot"] == receipt.after)
        .unwrap_or_else(|| panic!("{started:#?}"));
    assert_ne!(
        resumed["call_id"], started[1]["call_id"],
        "second wave never re-audited: {started:#?}"
    );
    assert_eq!(resumed["trigger"], "post_apply", "{resumed:#}");
    assert_eq!(
        resumed["apply_receipt"]["commit"], receipt.commit,
        "{resumed:#}"
    );
    assert_eq!(
        resumed["unexpected_paths"],
        serde_json::json!([]),
        "{resumed:#}"
    );
    assert!(
        started.iter().all(|d| d["trigger"] != "unexpected_change"),
        "{started:#?}"
    );
    assert_eq!(audit.state().unwrap().budget.unexpected_refreshes, 0);
}

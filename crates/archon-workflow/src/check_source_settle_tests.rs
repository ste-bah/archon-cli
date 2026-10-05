use std::collections::BTreeSet;
use std::sync::Mutex;

use super::*;
use crate::check_source_pins::tests_support::contract;
use crate::check_source_pins::{BlobStore, ORIGIN_FREEZE, pin_contract};
use crate::check_source_resolve::SourceRoot;

struct Judge {
    accept: bool,
    seen: Mutex<Vec<SourceJudgeInput>>,
}

#[async_trait::async_trait]
impl SourceJudge for Judge {
    async fn judge(&self, input: &SourceJudgeInput) -> Result<SourceVerdict, String> {
        self.seen.lock().unwrap().push(input.clone());
        Ok(SourceVerdict {
            accepted: self.accept,
            reason: if self.accept {
                "keeps every assertion"
            } else {
                "drops the assertion"
            }
            .into(),
            counterexample: "none".into(),
        })
    }
}

fn judge(accept: bool) -> Judge {
    Judge {
        accept,
        seen: Mutex::new(Vec::new()),
    }
}

fn git(root: &Path, args: &[&str]) -> String {
    let out = std::process::Command::new("git")
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
    String::from_utf8(out.stdout).unwrap().trim().to_string()
}

struct World {
    _dir: tempfile::TempDir,
    repo: std::path::PathBuf,
    run: std::path::PathBuf,
    store: PinStore,
    pins: CheckSourcePins,
    contract: crate::task_set_contract::AcceptanceContract,
}

/// Persist `branch`'s outcome with `status`, holding `request_id`.
fn mark_landed(run: &Path, branch: &str, request_id: &str, status: crate::WorkflowV2Status) {
    let result = crate::WorkflowV2Result {
        status,
        data: serde_json::json!({"check_source_held": [{"request_id": request_id}]}),
        ..crate::WorkflowV2Result::default()
    };
    crate::WorkflowV2ResultStore::new(run.join("v2"))
        .save_branch_outcome(
            "wave",
            &crate::v2::scheduler::WorkflowV2BranchOutcome {
                item_id: branch.to_string(),
                role: "coder".into(),
                status,
                result: Some(result),
                error: None,
                failure_kind: None,
                item_input_hash: None,
                completion_evidence: Vec::new(),
            },
        )
        .unwrap();
}

fn world() -> World {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(repo.join("scripts")).unwrap();
    std::fs::write(repo.join("scripts/check.sh"), "test -f built || exit 1\n").unwrap();
    git(&repo, &["init", "-q"]);
    git(&repo, &["add", "."]);
    git(
        &repo,
        &[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "commit",
            "-qm",
            "base",
        ],
    );
    let contract = contract(&[
        ("AC-1", "bash scripts/check.sh"),
        ("AC-2", "bash scripts/new.sh"),
    ]);
    let store = PinStore {
        sidecar: dir.path().join("pins/sidecar.json"),
        blobs: BlobStore::at(dir.path().join("pins/blobs")),
        frozen: true,
        pin: None,
        tasks_root: None,
    };
    let roots = Roots {
        repository: &repo,
        project: &repo,
    };
    let pins = pin_contract(&contract, "d", &roots, ORIGIN_FREEZE, &store.blobs);
    store.write(&pins).unwrap();
    World {
        run: dir.path().join("run"),
        _dir: dir,
        repo,
        store,
        pins,
        contract,
    }
}

impl World {
    /// A change held from a branch that then landed.
    fn held(&self, path: &str, bytes: &[u8]) -> SourceChangeRequest {
        self.held_from(path, bytes, Some(crate::WorkflowV2Status::Accepted))
    }

    /// A change held from a branch whose outcome is `landed` (`None`: the
    /// branch has not finished).
    fn held_from(
        &self,
        path: &str,
        bytes: &[u8],
        landed: Option<crate::WorkflowV2Status>,
    ) -> SourceChangeRequest {
        let pinned = self
            .pins
            .checks
            .values()
            .flat_map(|c| &c.sources)
            .find(|s| s.path == path)
            .unwrap();
        let check = if path.ends_with("new.sh") {
            "AC-2"
        } else {
            "AC-1"
        };
        let branch = format!("wave-{}", path.replace(['/', '.'], "_"));
        let request = requests::record(
            &self.run,
            NewRequest {
                origin: ORIGIN_LANDING,
                check_ids: BTreeSet::from([check.to_string()]),
                root: SourceRoot::Repository,
                path,
                item: None,
                was_pinned: true,
                pinned_digest: pinned.digest.clone(),
                proposed: Some(bytes),
                proposed_file: None,
                landed_file_digest: None,
                call_id: "wave",
                branch_id: &branch,
                task_ids: vec!["TASK-1".into()],
            },
        )
        .unwrap();
        if let Some(status) = landed {
            mark_landed(&self.run, &branch, &request.request_id, status);
        }
        request
    }

    async fn settle(&self, judge: Option<&dyn SourceJudge>) -> Settled {
        let ctx = Settle {
            run_root: &self.run,
            roots: Roots {
                repository: &self.repo,
                project: &self.repo,
            },
            store: &self.store,
            contract: &self.contract,
            judge,
            judge_note: String::new(),
        };
        settle(&ctx, self.pins.clone()).await
    }
}

#[tokio::test]
async fn an_accepted_held_creation_is_applied_committed_and_repinned() {
    let w = world();
    let request = w.held("scripts/new.sh", b"test -f built\n");
    let judge = judge(true);
    let settled = w.settle(Some(&judge)).await;
    assert!(settled.defects.is_empty(), "{:?}", settled.defects);
    let seen = judge.seen.lock().unwrap();
    assert_eq!(seen[0].pinned, None, "absent at freeze");
    assert_eq!(seen[0].proposed.as_deref(), Some("test -f built\n"));
    let resolution = settled.settlements[0].resolution.clone().unwrap();
    assert_eq!(
        (resolution.verdict.as_str(), resolution.applied),
        (VERDICT_ACCEPTED, true)
    );
    assert_eq!(
        std::fs::read(w.repo.join("scripts/new.sh")).unwrap(),
        b"test -f built\n"
    );
    assert_eq!(
        git(&w.repo, &["status", "--porcelain", "--", "scripts"]),
        "",
        "committed"
    );
    let pinned = &settled.pins.checks["AC-2"].sources[0];
    assert_eq!(pinned.digest, request.proposed_digest);
    assert_eq!(settled.pins.repins[0].request_id, request.request_id);
    assert_eq!(
        w.store.read().unwrap().unwrap(),
        settled.pins,
        "the sidecar was re-pinned"
    );
    assert!(requests::pending(&w.run).unwrap().is_empty());
}

#[tokio::test]
async fn a_refused_held_change_stays_out_and_the_check_runs_on_its_pin() {
    let w = world();
    w.held("scripts/check.sh", b"exit 0\n");
    let settled = w.settle(Some(&judge(false))).await;
    assert!(settled.defects.is_empty(), "{:?}", settled.defects);
    let resolution = settled.settlements[0].resolution.clone().unwrap();
    assert_eq!(
        (resolution.verdict.as_str(), resolution.applied),
        (VERDICT_REFUTED, false)
    );
    assert_eq!(
        std::fs::read(w.repo.join("scripts/check.sh")).unwrap(),
        b"test -f built || exit 1\n"
    );
    assert_eq!(settled.pins, w.pins);
}

#[tokio::test]
async fn an_edit_outside_a_landing_is_judged_and_restored_when_refused() {
    let w = world();
    std::fs::write(w.repo.join("scripts/check.sh"), "exit 0\n").unwrap();
    git(
        &w.repo,
        &[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "commit",
            "-qam",
            "weaken",
        ],
    );
    let settled = w.settle(Some(&judge(false))).await;
    assert!(settled.defects.is_empty(), "{:?}", settled.defects);
    let request = &settled.settlements[0].request;
    assert_eq!(request.origin, ORIGIN_ACCEPTANCE_DRIFT);
    assert_eq!(
        std::fs::read(w.repo.join("scripts/check.sh")).unwrap(),
        b"test -f built || exit 1\n",
        "restored to its pin"
    );
    assert_eq!(
        git(&w.repo, &["status", "--porcelain", "--", "scripts"]),
        "",
        "the restore is committed"
    );
}

#[tokio::test]
async fn with_no_judge_a_changed_source_fails_its_check_and_stays_pending() {
    let w = world();
    std::fs::write(w.repo.join("scripts/check.sh"), "exit 0\n").unwrap();
    let settled = w.settle(None).await;
    assert!(
        settled.defects["AC-1"].contains("pending judgment"),
        "{:?}",
        settled.defects
    );
    assert!(!settled.defects.contains_key("AC-2"));
    assert_eq!(requests::pending(&w.run).unwrap().len(), 1);
    // The next round, with a judge that accepts, re-pins the tree's version.
    let settled = w.settle(Some(&judge(true))).await;
    assert!(settled.defects.is_empty(), "{:?}", settled.defects);
    assert_eq!(
        settled.pins.checks["AC-1"].sources[0].digest.as_deref(),
        Some(content_digest(b"exit 0\n").as_str())
    );
}

#[tokio::test]
async fn a_held_change_over_a_source_that_moved_is_stale() {
    let w = world();
    w.held("scripts/new.sh", b"test -f built\n");
    std::fs::write(w.repo.join("scripts/new.sh"), "true\n").unwrap();
    let settled = w.settle(Some(&judge(true))).await;
    let stale = settled
        .settlements
        .iter()
        .find(|s| s.request.origin == ORIGIN_LANDING)
        .unwrap();
    assert_eq!(stale.resolution.as_ref().unwrap().verdict, VERDICT_STALE);
}

#[tokio::test]
async fn an_accepted_unit_test_change_goes_in_over_its_pin_though_the_file_moved_on() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(repo.join("src")).unwrap();
    std::fs::write(repo.join("Cargo.toml"), "[package]\nname = \"p\"\n").unwrap();
    let lib = "pub fn f() -> u8 { 1 }\n#[cfg(test)]\nmod tests {\n    #[test]\n    fn keeps() { assert_eq!(super::f(), 1); }\n}\n";
    std::fs::write(repo.join("src/lib.rs"), lib).unwrap();
    let contract = contract(&[("AC-1", "cargo test keeps")]);
    let store = PinStore {
        sidecar: dir.path().join("pins/sidecar.json"),
        blobs: BlobStore::at(dir.path().join("pins/blobs")),
        frozen: true,
        pin: None,
        tasks_root: None,
    };
    let roots = Roots {
        repository: &repo,
        project: &repo,
    };
    let pins = pin_contract(&contract, "d", &roots, ORIGIN_FREEZE, &store.blobs);
    let pinned = pins.checks["AC-1"]
        .sources
        .iter()
        .find(|s| s.item.as_deref() == Some("fn:tests::keeps"))
        .unwrap()
        .clone();
    let stronger =
        "#[test]\n    fn keeps() { assert_eq!(super::f(), 1); assert!(super::f() > 0); }";
    let run = dir.path().join("run");
    let request = requests::record(
        &run,
        NewRequest {
            origin: ORIGIN_LANDING,
            check_ids: BTreeSet::from(["AC-1".to_string()]),
            root: SourceRoot::Repository,
            path: "src/lib.rs",
            item: Some("fn:tests::keeps"),
            was_pinned: true,
            pinned_digest: pinned.digest.clone(),
            proposed: Some(stronger.as_bytes()),
            proposed_file: Some(b"irrelevant"),
            landed_file_digest: Some(content_digest(lib.as_bytes())),
            call_id: "wave",
            branch_id: "wave-0",
            task_ids: Vec::new(),
        },
    )
    .unwrap();
    mark_landed(
        &run,
        "wave-0",
        &request.request_id,
        crate::WorkflowV2Status::Accepted,
    );
    // A later landing changed the implementation around the pinned test.
    std::fs::write(repo.join("src/lib.rs"), lib.replace("{ 1 }", "{ 0 + 1 }")).unwrap();
    let judge = judge(true);
    let settled = settle(
        &Settle {
            run_root: &run,
            roots,
            store: &store,
            contract: &contract,
            judge: Some(&judge),
            judge_note: String::new(),
        },
        pins,
    )
    .await;
    assert!(settled.defects.is_empty(), "{:?}", settled.defects);
    let text = std::fs::read_to_string(repo.join("src/lib.rs")).unwrap();
    assert!(
        text.contains("{ 0 + 1 }") && text.contains("assert!(super::f() > 0)"),
        "{text}"
    );
    assert_eq!(
        settled.pins.checks["AC-1"]
            .sources
            .iter()
            .find(|s| s.item.as_deref() == Some("fn:tests::keeps"))
            .unwrap()
            .digest,
        request.proposed_digest
    );
}

#[path = "check_source_settle_tests_b.rs"]
mod b;
#[path = "check_source_settle_tests_c.rs"]
mod c;

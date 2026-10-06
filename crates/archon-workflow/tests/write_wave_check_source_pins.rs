//! PLAN-11: a landing cannot edit the test that judges it.
//!
//! Drives the production write wave with real Git writes over a task set
//! whose frozen checks' sources are pinned: an integration test target, a
//! unit test function inside an implementation file, and a target that does
//! not exist yet. A branch that weakens the pinned test and the unit test
//! while implementing has both held out of its landing -- the target
//! restored, the unit test spliced back to its pinned text -- and a
//! re-author request recorded for each check, while its implementation
//! lands. A branch that CREATES the missing target has the creation held and
//! recorded too; the acceptance judge then accepts it, the host applies and
//! commits it and re-pins the check, and refuses the weakening, which never
//! lands.
#[path = "support/write_wave_fixture.rs"]
mod support;

use std::path::Path;

use archon_workflow::check_source_pins::{BlobStore, PinStore};
use archon_workflow::check_source_requests::{
    self as requests, ORIGIN_LANDING, VERDICT_ACCEPTED, VERDICT_REFUTED,
};
use archon_workflow::check_source_resolve::Roots;
use archon_workflow::check_source_settle::{
    Settle, SourceJudge, SourceJudgeInput, SourceVerdict, settle,
};
use archon_workflow::task_set_contract::{AcceptanceContract, content_digest};
use archon_workflow::*;
use serde_json::json;
use support::{Edits, Fixture, git};

#[path = "support/check_source_world.rs"]
mod world;
use world::*;

#[tokio::test]
async fn a_landing_that_weakens_a_pinned_test_has_the_edit_held_and_the_rest_lands() {
    let f = Fixture::new();
    frozen_task_set(&f);
    let out = f
        .wave(
            "weaken",
            vec![(
                vec!["owned.txt", "src/lib.rs", "tests/judge.rs"],
                Edits {
                    files: vec![
                        ("owned.txt", "implemented\n"),
                        ("src/lib.rs", LIB_WEAKENED),
                        ("tests/judge.rs", "#[test]\nfn judges() {}\n"),
                    ],
                    report: vec!["owned.txt", "src/lib.rs", "tests/judge.rs"],
                    via_adapter: true,
                },
            )],
        )
        .await;
    assert_eq!(out.status, WorkflowV2Status::Accepted, "{out:#?}");
    let result = f.branch_result("weaken", "weaken-0");
    assert_eq!(result.data["patch_landed"], json!(true), "{result:#?}");
    assert_eq!(
        held(&result),
        [
            (
                "src/lib.rs".to_string(),
                Some("fn:tests::unit_guard".to_string())
            ),
            ("tests/judge.rs".to_string(), None),
        ]
    );
    assert!(
        result
            .residual_gaps
            .iter()
            .any(|gap| gap.id == "check_source_change_held_weaken-0"),
        "{result:#?}"
    );
    // The rest landed: the implementation, with the pinned unit test intact.
    assert_eq!(git(&f.repo, &["show", "HEAD:owned.txt"]), "implemented");
    let lib = git(&f.repo, &["show", "HEAD:src/lib.rs"]);
    assert!(lib.contains("    2\n}"), "{lib}");
    assert!(lib.contains("assert_eq!(super::value(), 2);"), "{lib}");
    assert_eq!(
        git(&f.repo, &["show", "HEAD:tests/judge.rs"]),
        JUDGE_TEST.trim_end()
    );
    // A re-author request per held change, carrying the proposed bytes.
    let pending = requests::pending(&run_root(&f)).unwrap();
    assert_eq!(pending.len(), 2, "{pending:#?}");
    let target = pending.iter().find(|r| r.path == "tests/judge.rs").unwrap();
    assert_eq!(target.origin, ORIGIN_LANDING);
    assert_eq!(target.check_ids.iter().collect::<Vec<_>>(), ["AC-1"]);
    assert_eq!(target.task_ids, ["TASK-001"]);
    let proposed = requests::blobs(&run_root(&f))
        .get(target.proposed_digest.as_ref().unwrap())
        .unwrap();
    assert_eq!(proposed, b"#[test]\nfn judges() {}\n");
    let unit = pending.iter().find(|r| r.path == "src/lib.rs").unwrap();
    assert_eq!(unit.check_ids.iter().collect::<Vec<_>>(), ["AC-2"]);
}

struct Judge;

#[async_trait::async_trait]
impl SourceJudge for Judge {
    async fn judge(&self, input: &SourceJudgeInput) -> Result<SourceVerdict, String> {
        // Accepts a real test created for the first time, refuses a proposal
        // that drops the assertion.
        let asserts = input
            .proposed
            .as_deref()
            .is_some_and(|text| text.contains("assert"));
        Ok(SourceVerdict {
            accepted: asserts,
            reason: if asserts {
                "tests the criterion"
            } else {
                "drops the assertion"
            }
            .into(),
            counterexample: "none".into(),
        })
    }
}

async fn settle_round(
    f: &Fixture,
    tasks: &Path,
    contract: &AcceptanceContract,
) -> archon_workflow::check_source_settle::Settled {
    let project = project_root(f);
    let roots = Roots {
        repository: &f.repo,
        project: &project,
    };
    let (store, pins) =
        archon_workflow::check_source_pins::load_for_run(&run_root(f), &project, tasks, &roots)
            .unwrap()
            .unwrap();
    assert!(store.frozen, "the run reads the frozen sidecar");
    settle(
        &Settle {
            run_root: &run_root(f),
            roots,
            store: &store,
            contract,
            judge: Some(&Judge),
            judge_note: String::new(),
        },
        pins,
    )
    .await
}

#[tokio::test]
async fn a_test_missing_at_freeze_is_created_only_through_the_judge_and_then_pinned() {
    let f = Fixture::new();
    let (tasks, contract) = frozen_task_set(&f);
    let out = f
        .wave(
            "create",
            vec![(
                vec!["other.txt", "tests/later.rs", "tests/judge.rs"],
                Edits {
                    files: vec![
                        ("other.txt", "other implemented\n"),
                        ("tests/later.rs", LATER_TEST),
                        ("tests/judge.rs", "#[test]\nfn judges() {}\n"),
                    ],
                    report: vec!["other.txt", "tests/later.rs", "tests/judge.rs"],
                    via_adapter: true,
                },
            )],
        )
        .await;
    assert_eq!(out.status, WorkflowV2Status::Accepted, "{out:#?}");
    let result = f.branch_result("create", "create-0");
    assert_eq!(result.data["patch_landed"], json!(true), "{result:#?}");
    assert_eq!(
        git(&f.repo, &["show", "HEAD:other.txt"]),
        "other implemented"
    );
    // The creation did not land unjudged.
    assert!(!f.repo.join("tests/later.rs").exists());
    let created = requests::pending(&run_root(&f))
        .unwrap()
        .into_iter()
        .find(|r| r.path == "tests/later.rs")
        .unwrap();
    assert!(
        created.was_pinned && created.pinned_digest.is_none(),
        "{created:#?}"
    );
    // The acceptance round: the judge accepts the created test and refuses
    // the weakening.
    let settled = settle_round(&f, &tasks, &contract).await;
    assert!(settled.defects.is_empty(), "{:?}", settled.defects);
    let verdicts: Vec<(String, String)> = settled
        .settlements
        .iter()
        .map(|s| {
            (
                s.request.path.clone(),
                s.resolution.clone().unwrap().verdict,
            )
        })
        .collect();
    assert!(
        verdicts.contains(&("tests/later.rs".into(), VERDICT_ACCEPTED.into())),
        "{verdicts:?}"
    );
    assert!(
        verdicts.contains(&("tests/judge.rs".into(), VERDICT_REFUTED.into())),
        "{verdicts:?}"
    );
    assert_eq!(
        git(&f.repo, &["show", "HEAD:tests/later.rs"]),
        LATER_TEST.trim_end()
    );
    assert_eq!(
        git(&f.repo, &["show", "HEAD:tests/judge.rs"]),
        JUDGE_TEST.trim_end()
    );
    // Re-pinned with recorded lineage, in the frozen sidecar.
    let sidecar = PinStore::frozen(&project_root(&f), &tasks)
        .read()
        .unwrap()
        .unwrap();
    assert_eq!(
        sidecar.checks["AC-3"]
            .sources
            .iter()
            .find(|s| s.path == "tests/later.rs")
            .unwrap()
            .digest
            .as_deref(),
        Some(content_digest(LATER_TEST.as_bytes()).as_str())
    );
    assert_eq!(sidecar.repins.len(), 1);
    assert_eq!(sidecar.repins[0].request_id, created.request_id);
    assert!(
        BlobStore::at(project_root(&f).join(".archon/task-set-pins/check-sources/blobs"))
            .get(&sidecar.repins[0].prior_digest)
            .is_some(),
        "the replaced sidecar is filed by digest"
    );
    // A later landing of the now-pinned test's same bytes holds nothing, and
    // nothing is left to settle.
    assert!(requests::pending(&run_root(&f)).unwrap().is_empty());
    let again = settle_round(&f, &tasks, &contract).await;
    assert!(again.settlements.is_empty() && again.defects.is_empty());
}

/// Fail closed: pins that exist but cannot be read refuse the branch with an
/// operational reason and land nothing; once they read again, the same work
/// re-run lands.
#[tokio::test]
async fn unreadable_pins_refuse_the_branch_and_a_rerun_lands_once_they_read() {
    let f = Fixture::new();
    let (tasks, _) = frozen_task_set(&f);
    let store = PinStore::frozen(&project_root(&f), &tasks);
    let good = std::fs::read(&store.sidecar).unwrap();
    std::fs::write(&store.sidecar, b"not json").unwrap();
    let edits = || Edits {
        files: vec![("owned.txt", "implemented\n")],
        report: vec!["owned.txt"],
        via_adapter: true,
    };
    f.wave("unread", vec![(vec!["owned.txt"], edits())]).await;
    let result = f.branch_result("unread", "unread-0");
    assert_eq!(result.status, WorkflowV2Status::Failed, "{result:#?}");
    assert_eq!(result.data["failure_kind"], json!("execution"));
    assert!(
        result.data["check_source_pins_unavailable"].is_string(),
        "{result:#?}"
    );
    assert_eq!(result.data["patch_landed"], json!(false));
    assert_eq!(git(&f.repo, &["show", "HEAD:owned.txt"]), "baseline");
    std::fs::write(&store.sidecar, good).unwrap();
    let out = f.wave("reread", vec![(vec!["owned.txt"], edits())]).await;
    assert_eq!(out.status, WorkflowV2Status::Accepted, "{out:#?}");
    assert_eq!(git(&f.repo, &["show", "HEAD:owned.txt"]), "implemented");
}

/// The module chain and the manifest entry are pinned with the test: a
/// branch that switches the pinned unit test's module off, and one that
/// points the pinned target at a weaker file through a `[[test]]` entry,
/// have those changes held while their other work -- a dependency line in
/// the same manifest included -- lands.
#[tokio::test]
async fn a_landing_cannot_switch_off_a_pinned_module_or_redirect_a_pinned_target() {
    let f = Fixture::new();
    frozen_task_set(&f);
    let off = LIB
        .replace("#[cfg(test)]\nmod tests", "#[cfg(any())]\nmod tests")
        .replace("    1\n}", "    2\n}");
    let off: &'static str = Box::leak(off.into_boxed_str());
    let manifest = "[package]\nname = \"demo\"\n\n[dependencies]\nserde = \"1\"\n\n[[test]]\nname = \"judge\"\npath = \"tests/weak.rs\"\n";
    let out = f
        .wave(
            "chain",
            vec![(
                vec!["src/lib.rs", "Cargo.toml", "tests/weak.rs"],
                Edits {
                    files: vec![
                        ("src/lib.rs", off),
                        ("Cargo.toml", manifest),
                        ("tests/weak.rs", "#[test]\nfn judges() {}\n"),
                    ],
                    report: vec!["src/lib.rs", "Cargo.toml", "tests/weak.rs"],
                    via_adapter: true,
                },
            )],
        )
        .await;
    assert_eq!(out.status, WorkflowV2Status::Accepted, "{out:#?}");
    let result = f.branch_result("chain", "chain-0");
    let held = held(&result);
    for expected in [
        ("src/lib.rs".to_string(), Some("mod:tests".to_string())),
        (
            "Cargo.toml".to_string(),
            Some("toml:test:judge".to_string()),
        ),
        ("tests/weak.rs".to_string(), None),
    ] {
        assert!(held.contains(&expected), "{expected:?} not held: {held:?}");
    }
    let lib = git(&f.repo, &["show", "HEAD:src/lib.rs"]);
    assert!(
        lib.contains("#[cfg(test)]\nmod tests") && lib.contains("    2\n}"),
        "{lib}"
    );
    let cargo = git(&f.repo, &["show", "HEAD:Cargo.toml"]);
    assert!(
        cargo.contains("serde = \"1\"") && !cargo.contains("[[test]]"),
        "{cargo}"
    );
    assert!(git(&f.repo, &["ls-files", "tests/weak.rs"]).is_empty());
}

/// Issue 338: the host's settlement, as this binary installs it: a journal
/// whose bytes say `stuck` cannot be settled (its cleanup fails); any other
/// is settled by removing it.
fn install_settlement() {
    archon_workflow::task_set_publish_lock::register_settle(|pin, _tasks| {
        for path in archon_workflow::task_set_publish_lock::journal_paths(pin) {
            match std::fs::read_to_string(&path) {
                Ok(text) if text.contains("stuck") => {
                    return Err(format!("removing {}: stuck", path.display()));
                }
                Ok(_) => std::fs::remove_file(&path).map_err(|error| error.to_string())?,
                Err(_) => {}
            }
        }
        Ok(())
    });
}

fn journal(f: &Fixture, tasks: &Path, temp: bool) -> std::path::PathBuf {
    let pin = PinStore::frozen(&project_root(f), tasks).pin.unwrap();
    archon_workflow::task_set_publish_lock::journal_paths(&pin)[usize::from(temp)].clone()
}

fn implement() -> Edits {
    Edits {
        files: vec![("owned.txt", "implemented\n")],
        report: vec!["owned.txt"],
        via_adapter: true,
    }
}

/// Issue 338: a publish a crash left mid-commit is settled under the
/// exclusive lock before the landing reads its pins, and the branch lands.
#[tokio::test]
async fn a_publish_left_mid_commit_is_settled_before_the_landing_reads_its_pins() {
    install_settlement();
    let f = Fixture::new();
    let (tasks, _) = frozen_task_set(&f);
    let left = journal(&f, &tasks, false);
    std::fs::write(&left, br#"{"state": "committed"}"#).unwrap();
    let out = f
        .wave("settle", vec![(vec!["owned.txt"], implement())])
        .await;
    assert_eq!(out.status, WorkflowV2Status::Accepted, "{out:#?}");
    assert!(!left.exists(), "the landing read over a left journal");
    assert_eq!(git(&f.repo, &["show", "HEAD:owned.txt"]), "implemented");
}

/// Issue 338: a journal no read can settle -- its cleanup fails, or only its
/// temp is left and that cannot be removed -- pauses the run with the
/// evidence; it never refuses the branch (every branch would be), and once
/// the operator fixes the cause the same work lands.
#[tokio::test]
async fn a_journal_no_read_can_settle_pauses_the_wave_and_the_work_lands_once_fixed() {
    install_settlement();
    for temp in [false, true] {
        let f = Fixture::new();
        let (tasks, _) = frozen_task_set(&f);
        let left = journal(&f, &tasks, temp);
        std::fs::write(&left, br#"{"state": "committed", "why": "stuck"}"#).unwrap();
        let error = f
            .try_wave("stuck", vec![(vec!["owned.txt"], implement())])
            .await
            .expect_err("a wave over an unsettled journal");
        let WorkflowError::ControlPaused(evidence) = error else {
            panic!("the wave failed instead of pausing: {error}");
        };
        let state = if temp {
            "a journal temp only"
        } else {
            "committed"
        };
        // The landing policy canonicalizes the project (including Windows
        // short names) and removes its verbatim prefix before locating pins.
        // Canonicalize the existing root: the journal itself is absent when
        // only its temp is left.
        let project = archon_shell::paths::canonicalize(project_root(&f)).unwrap();
        let pin = PinStore::frozen(&project, &tasks).pin.unwrap();
        let expected_journal = archon_workflow::task_set_publish_lock::journal_paths(&pin)[0]
            .display()
            .to_string();
        for needed in [
            expected_journal,
            format!("state: {state}"),
            "stuck".into(),
            "Operator remedy".into(),
            "archon workflow resume --live --yes".into(),
        ] {
            assert!(evidence.contains(&needed), "{needed} missing: {evidence}");
        }
        assert!(left.exists(), "the journal's decision was lost");
        assert_eq!(git(&f.repo, &["show", "HEAD:owned.txt"]), "baseline");
        let partial = run_root(&f).join("write-coordination/stages/stuck/partial/stuck-0.patch");
        let kept = std::fs::read_to_string(&partial).expect("the paused branch's work is kept");
        assert!(kept.contains("implemented"), "{kept}");
        // The operator's fix: the cleanup can run again.
        std::fs::write(&left, br#"{"state": "committed"}"#).unwrap();
        let out = f
            .wave("fixed", vec![(vec!["owned.txt"], implement())])
            .await;
        assert_eq!(out.status, WorkflowV2Status::Accepted, "{out:#?}");
        assert!(!left.exists());
        assert_eq!(git(&f.repo, &["show", "HEAD:owned.txt"]), "implemented");
    }
}

//! Batch O: scope amendments are planned from the run's facts, validated by
//! the host, and chained like the acceptance lineage.

use super::*;
use crate::task_universe::WorkflowV2TaskUniverseTask;

struct World {
    _dir: tempfile::TempDir,
    repo: PathBuf,
    run_root: PathBuf,
}

fn world() -> World {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    let run_root = dir.path().join("project/.archon/workflows/run1");
    std::fs::create_dir_all(&run_root).unwrap();
    for rel in [
        "crates/p/src/owner.rs",
        "crates/p/src/store/methods.rs",
        "crates/p/src/store/store_tests/unit.rs",
        "crates/p/src/providers/feed.rs",
        "crates/p/tests/gates.rs",
        "docs/audit.md",
        "src/sealed/x.rs",
    ] {
        let path = repo.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, "//\n").unwrap();
    }
    std::fs::create_dir_all(repo.join("tasks")).unwrap();
    std::fs::write(
        repo.join("tasks/TASK-C.md"),
        "Run `cargo test --test gates` over crates/p/tests/gates.rs.",
    )
    .unwrap();
    World {
        _dir: dir,
        repo,
        run_root,
    }
}

fn task(id: &str, owns: &[&str], forbids: &[&str]) -> WorkflowV2TaskUniverseTask {
    WorkflowV2TaskUniverseTask {
        canonical_task_id: id.into(),
        source_path: format!("tasks/{id}.md"),
        files_expected_to_change: owns.iter().map(|f| format!("`{f}` — exists")).collect(),
        files_forbidden_to_change: forbids.iter().map(|f| f.to_string()).collect(),
        ..Default::default()
    }
}

/// The live shape: two tasks share a declared module; the authored script
/// gave the first only one of its three files and the second none of its
/// shared one.
fn universe() -> WorkflowV2TaskUniverse {
    WorkflowV2TaskUniverse {
        schema_version: "test".into(),
        source_roots: Vec::new(),
        tasks: vec![
            task(
                "TASK-A",
                &[
                    "crates/p/src/owner.rs",
                    "crates/p/src/store/methods.rs",
                    "crates/p/src/store/store_tests/unit.rs",
                ],
                &[],
            ),
            task(
                "TASK-B",
                &["crates/p/src/store/methods.rs"],
                &["`src/sealed/`"],
            ),
            task("TASK-C", &["crates/p/src/owner_c.rs"], &[]),
        ],
    }
}

fn set(paths: &[&str]) -> BTreeSet<String> {
    paths.iter().map(|p| p.to_string()).collect()
}

#[test]
fn the_planner_restores_declared_files_and_assigns_ownerless_ones() {
    let w = world();
    let u = universe();
    let authored = BTreeMap::from([
        ("TASK-A".to_string(), set(&["crates/p/src/owner.rs"])),
        ("TASK-B".to_string(), set(&[])),
    ]);
    let landed = BTreeMap::from([(
        "TASK-B".to_string(),
        set(&["crates/p/src/providers/feed.rs", "src/sealed/x.rs"]),
    )]);
    let named = vec![set(&["crates/p/tests/gates.rs", ".git/config"])];
    let plan = plan_scope_amendments(&ScopePlanInputs {
        universe: &u,
        repository_root: &w.repo,
        project_root: None,
        authored: &authored,
        landed_files_by_task: &landed,
        finding_named_files: &named,
        focused_test_files_by_task: &BTreeMap::new(),
    });
    let got: Vec<(&str, &str, ScopeGrantKind)> = plan
        .amendments
        .iter()
        .map(|g| (g.task_id.as_str(), g.path.as_str(), g.kind))
        .collect();
    assert_eq!(
        got,
        [
            (
                "TASK-A",
                "crates/p/src/store/methods.rs",
                ScopeGrantKind::DeclaredRestore
            ),
            (
                "TASK-A",
                "crates/p/src/store/store_tests/unit.rs",
                ScopeGrantKind::DeclaredRestore
            ),
            (
                "TASK-B",
                "crates/p/src/providers/feed.rs",
                ScopeGrantKind::Owner
            ),
            (
                "TASK-B",
                "crates/p/src/store/methods.rs",
                ScopeGrantKind::DeclaredRestore
            ),
            ("TASK-C", "crates/p/tests/gates.rs", ScopeGrantKind::Owner),
        ],
        "TASK-C was not authored, so it is restored nothing; the file it names is its own -- ownerless files get an OWNER record, never write scope"
    );
    let unassigned: Vec<&str> = plan.unassigned.iter().map(|(f, _)| f.as_str()).collect();
    assert_eq!(
        unassigned,
        [".git/config", "src/sealed/x.rs"],
        "engine state is never granted, and a forbidding lander is passed over -- both returned, never dropped: {plan:?}"
    );
}

#[test]
fn a_transaction_validates_records_shares_and_chains_every_amendment() {
    let w = world();
    let u = universe();
    let grant = |task: &str, path: &str, kind| ScopeAmendment {
        task_id: task.into(),
        path: path.into(),
        kind,
        root: ScopeGrantRoot::Repository,
        shared_with: BTreeSet::new(),
        evidence: String::new(),
    };
    let first = amend_task_scope(ScopeAmendmentRequest {
        run_root: &w.run_root,
        universe: &u,
        repository_root: &w.repo,
        grants: vec![
            grant(
                "TASK-A",
                "crates/p/src/store/methods.rs",
                ScopeGrantKind::DeclaredRestore,
            ),
            grant("TASK-C", "docs/audit.md", ScopeGrantKind::DeliverableRoot),
            grant(
                "TASK-C",
                ".archon/config.toml",
                ScopeGrantKind::DeliverableRoot,
            ),
            grant(
                "TASK-C",
                "crates/p/src/absent.rs",
                ScopeGrantKind::OwnerlessAssignment,
            ),
            grant(
                "TASK-B",
                "src/sealed/x.rs",
                ScopeGrantKind::OwnerlessAssignment,
            ),
            grant("TASK-Z", "docs/audit.md", ScopeGrantKind::DeliverableRoot),
        ],
        trigger: "review finding f-1",
    })
    .unwrap();
    assert_eq!(first.applied.len(), 2, "{first:?}");
    let shared = &first.applied[0];
    assert_eq!(shared.path, "crates/p/src/store/methods.rs");
    assert_eq!(
        shared.shared_with,
        set(&["TASK-B"]),
        "another task's file is granted only as a recorded shared grant"
    );
    let refused: Vec<&str> = first.refused.iter().map(|(g, _)| g.path.as_str()).collect();
    assert_eq!(
        refused,
        [
            ".archon/config.toml",
            "crates/p/src/absent.rs",
            "src/sealed/x.rs",
            "docs/audit.md"
        ]
    );
    let link = first.link.clone().expect("a link");
    assert_eq!(link.from_digest, ScopeAmendmentSet::default().digest());
    assert_eq!(link.changed_task_ids, set(&["TASK-A", "TASK-C"]));
    assert_eq!(link.trigger, "review finding f-1");
    assert!(link.prior_link_digest.is_none());

    // The same grants again change nothing and add no link.
    let again = amend_task_scope(ScopeAmendmentRequest {
        run_root: &w.run_root,
        universe: &u,
        repository_root: &w.repo,
        grants: vec![grant(
            "TASK-C",
            "docs/audit.md",
            ScopeGrantKind::DeliverableRoot,
        )],
        trigger: "repeat",
    })
    .unwrap();
    assert!(again.link.is_none() && again.applied.is_empty());
    assert_eq!(again.digest, first.digest);

    let second = amend_task_scope(ScopeAmendmentRequest {
        run_root: &w.run_root,
        universe: &u,
        repository_root: &w.repo,
        grants: vec![grant(
            "TASK-C",
            "crates/p/tests/gates.rs",
            ScopeGrantKind::OwnerlessAssignment,
        )],
        trigger: "focused test",
    })
    .unwrap();
    let link2 = second.link.unwrap();
    assert_eq!(link2.from_digest, first.digest);
    assert_eq!(link2.prior_link_digest, Some(link.digest()));

    let ledger = ScopeAmendmentLedger::load(&w.run_root).unwrap();
    assert_eq!(ledger.lineage.len(), 2);
    assert_eq!(ledger.set.grants.len(), 3);
    let store = history(&w.run_root);
    for digest in [&first.digest, &second.digest] {
        assert!(store.get(digest).unwrap().is_some(), "filed by digest");
    }
    let log = std::fs::read_to_string(log_path(&w.run_root)).unwrap();
    assert_eq!(log.lines().count(), 3, "every decision is logged");

    // The amended universe gives each grantee its grant.
    let amended = amended_universe(&u, &ledger.set);
    let owners = |path: &str| {
        crate::v2::script::residual_paths::owners(&amended, path, &w.repo)
            .into_iter()
            .collect::<Vec<_>>()
    };
    assert_eq!(owners("docs/audit.md"), ["TASK-C"]);
    assert_eq!(
        owners("crates/p/src/store/methods.rs"),
        ["TASK-A", "TASK-B"]
    );

    // A broken chain is refused on read.
    let mut broken = ledger.clone();
    broken.lineage[1].prior_link_digest = None;
    assert!(broken.verify(&store).unwrap_err().0.contains("link 1"));
    let mut orphan = ledger;
    orphan.lineage.remove(0);
    assert!(orphan.verify(&store).is_err());
}

#[test]
fn project_data_is_granted_only_where_the_project_input_landing_can_place_it() {
    let w = world();
    let project = w
        .run_root
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    std::fs::create_dir_all(project.join("tasks")).unwrap();
    for rel in [".archon/lab/data/bars.json", ".archon/agents/a/prompt.md"] {
        let path = project.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, "{}").unwrap();
    }
    let u = universe();
    let grant = |path: &str| ScopeAmendment {
        task_id: "TASK-B".into(),
        path: path.into(),
        kind: ScopeGrantKind::DeliverableRoot,
        root: ScopeGrantRoot::Repository,
        shared_with: BTreeSet::new(),
        evidence: "finding names wrong stored data".into(),
    };
    crate::write_coordinator::project_inputs::write_test_policy(&w.run_root, project, &[]);
    let outcome = amend_task_scope(ScopeAmendmentRequest {
        run_root: &w.run_root,
        universe: &u,
        repository_root: &w.repo,
        grants: vec![
            grant(".archon/lab/data/bars.json"),
            grant(".archon/agents/a/prompt.md"),
        ],
        trigger: "t",
    })
    .unwrap();
    assert_eq!(outcome.applied.len(), 1, "{outcome:?}");
    assert_eq!(
        outcome.applied[0].root,
        ScopeGrantRoot::Project,
        "project data always lands through the project inputs"
    );
    assert_eq!(outcome.refused[0].0.path, ".archon/agents/a/prompt.md");
    let ledger = ScopeAmendmentLedger::load(&w.run_root).unwrap();
    assert_eq!(
        project_data_grants(&ledger.set, &["TASK-B".into()]),
        [".archon/lab/data/bars.json"]
    );
    assert!(project_data_grants(&ledger.set, &["TASK-A".into()]).is_empty());
}

/// Batch O (I11): an amendment never grants the frozen acceptance chain --
/// contract, skeleton, their locks, the pin store -- wherever it sits, and
/// the transaction writes only the run's own ledger, log and history.
#[test]
fn an_amendment_never_grants_the_frozen_acceptance_chain() {
    let w = world();
    let u = universe();
    for rel in [
        "tasks/SET/acceptance-contract.json",
        "tasks/SET/sub/task-skeleton.json",
        "crates/p/acceptance-contract.lock",
    ] {
        let path = w.repo.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, "{}").unwrap();
    }
    let grant = |path: &str| ScopeAmendment {
        task_id: "TASK-C".into(),
        path: path.into(),
        kind: ScopeGrantKind::DeliverableRoot,
        root: ScopeGrantRoot::Repository,
        shared_with: BTreeSet::new(),
        evidence: String::new(),
    };
    let outcome = amend_task_scope(ScopeAmendmentRequest {
        run_root: &w.run_root,
        universe: &u,
        repository_root: &w.repo,
        grants: vec![
            grant("tasks/SET/acceptance-contract.json"),
            grant("tasks/SET/sub/task-skeleton.json"),
            grant("crates/p/acceptance-contract.lock"),
            grant(".archon/task-set-pins/abc.json"),
        ],
        trigger: "t",
    })
    .unwrap();
    assert!(outcome.applied.is_empty(), "{outcome:?}");
    assert_eq!(outcome.refused.len(), 4, "{outcome:?}");
    let written: Vec<String> = std::fs::read_dir(w.run_root.join("v2"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert!(
        written
            .iter()
            .all(|name| name.starts_with("scope-amendments") || name == "history"),
        "{written:?}"
    );
}

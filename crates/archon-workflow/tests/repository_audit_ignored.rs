//! Issue-26: a gitignored declared deliverable is a project artifact, never a
//! repository-audit obligation. Live: TASK-DL-001 declared
//! `docs/trading-data-lake-gap-audit.md` under `.gitignore:67 /docs/*`; the
//! write layer retained it and accepted the wave, the audit recorded it
//! `absent / deliver`, and the task was re-dispatched on every resume.
use archon_workflow::repository_audit::budget::{AuditPolicy, Limit};
use archon_workflow::repository_audit::runtime::{AuditRuntime, Snapshot};
use archon_workflow::repository_audit::{AuditContract, AuditReport, ignored, reuse};
use archon_workflow::*;
use serde_json::json;
use std::path::{Path, PathBuf};

fn git(repo: &Path, args: &[&str]) {
    let status = std::process::Command::new("git")
        .args([
            "-c",
            "user.name=fixture",
            "-c",
            "user.email=fixture@example.invalid",
        ])
        .args(args)
        .current_dir(repo)
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?}");
}

/// A repository ignoring `docs/*`, with `src/y.rs` committed. The ignored
/// deliverable `docs/x.md` is not on disk: the write layer retains it as a run
/// artifact, so the repository never holds it.
fn repository(temp: &Path) -> PathBuf {
    let repo = temp.join("repo");
    std::fs::create_dir_all(repo.join("docs")).unwrap();
    std::fs::create_dir_all(repo.join("src")).unwrap();
    std::fs::write(repo.join("src/y.rs"), "fn y() {}\n").unwrap();
    std::fs::write(repo.join("docs/tracked.md"), "kept\n").unwrap();
    std::fs::write(repo.join(".gitignore"), "/docs/*\n").unwrap();
    git(&repo, &["init", "-q"]);
    git(
        &repo,
        &["add", "-f", "src/y.rs", "docs/tracked.md", ".gitignore"],
    );
    git(&repo, &["commit", "-qm", "base"]);
    repo
}

fn runtime(temp: &Path) -> (WorkflowStore, String, AuditRuntime) {
    let store = WorkflowStore::project(&temp.join("project"));
    let run = store
        .create_run(WorkflowSpec {
            schema: spec::WORKFLOW_SCHEMA.into(),
            name: "ignored".into(),
            task: "audit".into(),
            target_repository_root: None,
            max_agents: 1,
            max_parallelism: 1,
            stages: vec![],
            permissions: Default::default(),
            learning_hooks: vec![],
        })
        .unwrap();
    let policy = AuditPolicy {
        attempt_timeout_secs: Limit::Unlimited,
        total_time_secs: Limit::Unlimited,
        unexpected_change_refreshes: Limit::Finite(0),
    };
    let audit = AuditRuntime::initialize(store.clone(), run.id.clone(), policy).unwrap();
    (store, run.id, audit)
}

fn report(snapshot: &str, records: serde_json::Value) -> AuditReport {
    serde_json::from_value(json!({"schema_version": 1, "snapshot": snapshot, "records": records}))
        .unwrap()
}

fn contract(snapshot: &str, paths: &[&str]) -> AuditContract {
    AuditContract {
        schema_version: 1,
        snapshot: snapshot.into(),
        declared_paths: paths.iter().map(|path| (*path).to_string()).collect(),
    }
}

/// The live state: `docs/x.md` declared, assessed `absent / deliver`, its
/// obligation open at the assessed snapshot.
fn seed_open_obligation(audit: &AuditRuntime, repo: &Path) {
    audit
        .update(|state| {
            state.declared_paths.insert("docs/x.md".into());
            state.declared_paths.insert("src/y.rs".into());
            state.snapshot = Some(Snapshot {
                identity: "one".into(),
                root: repo.to_path_buf(),
                paths: vec!["src/y.rs".into()],
            });
            state.ledger.accept(
                contract("one", &["docs/x.md", "src/y.rs"]),
                report("one", json!([
                    {"declared_path": "docs/x.md", "verdict": "absent", "equivalents": [],
                     "required_action": "deliver", "reason": "not in the sealed tree"},
                    {"declared_path": "src/y.rs", "verdict": "exists_as_declared", "equivalents": [],
                     "required_action": "none", "reason": "present"},
                ])),
            )
        })
        .unwrap();
}

fn events(store: &WorkflowStore, run_id: &str, event: &str) -> Vec<serde_json::Value> {
    std::fs::read_to_string(store.events_path(run_id))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .filter(|record| record["detail"]["event"] == event)
        .collect()
}

#[test]
fn the_predicate_is_git_check_ignore_and_a_tracked_file_is_never_ignored() {
    let temp = tempfile::tempdir().unwrap();
    let repo = repository(temp.path());
    assert!(ignored::is_ignored(&repo, "docs/x.md"));
    assert!(!ignored::is_ignored(&repo, "src/y.rs"));
    assert!(
        !ignored::is_ignored(&repo, "docs/tracked.md"),
        "tracked files are deliverables"
    );
    let paths = [
        "docs/x.md".to_string(),
        "src/y.rs".into(),
        "docs/tracked.md".into(),
        "docs/new.md".into(),
    ];
    let ignored = ignored::ignored_among(&repo, &paths);
    assert_eq!(
        ignored.into_iter().collect::<Vec<_>>(),
        vec!["docs/new.md".to_string(), "docs/x.md".into()]
    );
    assert_eq!(
        ignored::repository_deliverables(&repo, paths.to_vec()),
        vec!["src/y.rs".to_string(), "docs/tracked.md".into()]
    );
    // Not a repository: nothing is ignored, nothing is filtered.
    let plain = temp.path().join("plain");
    std::fs::create_dir(&plain).unwrap();
    assert!(!ignored::is_ignored(&plain, "docs/x.md"));
    assert!(ignored::ignored_among(&plain, &paths).is_empty());
    assert!(ignored::ignored_among(&repo, &[]).is_empty());
}

/// Existing run state: the open obligation is reclaimed on load, history is
/// untouched, the event names the path, and the next refresh does not read the
/// missing declaration as a silent drop.
#[test]
fn reclaiming_an_ignored_obligation_stops_the_loop_without_touching_history() {
    let temp = tempfile::tempdir().unwrap();
    let repo = repository(temp.path());
    let (store, run_id, audit) = runtime(temp.path());
    seed_open_obligation(&audit, &repo);
    assert_eq!(
        audit.state().unwrap().ledger.unresolved("one").unwrap(),
        vec!["docs/x.md".to_string()]
    );

    assert_eq!(
        audit.reclaim_ignored(&repo).unwrap(),
        vec!["docs/x.md".to_string()]
    );

    let state = audit.state().unwrap();
    assert!(
        !state.declared_paths.contains("docs/x.md"),
        "{:?}",
        state.declared_paths
    );
    assert!(state.declared_paths.contains("src/y.rs"));
    assert!(
        !state.ledger.obligations.contains_key("docs/x.md"),
        "{:?}",
        state.ledger.obligations
    );
    assert!(state.ledger.unresolved("one").unwrap().is_empty());
    assert_eq!(state.ledger.history.len(), 1, "history is append-only");
    assert_eq!(
        state.ledger.history[0].records.len(),
        2,
        "the recorded judgment stays"
    );
    assert!(state.ledger.ignored_paths.contains("docs/x.md"));
    assert_eq!(
        state.status().unwrap()["ignored_paths"],
        json!(["docs/x.md"])
    );
    let dropped = events(&store, &run_id, "repository_audit_ignored_paths_dropped");
    assert_eq!(dropped.len(), 1, "{dropped:#?}");
    assert_eq!(dropped[0]["detail"]["paths"], json!(["docs/x.md"]));
    // A second load finds nothing to reclaim and says nothing.
    assert!(audit.reclaim_ignored(&repo).unwrap().is_empty());
    assert_eq!(
        events(&store, &run_id, "repository_audit_ignored_paths_dropped").len(),
        1
    );
    // The next assessment declares only the repository deliverables.
    audit
        .update(|state| {
            state.ledger.accept(
                contract("two", &["src/y.rs"]),
                report(
                    "two",
                    json!([
            {"declared_path": "src/y.rs", "verdict": "exists_as_declared", "equivalents": [],
             "required_action": "none", "reason": "present"}]),
                ),
            )
        })
        .expect("a reclaimed path is not a silently dropped declaration");
    assert_eq!(audit.state().unwrap().ledger.history.len(), 2);
}

struct Assessor;
#[async_trait::async_trait]
impl WorkflowAgentDispatch for Assessor {
    fn fanout_parallelism(&self, _: Option<usize>) -> usize {
        1
    }
    async fn run_call(
        &self,
        _: &str,
        _: Option<String>,
        e: &WorkflowV2CallExecution,
        a: &WorkflowV2AgentAdapter,
        _: Option<&WorkflowV2ResultStore>,
        _: Option<&task_universe::WorkflowV2TaskUniverse>,
    ) -> WorkflowResult<WorkflowV2Result> {
        let c = &e.call.options.extra["repository_audit_contract"];
        assert!(
            !c["declared_paths"]
                .as_array()
                .unwrap()
                .iter()
                .any(|p| p == "docs/x.md"),
            "{c}"
        );
        let records = c["declared_paths"].as_array().unwrap().iter().map(|p| json!({"declared_path": p,
            "verdict": "exists_as_declared", "equivalents": [], "required_action": "none", "reason": "present"})).collect::<Vec<_>>();
        let output = json!({"status": "accepted", "summary": "audited", "evidence": [{"kind": "inspection", "summary": "read source"}],
            "data": {"repository_audit": {"schema_version": 1, "snapshot": c["snapshot"], "records": records}}});
        a.parse_agent_output(
            &v2::call_data::v2_agent_request("audit", None, e, None),
            &output.to_string(),
        )
        .map_err(|e| WorkflowError::StageFailed(e.to_string()))
    }
}

/// Every path enters the jurisdiction through `assess`; a wave's declaration
/// of an ignored target (`audit_cache::refresh`, `worktree_wave_prepare`) is
/// reclaimed there, against the sealed view's own ignore rules, and cache
/// admission then asks only about the paths the audit owns.
#[tokio::test]
async fn an_ignored_target_declared_by_a_wave_never_becomes_an_obligation() {
    let temp = tempfile::tempdir().unwrap();
    let repo = repository(temp.path());
    let (store, run_id, audit) = runtime(temp.path());
    let v2 = WorkflowV2ResultStore::new(store.run_dir(&run_id).join("v2"));
    let paths = vec!["docs/x.md".to_string(), "src/y.rs".into()];
    let snapshot = Snapshot::capture(&repo, &paths, &v2).unwrap();
    assert!(
        !snapshot.paths.iter().any(|p| p == "docs/x.md"),
        "the sealed view never holds it: {:?}",
        snapshot.paths
    );

    audit
        .assess(&snapshot, &paths, "cache", &Assessor)
        .await
        .unwrap();

    let state = audit.state().unwrap();
    assert_eq!(
        state.declared_paths.iter().cloned().collect::<Vec<_>>(),
        vec!["src/y.rs".to_string()]
    );
    assert!(
        state.ledger.obligations.is_empty(),
        "{:?}",
        state.ledger.obligations
    );
    assert!(state.ledger.ignored_paths.contains("docs/x.md"));
    audit.require_closed(&snapshot.identity).unwrap();
    assert!(
        reuse::admits(&state, &["docs/x.md".to_string()]).unwrap(),
        "an item whose only deliverable is ignored"
    );
    assert!(reuse::admits(&state, &paths).unwrap());
    assert!(
        !reuse::admits(&state, &["docs/x.md".to_string(), "src/other.rs".into()]).unwrap(),
        "unassessed paths are still a miss"
    );
    assert!(
        !reuse::eligible(&state, &["docs/x.md".to_string()]).unwrap(),
        "`eligible` itself is unchanged"
    );
    let started = events(&store, &run_id, "repository_audit_started");
    assert_eq!(
        started[0]["detail"]["added_declared_paths"],
        json!(["src/y.rs"])
    );
    let dropped = events(&store, &run_id, "repository_audit_ignored_paths_dropped");
    assert_eq!(dropped.len(), 1, "{dropped:#?}");
    assert_eq!(dropped[0]["detail"]["paths"], json!(["docs/x.md"]));
    // The same declaration again is the assessed view: no refresh, and the
    // reclaim is reported once, not on every wave that re-declares it.
    audit
        .assess(&snapshot, &paths, "cache", &Assessor)
        .await
        .unwrap();
    assert_eq!(audit.state().unwrap().attempts, 1);
    assert_eq!(
        events(&store, &run_id, "repository_audit_ignored_paths_dropped").len(),
        1
    );
}

//! Issue-223 end to end: stored data is whatever lies under a root the run's
//! own records declare -- inside or outside the repository, relative or
//! absolute -- and its fix is granted to the routed owner on the run's
//! scope-amendment ledger and lands, under the rules `.archon/<namespace>/`
//! data lands by. A path no record declares, a `..` that climbs out of a
//! declared root and a link that points out of one stay refused, and with
//! no declaration only the `.archon/<namespace>/` default is open.
#[path = "support/escalation_harness.rs"]
mod harness;
#[path = "support/write_wave_fixture.rs"]
mod support;

#[path = "support/acceptance_grants_world.rs"]
mod world;

use std::collections::BTreeSet;
use std::path::Path;

use archon_workflow::task_scope_amendment::{
    ScopeAmendment, ScopeAmendmentLedger, ScopeAmendmentRequest, ScopeGrantKind, ScopeGrantRoot,
    amend_task_scope,
};
use harness::{Answer, NEW_PRELUDE, at_head, run};
use support::{Edits, Fixture, git};
use world::{FIX, SCRIPT, STORED, answer, fixture, host_round, project_root, session};

/// A data file under a root only the task set's artifact declaration names.
const LAKE: &str = "lake/data/bars.json";
const LAKE_COPY: &str = "@copy:lake/data/bars.json";
/// A sibling of the declared root no record declares.
const SECRET: &str = "secret/keys.json";
/// Stored data inside the repository, outside `.archon/`.
const IN_REPO: &str = "fixtures/data/bars.json";

/// `f` declares `artifacts` on TASK-A and records `inputs` as the policy's
/// project inputs; the project holds `LAKE` and `SECRET`.
fn declaring(f: &mut Fixture, scratch: &Path, inputs: &[&str], artifacts: Vec<String>) {
    world::with_project_inputs(f, scratch, inputs);
    f.universe.as_mut().unwrap().tasks[0].artifact_requirements = artifacts;
    let project = project_root(f);
    for file in [LAKE, SECRET] {
        std::fs::create_dir_all(project.join(file).parent().unwrap()).unwrap();
        std::fs::write(project.join(file), "{\"close\": 0}\n").unwrap();
    }
}

fn lake_declared(f: &mut Fixture, scratch: &Path) {
    declaring(f, scratch, &[], vec!["lake/data/index.json".into()]);
}

fn failure_naming(paths: &[std::path::PathBuf]) -> String {
    let named: Vec<String> = paths.iter().map(|p| p.display().to_string()).collect();
    format!("AssertionError: {} disagree\n", named.join(" and "))
}

/// Every write grant the run's ledger holds, as `(task, path, root)`.
fn ledger_grants(f: &Fixture) -> BTreeSet<(String, String, ScopeGrantRoot)> {
    let ledger = ScopeAmendmentLedger::load(&f.store.run_dir(&f.run)).unwrap();
    (ledger.set.grants.iter())
        .filter(|g| g.kind.writable())
        .map(|g| (g.task_id.clone(), g.path.clone(), g.root))
        .collect()
}

/// The ledger's answer to TASK-A asking for `path` as project data: `Ok`
/// with the root granted, or the logged refusal.
fn ask_as_data(f: &Fixture, path: &str) -> Result<ScopeGrantRoot, String> {
    let run_root = f.store.run_dir(&f.run);
    let outcome = amend_task_scope(ScopeAmendmentRequest {
        run_root: &run_root,
        universe: f.universe.as_ref().unwrap(),
        repository_root: &f.repo,
        grants: vec![ScopeAmendment {
            task_id: "TASK-A".into(),
            path: path.into(),
            kind: ScopeGrantKind::DeliverableRoot,
            root: ScopeGrantRoot::Project,
            shared_with: BTreeSet::new(),
            evidence: "issue-223 test".into(),
        }],
        trigger: "issue-223 test",
    })
    .unwrap();
    if let Some((_, why)) = outcome.refused.first() {
        let log = std::fs::read_to_string(run_root.join("v2/scope-amendments.jsonl")).unwrap();
        assert!(log.contains(path), "the refusal is logged: {log}");
        return Err(why.clone());
    }
    Ok(outcome.applied[0].root)
}

fn fix_lake(_key: &str, _round: u64, _escalated: bool) -> Edits {
    Edits {
        files: vec![
            ("crates/a/src/lib.rs", "// a, fixed\n"),
            (LAKE_COPY, "{\"close\": 101}\n"),
        ],
        report: vec!["crates/a/src/lib.rs"],
        via_adapter: false,
    }
}

fn fix_in_repo(_key: &str, _round: u64, _escalated: bool) -> Edits {
    Edits {
        files: vec![(IN_REPO, "{\"close\": 101}\n")],
        report: vec![IN_REPO],
        via_adapter: false,
    }
}

#[tokio::test]
async fn a_declared_root_outside_the_repository_is_granted_and_its_fix_lands() {
    let temp = tempfile::tempdir().unwrap();
    let mut f = fixture();
    lake_declared(&mut f, &temp.path().join("scratch"));
    let project = project_root(&f);
    let (record, round) = host_round(&f, &failure_naming(&[project.join(LAKE)]));
    let routing = record.checks[0].routing.clone().expect("routed");
    assert_eq!(
        routing.project_grants.get(LAKE),
        Some(&vec!["TASK-A".to_string()]),
        "{routing:?}"
    );
    assert!(routing.unwritable.is_empty(), "{routing:?}");
    assert!(ledger_grants(&f).contains(&("TASK-A".into(), LAKE.into(), ScopeGrantRoot::Project)));
    let host = session(f, round, fix_lake);
    run(SCRIPT, NEW_PRELUDE, host.clone()).await;
    assert_eq!(
        answer(&host, FIX),
        Some(Answer::Ran),
        "{:#?}",
        host.answers.borrow()
    );
    assert_eq!(
        std::fs::read_to_string(project.join(LAKE)).unwrap(),
        "{\"close\": 101}\n",
        "the fix reached the declared root outside the repository"
    );
    assert!(!host.f.repo.join(LAKE).exists(), "never the repository");
}

#[tokio::test]
async fn a_declared_root_inside_the_repository_outside_archon_is_granted() {
    let temp = tempfile::tempdir().unwrap();
    let mut f = fixture();
    let in_repo = f.repo.join(IN_REPO);
    std::fs::create_dir_all(in_repo.parent().unwrap()).unwrap();
    std::fs::write(&in_repo, "{\"close\": 0}\n").unwrap();
    git(&f.repo, &["add", "."]);
    git(&f.repo, &["commit", "-qm", "fixture data"]);
    // Declared absolute, as a record may name it.
    let index = f
        .repo
        .canonicalize()
        .unwrap()
        .join("fixtures/data/index.json");
    declaring(
        &mut f,
        &temp.path().join("scratch"),
        &[],
        vec![index.display().to_string()],
    );
    let (record, round) = host_round(&f, &failure_naming(std::slice::from_ref(&in_repo)));
    let routing = record.checks[0].routing.clone().expect("routed");
    assert_eq!(
        routing.granted_to.get(IN_REPO),
        Some(&vec!["TASK-A".to_string()]),
        "{routing:?}"
    );
    assert!(routing.granted_files.contains(&IN_REPO.to_string()));
    assert!(routing.unwritable.is_empty(), "{routing:?}");
    assert!(routing.project_grants.is_empty(), "the patch lands it");
    let host = session(f, round, fix_in_repo);
    run(SCRIPT, NEW_PRELUDE, host.clone()).await;
    assert_eq!(answer(&host, FIX), Some(Answer::Ran));
    assert_eq!(at_head(&host.f.repo, IN_REPO), "{\"close\": 101}");
}

#[test]
fn an_undeclared_sibling_of_a_declared_root_is_refused() {
    let temp = tempfile::tempdir().unwrap();
    let mut f = fixture();
    lake_declared(&mut f, &temp.path().join("scratch"));
    let project = project_root(&f);
    let text = failure_naming(&[project.join(LAKE), project.join(SECRET)]);
    let (record, _) = host_round(&f, &text);
    let routing = record.checks[0].routing.clone().expect("routed");
    assert_eq!(
        routing.project_grants.keys().collect::<Vec<_>>(),
        [LAKE],
        "{routing:?}"
    );
    assert!(!ledger_grants(&f).iter().any(|(_, path, _)| path == SECRET));
    let why = ask_as_data(&f, SECRET).unwrap_err();
    assert!(why.contains("data root the run's records declare"), "{why}");
}

#[test]
fn dot_dot_traversal_from_a_declared_root_is_refused() {
    let temp = tempfile::tempdir().unwrap();
    let mut f = fixture();
    // A declaration that climbs out of where it names is no root at all.
    declaring(
        &mut f,
        &temp.path().join("scratch"),
        &[],
        vec![
            "lake/data/index.json".into(),
            "lake/data/../../secret/index.json".into(),
        ],
    );
    let project = project_root(&f);
    let climbed = project.join("lake/data/../../secret/keys.json");
    assert!(climbed.is_file());
    let (record, _) = host_round(&f, &failure_naming(&[climbed]));
    assert!(
        (record.checks[0].routing.iter()).all(|routing| routing.project_grants.is_empty()),
        "{:?}",
        record.checks[0].routing
    );
    assert!(ledger_grants(&f).is_empty());
    assert!(ask_as_data(&f, "lake/data/../../secret/keys.json").is_err());
    assert!(ask_as_data(&f, SECRET).is_err());
}

#[cfg(unix)] // Plants symbolic links, which need privilege on Windows.
#[test]
fn a_symlink_pointing_out_of_a_declared_root_is_refused() {
    let temp = tempfile::tempdir().unwrap();
    let mut f = fixture();
    lake_declared(&mut f, &temp.path().join("scratch"));
    let project = project_root(&f);
    let link = project.join("lake/data/keys.json");
    std::os::unix::fs::symlink(project.join(SECRET), &link).unwrap();
    std::os::unix::fs::symlink(project.join("secret"), project.join("lake/data/vault")).unwrap();
    let text = failure_naming(&[link, project.join("lake/data/vault/keys.json")]);
    let (record, _) = host_round(&f, &text);
    assert!(
        (record.checks[0].routing.iter()).all(|routing| routing.project_grants.is_empty()),
        "{:?}",
        record.checks[0].routing
    );
    assert!(ledger_grants(&f).is_empty());
    for escaping in ["lake/data/keys.json", "lake/data/vault/keys.json"] {
        let why = ask_as_data(&f, escaping).unwrap_err();
        assert!(why.contains("every link resolved"), "{escaping}: {why}");
    }
}

#[test]
fn with_no_declared_root_only_the_archon_namespace_default_is_open() {
    let temp = tempfile::tempdir().unwrap();
    let mut f = fixture();
    declaring(&mut f, &temp.path().join("scratch"), &[], Vec::new());
    let project = project_root(&f);
    let text = failure_naming(&[project.join(LAKE), project.join(STORED)]);
    let (record, _) = host_round(&f, &text);
    assert!(
        (record.checks[0].routing.iter()).all(|routing| routing.project_grants.is_empty()),
        "{:?}",
        record.checks[0].routing
    );
    assert_eq!(ask_as_data(&f, STORED), Ok(ScopeGrantRoot::Project));
    let why = ask_as_data(&f, LAKE).unwrap_err();
    assert!(why.contains("data root the run's records declare"), "{why}");
}

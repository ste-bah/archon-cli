//! Batch K (I1): repository test material is refused at every project-input
//! landing path -- a branch's captured changes and a tracked input a patch
//! landed -- as a HIGH finding, with the refused bytes kept as evidence.
use super::*;

const FIXTURE: &str = "crates/lab/tests/fixtures/daily.csv";
const BARS: &str =
    "date,open,high,low,close,volume\n2026-01-02,1,2,0.5,1.5,100\n2026-01-03,1.5,2.5,1,2,120\n";

/// A repository tracking `FIXTURE` (and, when given, `tracked` files).
fn repository(p: &Project, tracked: &[(&str, &str)]) -> PathBuf {
    let repo = p._dir.path().join("repo");
    std::fs::create_dir_all(repo.join("crates/lab/tests/fixtures")).unwrap();
    git(&repo, &["init", "-q"]);
    std::fs::write(repo.join(FIXTURE), BARS).unwrap();
    for (rel, body) in tracked {
        std::fs::create_dir_all(repo.join(rel).parent().unwrap()).unwrap();
        std::fs::write(repo.join(rel), body).unwrap();
    }
    git(&repo, &["add", "-A"]);
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
    repo
}

fn record() -> super::super::super::ApplyRecord {
    super::super::super::ApplyRecord {
        wave_id: 0,
        started_at: std::time::SystemTime::now(),
        completed_at: std::time::SystemTime::now(),
        items_applied: vec![],
        items_failed: vec![],
        verify_result: None,
        project_input_refusals: vec![],
        fixture_landings: vec![],
    }
}

#[test]
fn a_captured_copy_of_a_tracked_fixture_is_refused_whole_and_kept() {
    let p = project();
    let repo = repository(&p, &[]);
    let landed = ".archon/lab/data/raw/response.csv";
    capture(
        &p,
        "a",
        &[(landed, "absent", Some(BARS)), (REGISTRY, "v1", Some("v2"))],
    );
    let mut rec = record();
    land(&p.run_root, &repo, &manifest("a"), false, &mut rec);
    // Nothing landed: the whole landing is refused, as any refusal is.
    assert_eq!(read(&p, landed), "<absent>");
    assert_eq!(read(&p, REGISTRY), "v1");
    let expected =
        format!("repository test fixture landed as project data: {landed} from {FIXTURE}");
    assert_eq!(rec.fixture_landings.len(), 1, "{:?}", rec.fixture_landings);
    assert!(
        rec.fixture_landings[0].1.starts_with(&expected),
        "{:?}",
        rec.fixture_landings
    );
    assert_eq!(rec.project_input_refusals.len(), 1);
    assert!(rec.project_input_refusals[0].1.contains(&expected));
    let log = run_project_input_landings(&p.run_root).unwrap();
    assert!(log.iter().all(|line| line.outcome == "refused"), "{log:?}");
    let kept = crate::write_coordinator::fixture_provenance::evidence_dir(&p.run_root, "impl", "a");
    assert_eq!(std::fs::read_to_string(kept.join(landed)).unwrap(), BARS);
    // A landing of the project's own data is untouched by the rule.
    capture(&p, "b", &[(REGISTRY, "v1", Some("v2"))]);
    let mut rec = record();
    land(&p.run_root, &repo, &manifest("b"), false, &mut rec);
    assert!(rec.fixture_landings.is_empty() && rec.project_input_refusals.is_empty());
    assert_eq!(read(&p, REGISTRY), "v2");
}

#[test]
fn a_tracked_input_a_patch_made_a_fixture_copy_is_never_synced_to_the_project() {
    let p = project();
    let spec = ".archon/lab/spec.json";
    let repo = repository(&p, &[(spec, "s1")]);
    std::fs::write(p.root.join(spec), "s1").unwrap();
    let base = git(&repo, &["rev-parse", "HEAD"]);
    // The landing rewrote the tracked input as a copy of the fixture.
    std::fs::write(repo.join(spec), BARS).unwrap();
    let mut m = manifest("a");
    m.baseline_commit = base;
    m.changed_files = vec![spec.into()];
    let mut fixtures = Vec::new();
    let refused = sync_tracked(&p.run_root, &repo, &m, &mut fixtures).expect("refused");
    assert!(refused.contains(FIXTURE), "{refused}");
    assert_eq!(read(&p, spec), "s1");
    assert_eq!(fixtures.len(), 1, "{fixtures:?}");
    let log = run_project_input_landings(&p.run_root).unwrap();
    assert_eq!(log.len(), 1);
    assert_eq!(log[0].outcome, "sync_refused");
}

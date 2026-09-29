//! Batch K (I1): a patch whose TRACKED project input is repository test
//! material -- a copy of a fixture already tracked, or of one the same patch
//! adds -- is refused after `git apply` and before anything is recorded:
//! the tree is put back and the landing is a HIGH finding.
use super::*;

const BARS: &str =
    "date,open,high,low,close,volume\n2026-01-02,1,2,0.5,1.5,100\n2026-01-03,1.5,2.5,1,2,120\n";

fn repo_with_inputs() -> tempfile::TempDir {
    let repo = canonical_repo();
    let root = repo.path();
    std::fs::create_dir_all(root.join("tests/fixtures")).unwrap();
    std::fs::create_dir_all(root.join("data")).unwrap();
    std::fs::write(root.join("tests/fixtures/daily.csv"), BARS).unwrap();
    std::fs::write(root.join("data/prices.csv"), "date,close\n").unwrap();
    git(&["add", "-A"], root);
    git(&["commit", "-q", "-m", "inputs"], root);
    let run_root = run_root_of(root);
    std::fs::create_dir_all(&run_root).unwrap();
    crate::write_coordinator::project_inputs::write_test_policy(&run_root, root, &["data"]);
    repo
}

fn land(repo: &Path, declared: &[&str], edits: &[(&str, &str)]) -> ApplyRecord {
    let (m, pre) = prepare(repo, "impl-0", declared, edits);
    let mut pre_by_item = BTreeMap::new();
    pre_by_item.insert(m.item_id.clone(), pre);
    apply_wave(
        repo,
        std::slice::from_ref(&m),
        &pre_by_item,
        0,
        &run_root_of(repo),
        "run1",
        "impl",
    )
    .expect("apply")
}

fn assert_refused(repo: &Path, rec: &ApplyRecord, fixture: &str) {
    assert!(rec.items_applied.is_empty(), "{rec:?}");
    assert_eq!(rec.items_failed.len(), 1, "{rec:?}");
    let expected =
        format!("repository test fixture landed as project data: data/prices.csv from {fixture}");
    assert!(rec.items_failed[0].1.contains(&expected), "{rec:?}");
    assert_eq!(rec.fixture_landings.len(), 1, "{rec:?}");
    // The tree is put back: the input, the code, and any file it created.
    assert_eq!(
        std::fs::read_to_string(repo.join("data/prices.csv")).unwrap(),
        "date,close\n"
    );
    assert_eq!(
        std::fs::read_to_string(repo.join("src/lib.rs")).unwrap(),
        "// original\n"
    );
}

#[test]
fn a_tracked_input_rewritten_as_a_tracked_fixture_is_refused_and_put_back() {
    let repo = repo_with_inputs();
    let rec = land(
        repo.path(),
        &["src/lib.rs", "data/prices.csv"],
        &[("src/lib.rs", "// edited\n"), ("data/prices.csv", BARS)],
    );
    assert_refused(repo.path(), &rec, "tests/fixtures/daily.csv");
}

#[test]
fn a_tracked_input_copied_from_a_fixture_the_same_patch_adds_is_refused() {
    let repo = repo_with_inputs();
    let fresh = format!("{BARS}2026-01-04,2,3,1,2.5,90\n");
    let rec = land(
        repo.path(),
        &["src/lib.rs", "data/prices.csv", "tests/fixtures/fresh.csv"],
        &[
            ("src/lib.rs", "// edited\n"),
            ("data/prices.csv", fresh.as_str()),
            ("tests/fixtures/fresh.csv", fresh.as_str()),
        ],
    );
    assert_refused(repo.path(), &rec, "tests/fixtures/fresh.csv");
    assert!(!repo.path().join("tests/fixtures/fresh.csv").exists());
}

#[test]
fn a_tracked_input_of_real_data_lands() {
    let repo = repo_with_inputs();
    let rec = land(
        repo.path(),
        &["data/prices.csv"],
        &[("data/prices.csv", "date,close\n2026-01-02,1.5\n")],
    );
    assert_eq!(rec.items_applied, vec!["impl-0".to_string()], "{rec:?}");
    assert!(rec.fixture_landings.is_empty());
}

use super::*;

const BARS: &str =
    "date,open,high,low,close,volume\n2026-01-02,1,2,0.5,1.5,100\n2026-01-03,1.5,2.5,1,2,120\n";

fn git(repo: &Path, args: &[&str]) {
    let status = std::process::Command::new("git")
        .current_dir(repo)
        .args(args)
        .status()
        .expect("git runs");
    assert!(status.success(), "git {args:?}");
}

/// A repository tracking a fixture CSV, a test source file and a product
/// data file with the same bytes as nothing else.
fn repo() -> tempfile::TempDir {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    git(root, &["init", "-q"]);
    git(root, &["config", "user.email", "t@example.com"]);
    git(root, &["config", "user.name", "t"]);
    std::fs::create_dir_all(root.join("crates/lab/tests/fixtures")).unwrap();
    std::fs::create_dir_all(root.join("crates/lab/src")).unwrap();
    std::fs::write(root.join("crates/lab/tests/fixtures/daily.csv"), BARS).unwrap();
    std::fs::write(
        root.join("crates/lab/tests/ingest.rs"),
        "#[test] fn t() {}\n",
    )
    .unwrap();
    std::fs::write(root.join("crates/lab/src/lib.rs"), "pub fn f() {}\n").unwrap();
    git(root, &["add", "."]);
    git(root, &["commit", "-qm", "seed"]);
    temp
}

#[test]
fn test_locations_are_recognised_by_segment_and_name() {
    for path in [
        "crates/lab/tests/fixtures/daily.csv",
        "pkg/test/data.json",
        "web/__fixtures__/a.json",
        "spec/fixtures/b.yml",
        "go/testdata/c.txt",
        "src/__snapshots__/x.snap",
        "data/spy_fixture.csv",
        "Tests/Data/y.csv",
    ] {
        assert!(is_test_location(path), "{path}");
    }
    for path in [
        "crates/lab/src/lib.rs",
        ".archon/lab/data/registry.json",
        "docs/testing-guide.md",
        "contest/results.csv",
    ] {
        assert!(!is_test_location(path), "{path}");
    }
}

#[test]
fn a_byte_identical_copy_of_a_tracked_fixture_is_a_hit() {
    let temp = repo();
    let index = FixtureIndex::load(temp.path(), &[]);
    let hits = index.judge(".archon/lab/data/raw/response.csv", BARS.as_bytes());
    assert_eq!(
        hits,
        [FixtureHit {
            landed: ".archon/lab/data/raw/response.csv".into(),
            fixture: "crates/lab/tests/fixtures/daily.csv".into(),
            identical: true,
        }]
    );
    assert!(hits[0].finding().starts_with(
        "repository test fixture landed as project data: .archon/lab/data/raw/response.csv from crates/lab/tests/fixtures/daily.csv"
    ));
    // One byte different is not a copy, and names nothing.
    let edited = BARS.replace("100", "101");
    assert!(index.judge("x.csv", edited.as_bytes()).is_empty());
}

#[test]
fn text_naming_a_tracked_fixture_as_its_source_is_a_hit() {
    let temp = repo();
    let index = FixtureIndex::load(temp.path(), &[]);
    let request = serde_json::json!({
        "fixture": "crates/lab/tests/fixtures/daily.csv", "provider": "manual"
    })
    .to_string();
    let hits = index.judge("raw/request.json", request.as_bytes());
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert!(!hits[0].identical);
    assert_eq!(hits[0].fixture, "crates/lab/tests/fixtures/daily.csv");
    // An absolute path into the checkout names it too.
    let absolute = format!(
        "source: {}/crates/lab/tests/fixtures/daily.csv\n",
        temp.path().display()
    );
    assert_eq!(index.judge("notes.md", absolute.as_bytes()).len(), 1);
}

#[test]
fn naming_a_test_source_file_or_a_bare_file_name_is_not_a_hit() {
    let temp = repo();
    let index = FixtureIndex::load(temp.path(), &[]);
    let audit = "Checked by crates/lab/tests/ingest.rs; see daily.csv for the shape.\n";
    assert!(index.judge("gap-audit.md", audit.as_bytes()).is_empty());
}

#[test]
fn tiny_files_never_count_as_copies() {
    let temp = repo();
    std::fs::write(
        temp.path().join("crates/lab/tests/fixtures/empty.json"),
        "{}",
    )
    .unwrap();
    git(temp.path(), &["add", "."]);
    git(temp.path(), &["commit", "-qm", "empty"]);
    let index = FixtureIndex::load(temp.path(), &[]);
    assert!(index.judge("data/empty.json", b"{}").is_empty());
}

#[test]
fn a_fixture_not_yet_committed_is_recognised_and_project_inputs_are_not_test_material() {
    let temp = repo();
    let fresh = "crates/lab/tests/fixtures/fresh.csv";
    let bytes = format!("{BARS}2026-01-04,2,3,1,2.5,90\n");
    std::fs::write(temp.path().join(fresh), &bytes).unwrap();
    let hits = FixtureIndex::load(temp.path(), &[]).judge("d.csv", bytes.as_bytes());
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(hits[0].fixture, fresh);
    // A link in the working tree is never read.
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(
            "/dev/zero",
            temp.path().join("crates/lab/tests/fixtures/zero"),
        )
        .unwrap();
        assert!(
            FixtureIndex::load(temp.path(), &[])
                .judge("z", &[0u8; 128])
                .is_empty()
        );
    }
    // Under the policy's project inputs, a `tests/` path is the project's data.
    let inputs = [std::path::PathBuf::from("crates/lab/tests")];
    assert!(
        FixtureIndex::load(temp.path(), &inputs)
            .judge("d.csv", BARS.as_bytes())
            .is_empty()
    );
}

#[test]
fn a_directory_that_is_not_a_checkout_judges_nothing() {
    let temp = tempfile::tempdir().unwrap();
    let index = FixtureIndex::load(temp.path(), &[]);
    assert!(index.is_empty());
    assert!(index.judge("a.csv", BARS.as_bytes()).is_empty());
}

#[test]
fn refused_bytes_and_findings_are_kept_as_evidence() {
    let temp = tempfile::tempdir().unwrap();
    let hit = FixtureHit {
        landed: "data/a.csv".into(),
        fixture: "tests/fixtures/a.csv".into(),
        identical: true,
    };
    keep_evidence(
        temp.path(),
        "stage",
        "item-0",
        std::slice::from_ref(&hit),
        &[("data/a.csv".into(), BARS.as_bytes().to_vec())],
    );
    let dir = evidence_dir(temp.path(), "stage", "item-0");
    assert_eq!(
        std::fs::read_to_string(dir.join("data/a.csv")).unwrap(),
        BARS
    );
    let findings = std::fs::read_to_string(dir.join("fixture-findings.json")).unwrap();
    assert!(findings.contains(FIXTURE_FINDING_PREFIX), "{findings}");
}

#[cfg(unix)]
#[test]
fn a_link_into_the_repositorys_test_material_is_that_material() {
    let temp = repo();
    let link = temp.path().join("data.csv");
    std::os::unix::fs::symlink("crates/lab/tests/fixtures/daily.csv", &link).unwrap();
    let index = FixtureIndex::load(temp.path(), &[]);
    let (hits, kept) = index.judge_file("data/data.csv", &link);
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert_eq!(hits[0].fixture, "crates/lab/tests/fixtures/daily.csv");
    assert!(kept.is_none());
}

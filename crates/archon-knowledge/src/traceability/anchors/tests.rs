use std::path::PathBuf;

use super::*;
use crate::traceability::requirements::extract_requirements;

/// A code index made of fixture chunks. No embedder, no indexing, no Cozo —
/// which is the point of the port.
struct FixtureIndex {
    hits: Vec<CodeHit>,
    calls: std::cell::RefCell<Vec<(String, Option<String>)>>,
}

impl FixtureIndex {
    fn new(hits: Vec<CodeHit>) -> Self {
        Self {
            hits,
            calls: std::cell::RefCell::new(Vec::new()),
        }
    }
}

impl CodeSearch for FixtureIndex {
    fn search(
        &self,
        query: &str,
        limit: usize,
        path_pattern: Option<&str>,
    ) -> Result<Vec<CodeHit>> {
        self.calls
            .borrow_mut()
            .push((query.to_string(), path_pattern.map(str::to_string)));
        Ok(self
            .hits
            .iter()
            .filter(|hit| path_pattern.is_none_or(|p| hit.file_path.contains(p)))
            .take(limit)
            .cloned()
            .collect())
    }
}

fn hit(path: &str, start: usize, end: usize, score: f64) -> CodeHit {
    CodeHit {
        file_path: path.to_string(),
        language: "rust".into(),
        line_start: start,
        line_end: end,
        relevance_score: score,
    }
}

fn requirement() -> Requirement {
    extract_requirements("- REQ-DL-034: Ingest OpenBB Polygon natively.\n")
        .pop()
        .expect("one requirement")
}

fn binding_with(scopes: &[&str]) -> TaskBinding {
    TaskBinding {
        task_id: "TASK-TDL-050".into(),
        source_path: "tests/TASK-TDL-050.md".into(),
        implements: vec!["REQ-DL-034".into()],
        path_scopes: scopes.iter().map(|s| s.to_string()).collect(),
        ..TaskBinding::default()
    }
}

/// Write a real file so the anchor can be hashed, and return the repo root.
fn repo_with(files: &[(&str, &str)]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    for (rel, contents) in files {
        let path: PathBuf = dir.path().join(rel);
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        std::fs::write(&path, contents).expect("write");
    }
    dir
}

#[test]
fn anchors_are_scoped_to_declared_paths() {
    let repo = repo_with(&[("src/ingest.rs", "fn ingest() {}\n")]);
    let index = FixtureIndex::new(vec![
        hit("src/ingest.rs", 10, 20, 0.81),
        hit("src/unrelated.rs", 1, 5, 0.99),
    ]);
    let anchors = anchor_requirement(
        &index,
        &requirement(),
        &binding_with(&["src/ingest.rs"]),
        repo.path(),
        5,
        8,
    )
    .expect("search ok")
    .expect("anchored");

    assert_eq!(anchors.len(), 1);
    assert_eq!(anchors[0].citation(), "src/ingest.rs:10-20");
    assert_eq!(anchors[0].path_scope, "src/ingest.rs");
    // The higher-scoring out-of-scope hit lost. Score decides nothing.
    assert!(anchors.iter().all(|a| a.file_path != "src/unrelated.rs"));
}

#[test]
fn one_query_per_declared_scope_carrying_the_requirement_text() {
    let repo = repo_with(&[("a/x.rs", "x\n"), ("b/y.rs", "y\n")]);
    let index = FixtureIndex::new(vec![hit("a/x.rs", 1, 2, 0.5), hit("b/y.rs", 3, 4, 0.5)]);
    let anchors = anchor_requirement(
        &index,
        &requirement(),
        &binding_with(&["a/x.rs", "b/y.rs"]),
        repo.path(),
        5,
        8,
    )
    .expect("search ok")
    .expect("anchored");
    assert_eq!(anchors.len(), 2);

    let calls = index.calls.borrow();
    assert_eq!(calls.len(), 2);
    assert!(
        calls[0].0.starts_with("REQ-DL-034 Ingest"),
        "{:?}",
        calls[0]
    );
    assert_eq!(calls[0].1.as_deref(), Some("a/x.rs"));
    assert_eq!(calls[1].1.as_deref(), Some("b/y.rs"));
}

#[test]
fn max_scopes_caps_the_query_budget() {
    let repo = repo_with(&[("a/x.rs", "x\n")]);
    let index = FixtureIndex::new(vec![hit("a/x.rs", 1, 2, 0.5)]);
    let _ = anchor_requirement(
        &index,
        &requirement(),
        &binding_with(&["a/x.rs", "b/y.rs", "c/z.rs"]),
        repo.path(),
        5,
        1,
    )
    .expect("search ok");
    assert_eq!(index.calls.borrow().len(), 1);
}

#[test]
fn a_task_with_no_declared_paths_yields_a_named_gap_not_a_repo_wide_search() {
    let repo = repo_with(&[("src/ingest.rs", "x\n")]);
    let index = FixtureIndex::new(vec![hit("src/ingest.rs", 1, 2, 0.99)]);
    let gap = anchor_requirement(
        &index,
        &requirement(),
        &binding_with(&[]),
        repo.path(),
        5,
        8,
    )
    .expect("search ok")
    .expect_err("no anchors");
    assert_eq!(
        gap,
        AnchorGap::NoDeclaredPaths {
            task_id: "TASK-TDL-050".into()
        }
    );
    // Crucially: nothing was searched at all.
    assert!(index.calls.borrow().is_empty());
}

#[test]
fn a_hit_whose_file_cannot_be_hashed_is_dropped() {
    let repo = repo_with(&[]);
    let index = FixtureIndex::new(vec![hit("src/ingest.rs", 1, 2, 0.99)]);
    let gap = anchor_requirement(
        &index,
        &requirement(),
        &binding_with(&["src/ingest.rs"]),
        repo.path(),
        5,
        8,
    )
    .expect("search ok")
    .expect_err("no anchors");
    assert_eq!(
        gap,
        AnchorGap::NoHitInScope {
            task_id: "TASK-TDL-050".into()
        }
    );
}

#[test]
fn file_hash_matches_a_known_sha256() {
    // Independently checkable: SHA-256 of the empty input.
    assert_eq!(
        file_hash(b""),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
}

#[test]
fn editing_the_file_makes_the_anchor_stale_rather_than_silently_wrong() {
    let repo = repo_with(&[("src/ingest.rs", "fn ingest() {}\n")]);
    let index = FixtureIndex::new(vec![hit("src/ingest.rs", 1, 1, 0.7)]);
    let anchors = anchor_requirement(
        &index,
        &requirement(),
        &binding_with(&["src/ingest.rs"]),
        repo.path(),
        5,
        8,
    )
    .expect("search ok")
    .expect("anchored");
    let anchor = anchors.into_iter().next().expect("one anchor");
    assert_eq!(
        check_freshness(&anchor, repo.path()),
        AnchorFreshness::Fresh
    );

    std::fs::write(
        repo.path().join("src/ingest.rs"),
        "fn ingest() { todo!() }\n",
    )
    .expect("edit");
    match check_freshness(&anchor, repo.path()) {
        AnchorFreshness::Stale { recorded, current } => {
            assert_eq!(recorded, anchor.file_hash);
            assert_ne!(current, recorded);
        }
        other => panic!("expected stale, got {other:?}"),
    }

    std::fs::remove_file(repo.path().join("src/ingest.rs")).expect("delete");
    assert_eq!(
        check_freshness(&anchor, repo.path()),
        AnchorFreshness::FileMissing
    );
}

#[test]
fn relation_identity_is_stable_and_names_its_citation() {
    let anchor = Anchor {
        requirement_id: "REQ-DL-034".into(),
        task_id: "TASK-TDL-050".into(),
        file_path: "src/ingest.rs".into(),
        line_start: 10,
        line_end: 20,
        file_hash: "abc".into(),
        path_scope: "src/ingest.rs".into(),
        relevance_score: 0.5,
    };
    let a = anchor_relation(&anchor, "req-entity");
    let b = anchor_relation(&anchor, "req-entity");
    assert_eq!(a.relation_id, b.relation_id);
    assert_eq!(a.relation_type, ANCHOR_RELATION_TYPE);
    assert_eq!(a.source_chunk_id, "src/ingest.rs:10-20");
    assert_eq!(a.target_entity_id, "src/ingest.rs:10-20");

    let mut moved = anchor.clone();
    moved.line_start = 11;
    assert_ne!(
        a.relation_id,
        anchor_relation(&moved, "req-entity").relation_id
    );
}

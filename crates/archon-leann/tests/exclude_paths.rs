//! Exclusions are decided inside the repository, not above it (issue #143).
//!
//! `is_excluded` matches path *components*, so it used to be handed the
//! absolute path of every walked entry. A checkout that happened to live under
//! a directory named `target`, `build` or `dist` therefore excluded itself: the
//! walk root failed `filter_entry`, `skip_current_dir` swallowed the rest, and
//! the pass returned `Ok` having indexed nothing at all.
//!
//! The two tests here are a pair on purpose. The first is the bug; the second
//! is the fence around the fix, because "ignore the ancestry" must not become
//! "ignore `target/` entirely".

use std::path::Path;

use cozo::DbInstance;

use archon_leann::indexer::{EmbeddingConfig, EmbeddingProviderKind, Indexer};
use archon_leann::metadata::IndexConfig;

/// A repository nested under a directory literally named `target`, containing
/// its own `target/` build directory.
///
/// Returns the repository root — the path the indexer is pointed at — which is
/// *below* the offending `target` component, exactly as a real checkout at
/// `C:\target\my-project` would be.
fn nested_repo(temp: &Path) -> std::path::PathBuf {
    let root = temp.join("target").join("my-project");
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(
        root.join("src").join("main.rs"),
        "fn main() {\n    println!(\"hello\");\n}\n",
    )
    .unwrap();
    std::fs::write(
        root.join("src").join("lib.rs"),
        "pub fn add(a: i32, b: i32) -> i32 {\n    a + b\n}\n",
    )
    .unwrap();

    // Build output *inside* the repository, which must still be skipped.
    std::fs::create_dir_all(root.join("target").join("debug")).unwrap();
    std::fs::write(
        root.join("target").join("debug").join("generated.rs"),
        "pub fn generated() -> i32 {\n    7\n}\n",
    )
    .unwrap();

    root
}

fn mock_indexer(db: DbInstance) -> Indexer {
    let indexer = Indexer::new(
        db,
        EmbeddingConfig {
            provider: EmbeddingProviderKind::Mock,
            dimension: 8,
        },
        None,
    )
    .expect("indexer creation");
    indexer.ensure_schema().expect("ensure_schema");
    indexer
}

fn index_config(root: &Path) -> IndexConfig {
    IndexConfig {
        root_path: root.to_path_buf(),
        include_patterns: Vec::new(),
        exclude_patterns: Vec::new(),
    }
}

/// Every indexed file, by absolute path, straight out of the store.
///
/// Read back from `code_chunks` rather than trusted from `IndexStats`, because
/// the failure being guarded against is a pass whose *stats* look plausible
/// while the corpus is empty or over-pruned.
fn indexed_files(db: &DbInstance) -> Vec<String> {
    let mut paths = db
        .run_script(
            "?[file_path] := *code_chunks{file_path}",
            Default::default(),
            cozo::ScriptMutability::Immutable,
        )
        .expect("file_path query")
        .rows
        .iter()
        .filter_map(|row| row.first()?.get_str().map(str::to_owned))
        .collect::<Vec<_>>();
    paths.sort();
    paths.dedup();
    paths
}

/// The bug: an ancestor named `target` must not exclude the repository.
#[tokio::test]
async fn a_checkout_below_a_directory_named_target_still_indexes() {
    let temp = tempfile::tempdir().unwrap();
    let root = nested_repo(temp.path());
    let db = DbInstance::new("mem", "", Default::default()).unwrap();
    let indexer = mock_indexer(db.clone());

    let stats = indexer
        .index_repository(&root, &index_config(&root))
        .await
        .expect("index_repository");

    assert_eq!(
        stats.total_files,
        2,
        "src/main.rs and src/lib.rs should both be indexed under {}, got {:?}",
        root.display(),
        indexed_files(&db)
    );
    let files = indexed_files(&db);
    for expected in ["main.rs", "lib.rs"] {
        assert!(
            files.iter().any(|path| path.ends_with(expected)),
            "{expected} missing from the index: {files:?}"
        );
    }
}

/// The fence: a `target/` *inside* the repository is still excluded.
///
/// Without this the obvious over-correction — matching on the relative path but
/// forgetting that the relative path still has components — passes the test
/// above while quietly indexing every build artefact in the tree.
#[tokio::test]
async fn a_target_directory_inside_that_checkout_is_still_excluded() {
    let temp = tempfile::tempdir().unwrap();
    let root = nested_repo(temp.path());
    let db = DbInstance::new("mem", "", Default::default()).unwrap();
    let indexer = mock_indexer(db.clone());

    indexer
        .index_repository(&root, &index_config(&root))
        .await
        .expect("index_repository");

    let files = indexed_files(&db);
    assert!(
        !files.iter().any(|path| path.contains("generated.rs")),
        "target/debug/generated.rs is build output and must stay out: {files:?}"
    );
    assert_eq!(
        files.len(),
        2,
        "only the two source files belong in the index: {files:?}"
    );
}

/// The matcher itself, without a filesystem: ancestry is not consulted.
#[test]
fn is_excluded_under_root_ignores_components_above_the_root() {
    use archon_leann::language::{default_exclude_patterns, is_excluded_under_root};

    let patterns = default_exclude_patterns();
    let root = Path::new("/var/tmp/target/work/repo");

    assert!(!is_excluded_under_root(
        &root.join("src/main.rs"),
        root,
        &patterns
    ));
    assert!(is_excluded_under_root(
        &root.join("target/debug/main.rs"),
        root,
        &patterns
    ));
    assert!(is_excluded_under_root(
        &root.join("node_modules/pkg/index.js"),
        root,
        &patterns
    ));
    // The root itself has no components once made relative, so it survives —
    // this is the entry `filter_entry` used to reject, taking the walk with it.
    assert!(!is_excluded_under_root(root, root, &patterns));
    // Not under the root at all: not this walk's business.
    assert!(!is_excluded_under_root(
        Path::new("/elsewhere/target/x.rs"),
        root,
        &patterns
    ));
}

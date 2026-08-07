//! `IndexConfig::include_patterns` narrows the walk (issue #139).
//!
//! The field was declared, populated by `archon-pipeline`'s runner with three
//! language globs, and read nowhere -- every recognised code language was
//! indexed regardless. These tests pin both halves of the contract: a
//! non-empty list is an additional filter on top of the language check, and an
//! empty list is still "everything the language check accepts".

use std::collections::BTreeMap;
use std::path::Path;

use archon_leann::indexer::{EmbeddingConfig, EmbeddingProviderKind, Indexer};
use archon_leann::metadata::IndexConfig;
use cozo::{DataValue, DbInstance, ScriptMutability};

/// One file per language, nested so `**/` has a directory to match through.
fn create_polyglot_repo(dir: &Path) -> [std::path::PathBuf; 4] {
    let src = dir.join("src");
    std::fs::create_dir_all(&src).unwrap();
    let rs = src.join("main.rs");
    let py = src.join("util.py");
    let ts = src.join("app.ts");
    let go = src.join("server.go");
    std::fs::write(&rs, "fn main() {\n    println!(\"hi\");\n}\n").unwrap();
    std::fs::write(&py, "def greet():\n    return \"hi\"\n").unwrap();
    std::fs::write(
        &ts,
        "export function greet(): string {\n  return \"hi\";\n}\n",
    )
    .unwrap();
    std::fs::write(&go, "package main\n\nfunc main() {\n\tprintln(\"hi\")\n}\n").unwrap();
    [rs, py, ts, go]
}

fn is_indexed(db: &DbInstance, file_path: &Path) -> bool {
    let mut params = BTreeMap::new();
    params.insert(
        "fp".to_string(),
        DataValue::from(file_path.to_string_lossy().as_ref()),
    );
    let result = db
        .run_script(
            "?[chunk_id] := *code_chunks{chunk_id, file_path}, file_path = $fp",
            params,
            ScriptMutability::Immutable,
        )
        .expect("file query");
    !result.rows.is_empty()
}

fn index_with(
    include_patterns: Vec<String>,
) -> (DbInstance, tempfile::TempDir, [std::path::PathBuf; 4]) {
    let db = DbInstance::new("mem", "", Default::default()).expect("in-memory CozoDB");
    let indexer = Indexer::new(
        db.clone(),
        EmbeddingConfig {
            provider: EmbeddingProviderKind::Mock,
            dimension: 8,
        },
        None,
    )
    .expect("indexer creation");
    indexer.ensure_schema().unwrap();

    let tmp = tempfile::tempdir().unwrap();
    let files = create_polyglot_repo(tmp.path());
    let config = IndexConfig {
        root_path: tmp.path().to_path_buf(),
        include_patterns,
        exclude_patterns: vec![],
    };
    let stats = indexer
        .index_repository_blocking(tmp.path(), &config)
        .expect("index_repository");
    assert!(stats.total_chunks > 0, "fixture produced no chunks");
    (db, tmp, files)
}

#[test]
fn include_patterns_exclude_languages_they_do_not_name() {
    let (db, _tmp, [rs, py, ts, go]) = index_with(vec![
        "**/*.rs".to_string(),
        "**/*.py".to_string(),
        "**/*.ts".to_string(),
    ]);

    assert!(is_indexed(&db, &rs), "main.rs matches **/*.rs");
    assert!(is_indexed(&db, &py), "util.py matches **/*.py");
    assert!(is_indexed(&db, &ts), "app.ts matches **/*.ts");
    assert!(
        !is_indexed(&db, &go),
        "server.go matches no include pattern and must not be indexed"
    );
}

#[test]
fn empty_include_patterns_index_every_code_language() {
    let (db, _tmp, [rs, py, ts, go]) = index_with(vec![]);

    for path in [&rs, &py, &ts, &go] {
        assert!(
            is_indexed(&db, path),
            "{} should be indexed when include_patterns is empty",
            path.display()
        );
    }
}

/// `*.rs` has no `**/` prefix, so it only ever matched the bare file name.
/// Callers write both spellings; both have to mean the same thing.
#[test]
fn bare_extension_pattern_matches_nested_files() {
    let (db, _tmp, [rs, _py, _ts, go]) = index_with(vec!["*.rs".to_string()]);

    assert!(is_indexed(&db, &rs), "src/main.rs should match *.rs");
    assert!(!is_indexed(&db, &go), "server.go should not match *.rs");
}

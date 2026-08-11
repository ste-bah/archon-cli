use std::fs;
use std::path::{Path, PathBuf};

use cozo::{DbInstance, ScriptMutability};
use tempfile::TempDir;

use super::{create_kb, knowledge_bases};
use crate::web::WebRuntimePaths;
use crate::web::api::EffectivePolicySummary;
use crate::web::ingest::{WebKbCreateRequest, WebKnowledgeBaseItem};

/// Point both the working directory and the home-scope root at a temporary
/// tree. `home` is derived from the real home directory, so the tests only
/// assert about project-scope rows and knowledge bases they created.
fn runtime_paths(cwd: &Path) -> WebRuntimePaths {
    let archon_home = cwd.join("home-archon");
    WebRuntimePaths {
        cwd: cwd.to_path_buf(),
        archon_home: archon_home.clone(),
        archon_data: archon_home.join("data"),
        memory_db: archon_home.join("data/memory.db"),
        session_db: archon_home.join("data/sessions/sessions.db"),
        session_activity_root: archon_home.join("sessions"),
        world_model_root: archon_home.join("world-model"),
        reasoning_quality_root: archon_home.join("reasoning-quality"),
    }
}

fn store_at(cwd: &Path) -> std::sync::Arc<DbInstance> {
    let path = cwd.join(".archon/archon-data.db");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let db = archon_docs::acquire_docs_db(&path).unwrap();
    archon_docs::schema::ensure_doc_schema(&db).unwrap();
    archon_knowledge::schema::ensure_knowledge_schema(&db).unwrap();
    db
}

fn attach(db: &DbInstance, kb_id: &str, document_id: &str) {
    let script = format!(
        "?[kb_id, document_id, assigned_at] <- [[\"{kb_id}\", \"{document_id}\", \"2026-01-01T00:00:00Z\"]]
         :put doc_kb_memberships {{ kb_id, document_id => assigned_at }}"
    );
    db.run_script(&script, Default::default(), ScriptMutability::Mutable)
        .unwrap();
}

fn make_dir(cwd: &Path, slug: &str) -> PathBuf {
    let dir = cwd.join(".archon/kb").join(slug);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("README.md"), "notes\n").unwrap();
    dir
}

fn names(items: &[WebKnowledgeBaseItem]) -> Vec<String> {
    items.iter().map(|item| item.name.clone()).collect()
}

fn temp() -> TempDir {
    tempfile::tempdir().unwrap()
}

/// `archon kb ingest <path> --kb alpha` writes membership rows and no
/// directory. The tab has to list it anyway.
#[test]
fn a_store_only_knowledge_base_is_listed_without_a_directory() {
    let dir = temp();
    let paths = runtime_paths(dir.path());
    let db = store_at(dir.path());
    attach(&db, "alpha", "doc-1");
    attach(&db, "alpha", "doc-2");

    let mut warnings = Vec::new();
    let items = knowledge_bases(&paths, Some(db.as_ref()), &mut warnings);

    let alpha = items
        .iter()
        .find(|item| item.name == "alpha")
        .expect("a knowledge base that only exists in the store must still be listed");
    assert_eq!(alpha.origin, "db");
    assert_eq!(alpha.documents, 2);
    assert!(alpha.path.is_empty());
    assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");
}

/// Creating from the web must leave the name in the store, because that is
/// what `--kb` matches from the CLI and the TUI.
#[test]
fn a_web_created_knowledge_base_is_registered_for_command_line_use() {
    let dir = temp();
    let paths = runtime_paths(dir.path());
    let db = store_at(dir.path());

    let item = create_kb(
        &paths,
        &WebKbCreateRequest {
            name: "alpha notes".into(),
            scope: "project".into(),
            description: Some("notes".into()),
            confirmed: true,
        },
    )
    .unwrap();

    assert_eq!(item.name, "alpha notes");
    let stored = archon_knowledge::store::list_kbs(&db).unwrap();
    assert_eq!(
        stored
            .iter()
            .map(|row| row.kb_id.as_str())
            .collect::<Vec<_>>(),
        vec!["alpha notes"],
        "the name a caller passes to --kb must be in the store"
    );
}

/// One knowledge base recorded in both places is one row.
#[test]
fn a_knowledge_base_on_both_sides_appears_once() {
    let dir = temp();
    let paths = runtime_paths(dir.path());
    let db = store_at(dir.path());
    make_dir(dir.path(), "alpha");
    attach(&db, "alpha", "doc-1");

    let mut warnings = Vec::new();
    let items = knowledge_bases(&paths, Some(db.as_ref()), &mut warnings);

    assert_eq!(names(&items), vec!["alpha".to_string()]);
    assert_eq!(items[0].origin, "both");
    assert_eq!(items[0].documents, 1);
    assert!(items[0].files > 0, "the directory contents still count");
}

/// The directory is a slug; the stored name is not. `--kb` matches the stored
/// name exactly, so that is the one the row has to show.
#[test]
fn the_stored_name_wins_over_the_directory_slug() {
    let dir = temp();
    let paths = runtime_paths(dir.path());
    let db = store_at(dir.path());
    make_dir(dir.path(), "alpha-notes");
    attach(&db, "alpha notes", "doc-1");

    let mut warnings = Vec::new();
    let items = knowledge_bases(&paths, Some(db.as_ref()), &mut warnings);

    assert_eq!(names(&items), vec!["alpha notes".to_string()]);
    assert_eq!(items[0].origin, "both");
}

/// A store that could not be read must say so. Returning the directory rows
/// alone would report a shorter list as if it were the whole truth.
#[test]
fn an_unreadable_store_warns_instead_of_listing_nothing() {
    let dir = temp();
    let paths = runtime_paths(dir.path());
    make_dir(dir.path(), "alpha");

    let mut warnings = Vec::new();
    let items = knowledge_bases(&paths, None, &mut warnings);

    assert_eq!(names(&items), vec!["alpha".to_string()]);
    assert_eq!(warnings.len(), 1, "unexpected warnings: {warnings:?}");
    assert!(
        warnings[0].contains("not readable"),
        "unexpected warning: {}",
        warnings[0]
    );
}

/// The contrast that makes the previous test mean something: nothing to list
/// and nothing to warn about are different answers.
#[test]
fn an_empty_readable_store_lists_nothing_and_warns_about_nothing() {
    let dir = temp();
    let paths = runtime_paths(dir.path());
    let db = store_at(dir.path());

    let mut warnings = Vec::new();
    let items = knowledge_bases(&paths, Some(db.as_ref()), &mut warnings);

    assert!(items.is_empty());
    assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");
}

/// The whole summary carries the same union the tab lists and the header
/// counts, plus the warnings that explain a short list.
#[test]
fn the_summary_carries_the_union_and_its_warnings() {
    let dir = temp();
    let paths = runtime_paths(dir.path());
    let db = store_at(dir.path());
    make_dir(dir.path(), "beta");
    attach(&db, "alpha", "doc-1");

    let summary =
        super::super::summary(&paths, &EffectivePolicySummary::default_safe(), Vec::new());

    let listed = names(&summary.knowledge_bases);
    assert!(listed.contains(&"alpha".to_string()), "listed: {listed:?}");
    assert!(listed.contains(&"beta".to_string()), "listed: {listed:?}");
    assert!(
        summary.knowledge_base_warnings.is_empty(),
        "unexpected warnings: {:?}",
        summary.knowledge_base_warnings
    );
}

use cozo::{DbInstance, ScriptMutability};

use super::{list_kbs, register_kb};
use crate::schema::ensure_knowledge_schema;

fn store() -> DbInstance {
    let db = DbInstance::new("mem", "", "").unwrap();
    archon_docs::schema::ensure_doc_schema(&db).unwrap();
    ensure_knowledge_schema(&db).unwrap();
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

/// A knowledge base created by attaching documents to a name is enumerable
/// without anything on disk. Before #170 nothing projected these names, so the
/// only way to find one was to have written it down.
#[test]
fn membership_names_are_listed_with_their_document_counts() {
    let db = store();
    attach(&db, "alpha", "doc-1");
    attach(&db, "alpha", "doc-2");
    attach(&db, "beta", "doc-3");

    let rows = list_kbs(&db).unwrap();

    let names: Vec<&str> = rows.iter().map(|row| row.kb_id.as_str()).collect();
    assert_eq!(names, vec!["alpha", "beta"]);
    assert_eq!(rows[0].documents, 2);
    assert_eq!(rows[1].documents, 1);
    assert!(!rows[0].registered);
}

/// A name declared up front has no membership rows at all, so it has to come
/// from the registry or it is invisible until the first document lands.
#[test]
fn declared_name_is_listed_before_any_document_is_attached() {
    let db = store();
    register_kb(&db, "alpha", "project", "notes").unwrap();

    let rows = list_kbs(&db).unwrap();

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].kb_id, "alpha");
    assert_eq!(rows[0].documents, 0);
    assert_eq!(rows[0].scope, "project");
    assert!(rows[0].registered);
}

/// Present in both relations is still one knowledge base.
#[test]
fn a_name_in_both_relations_is_listed_once() {
    let db = store();
    register_kb(&db, "alpha", "project", "notes").unwrap();
    attach(&db, "alpha", "doc-1");

    let rows = list_kbs(&db).unwrap();

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].kb_id, "alpha");
    assert_eq!(rows[0].documents, 1);
    assert!(rows[0].registered);
}

/// Re-declaring a name refreshes it rather than producing a second row.
#[test]
fn re_declaring_a_name_does_not_duplicate_it() {
    let db = store();
    register_kb(&db, "alpha", "project", "notes").unwrap();
    register_kb(&db, "alpha", "home", "revised notes").unwrap();

    let rows = list_kbs(&db).unwrap();

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].scope, "home");
}

/// A store with no knowledge bases reads as an empty list...
#[test]
fn a_store_with_no_knowledge_bases_lists_nothing() {
    let db = store();
    assert!(list_kbs(&db).unwrap().is_empty());
}

/// ...and a store that cannot be read must not produce the same answer. A
/// relation whose shape does not match the query is a read failure, and
/// swallowing it would make an unreadable store indistinguishable from an
/// empty one.
#[test]
fn an_unreadable_relation_is_an_error_not_an_empty_list() {
    let db = DbInstance::new("mem", "", "").unwrap();
    db.run_script(
        ":create doc_kb_memberships { kb_id: String => assigned_at: String }",
        Default::default(),
        ScriptMutability::Mutable,
    )
    .unwrap();

    let error = list_kbs(&db).expect_err("a shape mismatch must not read as no knowledge bases");

    assert!(
        error.to_string().contains("list kb ids failed"),
        "unexpected error: {error}"
    );
}

/// An empty name is not a knowledge base; accepting it would put a row nobody
/// can pass to `--kb` into the listing.
#[test]
fn an_empty_name_is_refused() {
    let db = store();
    assert!(register_kb(&db, "   ", "project", "").is_err());
}

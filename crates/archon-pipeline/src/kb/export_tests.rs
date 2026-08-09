//! Export tests — deterministic, over a real `archon-docs` corpus.

use cozo::DbInstance;

use super::*;

fn docs_db() -> DbInstance {
    let db = DbInstance::new("mem", "", "").unwrap();
    archon_docs::schema::ensure_doc_schema(&db).unwrap();
    db
}

fn ingest(db: &DbInstance, path: &str, content: &str) -> String {
    archon_docs::ingest_text::ingest_text_source(db, path, "text/plain", content)
        .unwrap()
        .document_id
}

/// Fixture covering every group the export renders.
fn populated_db() -> (DbInstance, String) {
    let db = docs_db();
    let source = ingest(&db, "policy.txt", "Retention is thirty days.");
    ingest(
        &db,
        &format!("{SUMMARY_SOURCE_PREFIX}{source}"),
        "Summary of the retention policy.",
    );
    ingest(
        &db,
        &format!("{CONCEPT_SOURCE_PREFIX}retention"),
        "Retention.",
    );
    ingest(&db, &format!("{ANSWER_SOURCE_PREFIX}a1"), "Thirty days.");
    ingest(&db, INDEX_SOURCE_PATH, "Source documents: 1");
    (db, source)
}

#[test]
fn every_node_type_group_survives_the_move_to_documents() {
    let (db, _) = populated_db();

    let documents = collect(&db, &ExportOptions::default()).unwrap();

    let groups: Vec<&str> = documents.iter().map(|d| d.group).collect();
    assert_eq!(
        groups,
        vec!["raw", "compiled", "concepts", "answers", "index"]
    );
}

#[test]
fn frontmatter_reports_memberships_provenance_and_chunk_count() {
    let db = docs_db();
    let source = ingest(&db, "policy.txt", "Retention is thirty days.");
    let summary = ingest(
        &db,
        &format!("{SUMMARY_SOURCE_PREFIX}{source}"),
        "Summary of the retention policy.",
    );
    archon_docs::store::assign_document_to_kb(&db, "team", &summary).unwrap();
    archon_docs::store::insert_provenance_edge(
        &db,
        &archon_docs::models::ProvenanceEdge {
            edge_id: "edge-export-test".into(),
            from_artifact_id: summary.clone(),
            to_artifact_id: source.clone(),
            edge_type: archon_docs::models::ProvenanceEdgeType::DerivedFrom,
            created_at: chrono::Utc::now().to_rfc3339(),
        },
    )
    .unwrap();

    let documents = collect(&db, &ExportOptions::default()).unwrap();
    let compiled = documents.iter().find(|d| d.group == "compiled").unwrap();
    let rendered = render(compiled);

    assert!(rendered.contains("kb: [team]"), "{rendered}");
    assert!(
        rendered.contains(&format!("derived_from: [{source}]")),
        "{rendered}"
    );
    assert!(rendered.contains("chunks: 1"), "{rendered}");
    assert!(rendered.contains("Summary of the retention policy."));
}

/// Derived documents are keyed by a generated ID, so a bare last-segment title
/// renders as `# doc-3f9a…` — unreadable for four of the five groups.
#[test]
fn each_group_gets_a_readable_heading() {
    let (db, source) = populated_db();

    let documents = collect(&db, &ExportOptions::default()).unwrap();
    let heading = |group: &str| {
        let document = documents.iter().find(|d| d.group == group).unwrap();
        render(document)
            .lines()
            .find(|line| line.starts_with("# "))
            .unwrap()
            .to_string()
    };

    assert_eq!(heading("raw"), "# policy.txt");
    assert_eq!(heading("compiled"), format!("# Summary of {source}"));
    assert_eq!(heading("concepts"), "# retention");
    assert_eq!(heading("answers"), "# Filed answer a1");
    assert_eq!(heading("index"), "# Knowledge Base Index");
}

#[test]
fn a_knowledge_base_filter_narrows_the_export() {
    let db = docs_db();
    let inside = ingest(&db, "inside.txt", "Inside the knowledge base.");
    ingest(&db, "outside.txt", "Outside the knowledge base.");
    archon_docs::store::assign_document_to_kb(&db, "team", &inside).unwrap();

    let documents = collect(
        &db,
        &ExportOptions {
            kb: Some("team".into()),
        },
    )
    .unwrap();

    assert_eq!(documents.len(), 1);
    assert_eq!(documents[0].document_id, inside);
}

#[test]
fn directory_export_writes_one_file_per_document_under_its_group() {
    let (db, _) = populated_db();
    let dir = tempfile::tempdir().unwrap();

    let summary = export_to_directory(&db, dir.path(), &ExportOptions::default()).unwrap();

    assert_eq!(summary.total(), 5);
    assert_eq!(summary.raw, 1);
    assert_eq!(summary.compiled, 1);
    assert_eq!(summary.concepts, 1);
    assert_eq!(summary.answers, 1);
    assert_eq!(summary.index, 1);
    for group in GROUPS {
        let entries = std::fs::read_dir(dir.path().join(group)).unwrap().count();
        assert_eq!(entries, 1, "group {group}");
    }
}

#[test]
fn stream_export_groups_documents_under_headings() {
    let (db, _) = populated_db();

    let markdown = export_markdown(&db, &ExportOptions::default()).unwrap();

    assert!(markdown.starts_with("# Knowledge base export"));
    for group in GROUPS {
        assert!(markdown.contains(&format!("## {group} (1)")), "{group}");
    }
}

#[test]
fn an_empty_corpus_says_so_rather_than_producing_a_bare_heading() {
    let db = docs_db();

    let markdown = export_markdown(&db, &ExportOptions::default()).unwrap();

    assert!(markdown.contains("No documents."));
}

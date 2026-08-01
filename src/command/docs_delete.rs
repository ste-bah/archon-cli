//! `archon docs delete <TARGET>` — permanently remove ingested documents.
//!
//! TARGET resolution is shared with `docs reprocess` (document ID, source path, or source path
//! prefix). Unlike reprocess, delete drops the `doc_sources` row, which is what releases the
//! content-hash dedupe registration and lets identical content be ingested again.

use anyhow::Result;
use archon_docs::models::SourceDocument;
use cozo::DbInstance;

pub(crate) fn handle_delete(target: &str, yes: bool) -> Result<()> {
    let db = crate::command::docs_reprocess::open_docs_db()?;
    delete_documents(&db, target, yes)
}

pub(crate) fn delete_documents(db: &DbInstance, target: &str, yes: bool) -> Result<()> {
    let docs = crate::command::docs_reprocess::resolve_target_documents(db, target)?;
    // A prefix that fans out across several documents is the easy way to delete far more than
    // intended, so that case needs an explicit confirmation. A single unambiguous match does not.
    if docs.len() > 1 && !yes {
        anyhow::bail!(
            "target `{target}` matches {} documents; re-run with --yes to delete them all:\n{}",
            docs.len(),
            preview(&docs)
        );
    }

    let mut deleted_chunks = 0usize;
    let mut failed = 0usize;
    for doc in &docs {
        match archon_docs::delete::delete_document(db, &doc.document_id) {
            Ok(result) => {
                deleted_chunks += result.chunks;
                println!(
                    "Deleted: {}  {}  ({} chunk(s), {} page(s), {} vector(s))",
                    result.document_id,
                    result.source_path,
                    result.chunks,
                    result.pages,
                    result.vectors
                );
            }
            Err(err) => {
                failed += 1;
                println!("Failed: {}  {err}", doc.document_id);
            }
        }
    }

    println!(
        "Deleted {} document(s), {deleted_chunks} chunk(s) total",
        docs.len() - failed
    );
    if failed > 0 {
        anyhow::bail!("delete completed with {failed} failed document(s)");
    }
    Ok(())
}

fn preview(docs: &[SourceDocument]) -> String {
    const MAX: usize = 10;
    let mut lines = docs
        .iter()
        .take(MAX)
        .map(|doc| format!("  {}  {}", doc.document_id, doc.source_path))
        .collect::<Vec<_>>();
    if docs.len() > MAX {
        lines.push(format!("  ... and {} more", docs.len() - MAX));
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use std::fs;

    use archon_docs::ingest::ingest_file_with_policy;
    use archon_docs::store;
    use cozo::DbInstance;

    use super::*;

    fn test_db() -> DbInstance {
        let db = DbInstance::new("mem", "", "").unwrap();
        archon_docs::schema::ensure_doc_schema(&db).unwrap();
        db
    }

    /// Exact mode keeps the assertion free of any embedding provider.
    fn search_exact(db: &DbInstance, query: &str) -> Vec<archon_docs::retrieval::SearchResult> {
        archon_docs::retrieval::search_with_mode(
            db,
            query,
            10,
            archon_docs::retrieval::SearchMode::Exact,
            archon_docs::retrieval::RetrievalWeights::default(),
        )
        .unwrap()
        .results
    }

    async fn ingest(db: &DbInstance, path: &std::path::Path, body: &str) -> String {
        fs::write(path, body).unwrap();
        let policy = archon_policy::EffectivePolicy::default();
        ingest_file_with_policy(db, path, &policy)
            .await
            .unwrap()
            .document_id
    }

    #[tokio::test]
    #[serial_test::serial(docs_global_state)]
    async fn delete_by_document_id_removes_the_document() {
        let db = test_db();
        let dir = tempfile::tempdir().unwrap();
        let doc_id = ingest(&db, &dir.path().join("a.md"), "Wave one is impulsive.\n").await;

        delete_documents(&db, &doc_id, false).unwrap();

        assert!(store::get_doc_source(&db, &doc_id).unwrap().is_none());
    }

    /// The motivating bug: a killed ingest leaves the `doc_sources` row (and therefore the
    /// content-hash registration) behind, so re-ingesting the same bytes is skipped as a
    /// duplicate. Delete has to clear that, or the command does not actually unblock anything.
    #[tokio::test]
    #[serial_test::serial(docs_global_state)]
    async fn deleted_content_reingests_as_new_and_search_forgets_it() {
        let db = test_db();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("elliott.md");
        let body = "Wave one starts the impulse and wave two retraces.\n";
        fs::write(&path, body).unwrap();
        let policy = archon_policy::EffectivePolicy::default();

        let first = ingest_file_with_policy(&db, &path, &policy).await.unwrap();
        assert!(first.was_new);
        assert!(!search_exact(&db, "impulse").is_empty());

        // Before the delete, identical bytes are refused as a duplicate.
        let duplicate = ingest_file_with_policy(&db, &path, &policy).await.unwrap();
        assert!(!duplicate.was_new);

        delete_documents(&db, &first.document_id, false).unwrap();

        assert!(
            store::get_doc_source(&db, &first.document_id)
                .unwrap()
                .is_none()
        );
        assert!(
            store::list_chunks_for_doc(&db, &first.document_id)
                .unwrap()
                .is_empty()
        );
        assert!(
            search_exact(&db, "impulse").is_empty(),
            "deleted document still surfaces in search"
        );

        let again = ingest_file_with_policy(&db, &path, &policy).await.unwrap();
        assert!(
            again.was_new,
            "identical content must ingest as new after delete"
        );
        assert_ne!(again.document_id, first.document_id);
        assert!(!search_exact(&db, "impulse").is_empty());
    }

    #[tokio::test]
    #[serial_test::serial(docs_global_state)]
    async fn prefix_delete_of_many_documents_requires_yes() {
        let db = test_db();
        let dir = tempfile::tempdir().unwrap();
        let first = ingest(&db, &dir.path().join("a.md"), "Wave one is impulsive.\n").await;
        let second = ingest(&db, &dir.path().join("b.md"), "Wave two retraces deeply.\n").await;
        let prefix = dir.path().to_string_lossy().to_string();

        let err = delete_documents(&db, &prefix, false).unwrap_err();
        assert!(err.to_string().contains("matches 2 documents"));
        assert!(store::get_doc_source(&db, &first).unwrap().is_some());
        assert!(store::get_doc_source(&db, &second).unwrap().is_some());

        delete_documents(&db, &prefix, true).unwrap();

        assert!(store::get_doc_source(&db, &first).unwrap().is_none());
        assert!(store::get_doc_source(&db, &second).unwrap().is_none());
    }

    #[test]
    fn delete_of_nonexistent_target_errors() {
        let db = test_db();
        let err = delete_documents(&db, "doc-nope", false).unwrap_err();
        assert!(err.to_string().contains("no documents matched target"));
    }
}

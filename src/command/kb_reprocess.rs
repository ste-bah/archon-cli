use anyhow::Result;

pub(crate) async fn handle_reprocess(kb: &str, defer_index: bool) -> Result<()> {
    let result = handle_reprocess_inner(kb, defer_index).await;
    archon_docs::vlm::clear_provider_blocking_safe().await;
    result
}

async fn handle_reprocess_inner(kb: &str, defer_index: bool) -> Result<()> {
    let db = crate::command::docs_reprocess::open_docs_db()?;
    let document_ids = archon_docs::store::list_kb_document_ids(&db, kb)?;
    if document_ids.is_empty() {
        anyhow::bail!("knowledge base `{kb}` has no attached documents");
    }

    let mut docs = Vec::new();
    for document_id in document_ids {
        if let Some(doc) = archon_docs::store::get_doc_source(&db, &document_id)? {
            docs.push(doc);
        }
    }
    let policy = crate::command::docs_reprocess::load_policy();
    let vlm_report =
        archon_docs::vlm::factory::configure_registered_provider_blocking_safe(&policy).await;
    println!("KB: {kb}");
    crate::command::docs_reprocess::reprocess_documents(
        &db,
        &policy,
        &vlm_report,
        &docs,
        "knowledge-base",
        !defer_index,
    )
    .await
}

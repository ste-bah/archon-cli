//! Slash handlers for Evidence Engine TUI inspection views.

use anyhow::Result;
use archon_tui::app::{EvidenceRowPayload, TuiEvent, ViewId};
use cozo::DbInstance;

use crate::command::registry::{CommandContext, CommandHandler};

const DOCS_SUBCOMMANDS: &[&str] = &[
    "open",
    "view",
    "ingest",
    "list",
    "status",
    "show",
    "inspect",
    "chunks",
    "search",
    "answer",
    "provenance",
    "index",
    "model-status",
];

pub(crate) struct DocsViewHandler;
pub(crate) struct LearningViewHandler;

impl CommandHandler for DocsViewHandler {
    fn execute(&self, ctx: &mut CommandContext, args: &[String]) -> Result<()> {
        let subcommand = args.first().map(String::as_str).unwrap_or("open");
        let rest = if args.is_empty() { &[] } else { &args[1..] };

        match subcommand {
            "open" | "view" => emit_docs_event(ctx, open_docs_rows_event),
            "list" => emit_docs_db(ctx, |db| {
                let sources = archon_docs::store::list_doc_sources(db)?;
                Ok(archon_docs::inspect::format_list_output(&sources))
            }),
            "status" => emit_docs_db(ctx, render_docs_status),
            "show" | "inspect" => match rest.first() {
                Some(document_id) => emit_docs_db(ctx, |db| render_doc_inspect(db, document_id)),
                None => emit(ctx, docs_usage_line("show requires <document-id>")),
            },
            "chunks" => match rest.first() {
                Some(document_id) => emit_docs_db(ctx, |db| render_doc_chunks(db, document_id)),
                None => emit(ctx, docs_usage_line("chunks requires <document-id>")),
            },
            "provenance" => match rest.first() {
                Some(artifact_id) => emit_docs_db(ctx, |db| render_doc_provenance(db, artifact_id)),
                None => emit(
                    ctx,
                    docs_usage_line("provenance requires <chunk-or-artifact-id>"),
                ),
            },
            "ingest" | "search" | "answer" | "index" => {
                crate::command::cli_mirror::spawn_cli_mirror(ctx, "docs", args)
            }
            "model-status" => emit_docs_db(ctx, render_docs_model_status),
            "help" => emit(ctx, docs_usage()),
            other => emit(
                ctx,
                docs_usage_line(&format!("unknown subcommand `{other}`")),
            ),
        }
    }

    fn description(&self) -> &str {
        "Open and inspect the document/evidence browser"
    }
}

impl CommandHandler for LearningViewHandler {
    fn execute(&self, ctx: &mut CommandContext, args: &[String]) -> Result<()> {
        if matches!(
            args.iter()
                .map(String::as_str)
                .collect::<Vec<_>>()
                .as_slice(),
            ["gnn", "status"]
        ) {
            let config = archon_core::config::load_config().unwrap_or_default();
            let live = ctx.auto_trainer.as_ref().map(|at| at.status());
            let durable = ctx.memory.as_ref().and_then(|memory| {
                crate::command::learning::gnn::durable_memory_stats(memory.as_ref())
            });
            return emit(
                ctx,
                crate::command::learning::gnn::render_gnn_status_with_durable(
                    &config,
                    live.as_ref(),
                    durable,
                ),
            );
        }
        if matches!(
            args.first().map(String::as_str),
            None | Some("open" | "view")
        ) {
            let event = match ctx.cozo_db.as_ref() {
                Some(db) => open_learning_rows_event(db.as_ref())?,
                None => {
                    let db = open_learning_db()?;
                    open_learning_rows_event(&db)?
                }
            };
            ctx.emit(event);
            return Ok(());
        }
        ctx.emit(TuiEvent::TextDelta(
            "Usage: /learning [open|view] | /learning gnn status\nOpens the governed-learning TUI browser or reports GNN auto-trainer status.".into(),
        ));
        Ok(())
    }

    fn description(&self) -> &str {
        "Open the governed-learning TUI browser"
    }
}

fn emit_docs_db<F>(ctx: &mut CommandContext, render: F) -> Result<()>
where
    F: FnOnce(&DbInstance) -> Result<String>,
{
    let rendered = match ctx.cozo_db.as_ref() {
        Some(db) => render(db.as_ref())?,
        None => {
            let db = open_docs_db()?;
            render(&db)?
        }
    };
    emit(ctx, rendered)
}

fn emit_docs_event<F>(ctx: &mut CommandContext, render: F) -> Result<()>
where
    F: FnOnce(&DbInstance) -> Result<TuiEvent>,
{
    let event = match ctx.cozo_db.as_ref() {
        Some(db) => render(db.as_ref())?,
        None => {
            let db = open_docs_db()?;
            render(&db)?
        }
    };
    ctx.emit(event);
    Ok(())
}

fn open_docs_rows_event(db: &DbInstance) -> Result<TuiEvent> {
    archon_docs::schema::ensure_doc_schema(db)?;
    let rows = archon_docs::store::list_doc_sources(db)?
        .into_iter()
        .map(|source| EvidenceRowPayload {
            id: source.document_id,
            title: source.source_path,
            status: format!("{:?}", source.status),
            detail: format!("{} {}", source.media_type, source.content_hash),
        })
        .collect();
    Ok(TuiEvent::OpenViewRows {
        view_id: ViewId::Docs,
        rows,
    })
}

fn open_learning_rows_event(db: &DbInstance) -> Result<TuiEvent> {
    archon_learning::schema::ensure_learning_schema(db)?;
    let rows = archon_learning::store::list_behaviour_proposals(db, None)?
        .into_iter()
        .map(|proposal| EvidenceRowPayload {
            id: proposal.proposal_id,
            title: format!("{:?}", proposal.manifest_kind),
            status: format!("{:?}", proposal.status),
            detail: format!(
                "{:?} {:?} {}",
                proposal.risk_level, proposal.policy_decision, proposal.diff
            ),
        })
        .collect();
    Ok(TuiEvent::OpenViewRows {
        view_id: ViewId::Learning,
        rows,
    })
}

fn emit(ctx: &mut CommandContext, msg: String) -> Result<()> {
    ctx.emit(TuiEvent::TextDelta(msg));
    Ok(())
}

fn docs_usage() -> String {
    format!(
        "/docs subcommands: {}\n\nUsage:\n  /docs open\n  /docs ingest <path>\n  /docs reprocess <document-id-or-path-prefix>\n  /docs list\n  /docs status\n  /docs show <document-id>\n  /docs inspect <document-id>\n  /docs chunks <document-id>\n  /docs search <query> [--mode hybrid|exact|semantic] [--debug]\n  /docs answer <question>\n  /docs provenance <chunk-or-artifact-id>\n  /docs index [--all]\n  /docs model-status\n",
        DOCS_SUBCOMMANDS.join(", ")
    )
}

fn docs_usage_line(reason: &str) -> String {
    format!("{reason}\n\n{}", docs_usage())
}

fn open_docs_db() -> Result<DbInstance> {
    let db = crate::command::store_paths::open_evidence_db("document", &["ARCHON_DOCS_DB_PATH"])?;
    archon_docs::schema::ensure_doc_schema(&db)?;
    Ok(db)
}

fn open_learning_db() -> Result<DbInstance> {
    let db =
        crate::command::store_paths::open_evidence_db("learning", &["ARCHON_LEARNING_DB_PATH"])?;
    archon_learning::schema::ensure_learning_schema(&db)?;
    Ok(db)
}

fn render_docs_status(db: &DbInstance) -> Result<String> {
    let summary = archon_docs::status::get_status_summary(db)?;
    Ok(format!(
        "Document Status\n===============\nTotal sources: {}\nProcessed:     {}\nFailed:        {}\nTotal chunks:  {}\nTotal pages:   {}\nPDF images:    {} extracted, {} filtered, {} rendered\nPDF OCR:       {} run(s), {} failed\nPDF VLM:       {} description(s), {} failed\n",
        summary.total_sources,
        summary.processed,
        summary.failed,
        summary.total_chunks,
        summary.total_pages,
        summary.pdf_embedded_images_extracted,
        summary.pdf_embedded_images_skipped_filter,
        summary.pdf_pages_rendered,
        summary.pdf_image_ocr_runs,
        summary.pdf_image_ocr_failures,
        summary.pdf_image_vlm_descriptions,
        summary.pdf_image_vlm_failures
    ))
}

fn render_doc_inspect(db: &DbInstance, document_id: &str) -> Result<String> {
    let output = archon_docs::inspect::inspect_document(db, document_id)?;
    Ok(archon_docs::inspect::format_inspect_output(&output))
}

fn render_doc_chunks(db: &DbInstance, document_id: &str) -> Result<String> {
    let chunks = archon_docs::store::list_chunks_for_doc(db, document_id)?;
    if chunks.is_empty() {
        return Ok(format!("No chunks for document {document_id}"));
    }

    let mut out = format!("{} chunk(s) for document {document_id}:\n", chunks.len());
    for chunk in chunks {
        out.push_str(&format!(
            "  {} pages {}-{} embed={}\n",
            chunk.chunk_id, chunk.page_start, chunk.page_end, chunk.embedding_status
        ));
    }
    Ok(out)
}

fn render_doc_provenance(db: &DbInstance, artifact_id: &str) -> Result<String> {
    let outgoing = archon_docs::store::list_provenance_from(db, artifact_id).unwrap_or_default();
    let incoming = archon_docs::store::list_provenance_to(db, artifact_id).unwrap_or_default();
    if outgoing.is_empty() && incoming.is_empty() {
        return Ok(format!("No provenance edges found for {artifact_id}"));
    }

    let mut out = format!("Provenance for {artifact_id}\n====================\n");
    for edge in outgoing {
        out.push_str(&format!(
            "  {} {:?} -> {}\n",
            edge.edge_id, edge.edge_type, edge.to_artifact_id
        ));
    }
    for edge in incoming {
        out.push_str(&format!(
            "  {} {:?} <- {}\n",
            edge.edge_id, edge.edge_type, edge.from_artifact_id
        ));
    }
    Ok(out)
}

fn render_docs_model_status(db: &DbInstance) -> Result<String> {
    let provider = archon_docs::embed::get_provider();
    let vectors = archon_docs::store::count_embeddings(db).unwrap_or(0);
    let pending = archon_docs::store::count_pending_chunks(db).unwrap_or(0);
    let backend = provider
        .as_ref()
        .map(|p| p.backend_name().to_string())
        .unwrap_or_else(|| "not configured".to_string());
    Ok(format!(
        "Document Model Status\n=====================\nBackend: {backend}\nVectors: {vectors}\nPending chunks: {pending}\n"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::registry::default_registry;
    use crate::command::test_support::{CtxBuilder, drain_tui_events};
    use archon_docs::models::{ChunkArtifact, DocumentStatus, SourceDocument};

    #[test]
    fn default_registry_registers_evidence_view_primaries() {
        let registry = default_registry();
        assert!(registry.is_primary("docs"));
        assert!(registry.is_primary("learning"));
    }

    #[test]
    fn docs_usage_lists_prd_command_family() {
        let (mut ctx, mut rx) = CtxBuilder::new().build();
        DocsViewHandler
            .execute(&mut ctx, &[String::from("help")])
            .unwrap();
        let events = drain_tui_events(&mut rx);
        let text = match &events[0] {
            TuiEvent::TextDelta(text) => text,
            other => panic!("expected TextDelta, got {other:?}"),
        };
        for subcommand in DOCS_SUBCOMMANDS {
            assert!(text.contains(subcommand), "missing {subcommand}");
        }
    }

    #[test]
    fn docs_view_handler_opens_rows_from_cozo_source_of_truth() {
        let db = std::sync::Arc::new(test_docs_db());
        seed_doc(&db);
        let (mut ctx, mut rx) = CtxBuilder::new().with_cozo_db(db).build();
        DocsViewHandler.execute(&mut ctx, &[]).unwrap();
        let events = drain_tui_events(&mut rx);
        let [TuiEvent::OpenViewRows { view_id, rows }] = events.as_slice() else {
            panic!("expected OpenViewRows, got {events:?}");
        };
        assert_eq!(*view_id, ViewId::Docs);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, "doc-slash");
        assert!(rows[0].detail.contains("hash-slash"));
    }

    #[test]
    fn learning_view_handler_opens_rows_from_cozo_source_of_truth() {
        let db = std::sync::Arc::new(test_learning_db());
        seed_learning_proposal(&db);
        let (mut ctx, mut rx) = CtxBuilder::new().with_cozo_db(db).build();
        LearningViewHandler.execute(&mut ctx, &[]).unwrap();
        let events = drain_tui_events(&mut rx);
        let [TuiEvent::OpenViewRows { view_id, rows }] = events.as_slice() else {
            panic!("expected OpenViewRows, got {events:?}");
        };
        assert_eq!(*view_id, ViewId::Learning);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, "proposal-slash");
        assert_eq!(rows[0].status, "Pending");
    }

    #[test]
    fn docs_status_reads_document_cozo_source_of_truth() {
        let db = std::sync::Arc::new(test_docs_db());
        seed_doc(&db);

        let (mut ctx, mut rx) = CtxBuilder::new().with_cozo_db(db).build();
        DocsViewHandler
            .execute(&mut ctx, &[String::from("status")])
            .unwrap();
        let events = drain_tui_events(&mut rx);
        let text = match &events[0] {
            TuiEvent::TextDelta(text) => text,
            other => panic!("expected TextDelta, got {other:?}"),
        };

        assert!(text.contains("Total sources: 1"));
        assert!(text.contains("Processed:     1"));
        assert!(text.contains("Total chunks:  1"));
    }

    #[test]
    fn docs_chunks_reads_document_cozo_source_of_truth() {
        let db = std::sync::Arc::new(test_docs_db());
        seed_doc(&db);

        let (mut ctx, mut rx) = CtxBuilder::new().with_cozo_db(db).build();
        DocsViewHandler
            .execute(
                &mut ctx,
                &[String::from("chunks"), String::from("doc-slash")],
            )
            .unwrap();
        let events = drain_tui_events(&mut rx);
        let text = match &events[0] {
            TuiEvent::TextDelta(text) => text,
            other => panic!("expected TextDelta, got {other:?}"),
        };

        assert!(text.contains("chunk-slash"));
        assert!(text.contains("pages 1-1"));
    }

    fn test_docs_db() -> DbInstance {
        let path = format!("/tmp/test-docs-slash-{}.db", uuid::Uuid::new_v4());
        let db = DbInstance::new("sqlite", &path, "").unwrap();
        archon_docs::schema::ensure_doc_schema(&db).unwrap();
        db
    }

    fn test_learning_db() -> DbInstance {
        let path = format!("/tmp/test-learning-slash-{}.db", uuid::Uuid::new_v4());
        let db = DbInstance::new("sqlite", &path, "").unwrap();
        archon_learning::schema::ensure_learning_schema(&db).unwrap();
        db
    }

    fn seed_learning_proposal(db: &DbInstance) {
        archon_learning::store::insert_behaviour_proposal(
            db,
            &archon_learning::models::BehaviourProposal {
                proposal_id: "proposal-slash".into(),
                workspace_id: "workspace-slash".into(),
                manifest_kind: archon_learning::models::BehaviourManifestKind::RetrievalProfile,
                current_version: "v1".into(),
                proposed_version: "v2".into(),
                diff: "increase exact-search weight".into(),
                evidence_ids: vec!["le-1".into()],
                risk_level: archon_learning::models::RiskLevel::Low,
                policy_decision: archon_learning::models::PolicyDecision::PendingApproval,
                status: archon_learning::models::ProposalStatus::Pending,
                created_at: "2026-05-04T00:00:00Z".into(),
            },
        )
        .unwrap();
    }

    fn seed_doc(db: &DbInstance) {
        archon_docs::store::insert_doc_source(
            db,
            &SourceDocument {
                document_id: "doc-slash".into(),
                source_path: "/tmp/slash.md".into(),
                media_type: "text/markdown".into(),
                content_hash: "hash-slash".into(),
                discovered_at: "2026-05-04T00:00:00Z".into(),
                status: DocumentStatus::Processed,
            },
        )
        .unwrap();
        archon_docs::store::insert_chunk(
            db,
            &ChunkArtifact {
                chunk_id: "chunk-slash".into(),
                document_id: "doc-slash".into(),
                artifact_id: "artifact-slash".into(),
                chunk_index: 0,
                page_start: 1,
                page_end: 1,
                content: "slash source of truth content".into(),
                content_hash: "chunk-hash".into(),
                embedding_status: "pending".into(),
            },
        )
        .unwrap();
    }
}

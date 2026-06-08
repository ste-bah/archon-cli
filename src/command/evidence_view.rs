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
    "index-status",
    "index-retry-failed",
    "index-pause",
    "index-resume",
    "index-cancel",
    "index-daemon",
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
            "ingest" | "search" | "answer" | "index" | "index-retry-failed" | "index-pause"
            | "index-resume" | "index-cancel" | "index-daemon" => {
                crate::command::cli_mirror::spawn_cli_mirror(ctx, "docs", args)
            }
            "index-status" => emit_docs_db(ctx, render_docs_index_status),
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
            let db = open_learning_db()?;
            let event = open_learning_rows_event(&db)?;
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
    let db = open_docs_db()?;
    let rendered = render(&db)?;
    emit(ctx, rendered)
}

fn emit_docs_event<F>(ctx: &mut CommandContext, render: F) -> Result<()>
where
    F: FnOnce(&DbInstance) -> Result<TuiEvent>,
{
    let db = open_docs_db()?;
    let event = render(&db)?;
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
        "/docs subcommands: {}\n\nUsage:\n  /docs open\n  /docs ingest <path>\n  /docs reprocess <document-id-or-path-prefix> [--defer-index]\n  /docs list\n  /docs status\n  /docs show <document-id>\n  /docs inspect <document-id>\n  /docs chunks <document-id>\n  /docs search <query> [--mode hybrid|exact|semantic] [--debug]\n  /docs answer <question>\n  /docs provenance <chunk-or-artifact-id>\n  /docs index [--all] [--document <id>] [--batch-size <n>] [--limit <n>]\n  /docs index-status\n  /docs index-retry-failed [--limit <n>]\n  /docs index-pause|index-resume|index-cancel <job-id>\n  /docs index-daemon start|stop|status\n  /docs vector-status\n  /docs vector-migrate [--limit <n>] [--batch-size <n>] [--after <chunk-id>]\n  /docs vector-compact [--provider <name>] [--dimension <n>] [--limit <n>]\n  /docs model-status\n",
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
    let db = crate::command::store_paths::open_learning_db("learning")?;
    archon_learning::schema::ensure_learning_schema(&db)?;
    Ok(db)
}

fn render_docs_status(db: &DbInstance) -> Result<String> {
    let summary = archon_docs::status::get_status_summary(db)?;
    let queue = render_queue_summary(db);
    Ok(format!(
        "Document Status\n===============\nTotal sources: {}\nProcessed:     {}\nFailed:        {}\nTotal chunks:  {}\nTotal pages:   {}\nPDF images:    {} extracted, {} filtered, {} rendered\nPDF OCR:       {} run(s), {} failed\nPDF VLM:       {} description(s), {} failed\n{queue}",
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

fn render_docs_index_status(db: &DbInstance) -> Result<String> {
    Ok(format!(
        "Document Index Queue\n====================\n{}",
        render_queue_summary(db)
    ))
}

fn render_queue_summary(db: &DbInstance) -> String {
    let queue = match archon_docs::index_queue::stats(db) {
        Ok(queue) => format!(
            "Index queue:   {} pending, {} leased, {} indexed, {} failed\n",
            queue.pending, queue.leased, queue.indexed, queue.failed
        ),
        Err(e) => format!("Index queue:   unavailable — {e}\n"),
    };
    let jobs = match archon_docs::index_jobs::summary(db) {
        Ok(jobs) => format!(
            "Index jobs:    {} running, {} completed, {} failed\n",
            jobs.running, jobs.completed, jobs.failed
        ),
        Err(e) => format!("Index jobs:    unavailable — {e}\n"),
    };
    format!("{queue}{jobs}")
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
mod tests;

//! Dispatch of `archon docs` subcommands.

use crate::cli_args::{
    DocsAction, DocsAnswerArgs, DocsChunksArgs, DocsCompileArgs, DocsDeleteArgs, DocsExportArgs,
    DocsIndexArgs, DocsIndexCancelArgs, DocsIndexPauseArgs, DocsIndexResumeArgs,
    DocsIndexRetryFailedArgs, DocsIngestArgs, DocsInspectArgs, DocsProvenanceArgs,
    DocsReprocessArgs, DocsSearchArgs, DocsSearchImagesArgs, DocsShowArgs, DocsVectorCompactArgs,
    DocsVectorMigrateArgs, DocsVerifyIntegrityArgs, DocsVerifyQuoteArgs,
};

use super::*;

pub async fn handle_docs_command(
    action: DocsAction,
    config: &archon_core::config::ArchonConfig,
    env_vars: &archon_core::env_vars::ArchonEnvVars,
) -> Result<()> {
    match action {
        DocsAction::Ingest(DocsIngestArgs { path, yes, jobs }) => {
            handle_ingest(&path, yes, jobs.as_deref()).await
        }
        DocsAction::Reprocess(DocsReprocessArgs {
            target,
            defer_index,
        }) => crate::command::docs_reprocess::handle_reprocess(&target, defer_index).await,
        DocsAction::Delete(DocsDeleteArgs { target, yes }) => {
            crate::command::docs_delete::handle_delete(&target, yes)
        }
        DocsAction::List => handle_list().await,
        DocsAction::Show(DocsShowArgs { document_id }) => handle_show(&document_id).await,
        DocsAction::Status => crate::command::docs_status::handle_status(open_db()?).await,
        DocsAction::Chunks(DocsChunksArgs { document_id }) => handle_chunks(&document_id).await,
        DocsAction::Inspect(DocsInspectArgs { document_id }) => handle_inspect(&document_id).await,
        DocsAction::Search(DocsSearchArgs { query, mode, debug }) => {
            handle_search(&query, &mode, debug).await
        }
        DocsAction::SearchImages(DocsSearchImagesArgs { query, limit }) => {
            handle_search_images(&query, limit).await
        }
        DocsAction::Compile(DocsCompileArgs { kb, model }) => {
            crate::command::docs_compile::handle_compile(config, env_vars, kb, model).await
        }
        DocsAction::Export(DocsExportArgs { out, kb }) => {
            crate::command::docs_compile::handle_export(out.as_deref(), kb)
        }
        DocsAction::Answer(DocsAnswerArgs {
            query,
            no_synthesis,
            file,
            kb,
            limit,
            mode,
            model,
        }) => {
            crate::command::docs_answer::handle_answer(
                config,
                env_vars,
                &query,
                no_synthesis,
                file,
                kb,
                limit,
                &mode,
                model,
            )
            .await
        }
        DocsAction::Provenance(DocsProvenanceArgs { chunk_or_answer_id }) => {
            handle_provenance(&chunk_or_answer_id).await
        }
        DocsAction::Index(DocsIndexArgs {
            all,
            document,
            batch_size,
            limit,
        }) => {
            crate::command::docs_index::handle_index(all, document, batch_size, limit, open_db()?)
                .await
        }
        DocsAction::IndexStatus => crate::command::docs_index::handle_index_status(open_db()?),
        DocsAction::IndexRetryFailed(DocsIndexRetryFailedArgs { limit }) => {
            crate::command::docs_index::handle_index_retry_failed(open_db()?, limit)
        }
        DocsAction::IndexPause(DocsIndexPauseArgs { job_id }) => {
            crate::command::docs_index::handle_index_pause(open_db()?, &job_id)
        }
        DocsAction::IndexResume(DocsIndexResumeArgs { job_id }) => {
            crate::command::docs_index::handle_index_resume(open_db()?, &job_id)
        }
        DocsAction::IndexCancel(DocsIndexCancelArgs { job_id }) => {
            crate::command::docs_index::handle_index_cancel(open_db()?, &job_id)
        }
        DocsAction::IndexDaemon { action } => {
            crate::command::docs_index_daemon::handle_index_daemon(action).await
        }
        DocsAction::VectorStatus => crate::command::docs_vector::handle_vector_status(open_db()?),
        DocsAction::VectorMigrate(DocsVectorMigrateArgs {
            limit,
            batch_size,
            after,
        }) => {
            crate::command::docs_vector::handle_vector_migrate(open_db()?, limit, batch_size, after)
        }
        DocsAction::VectorCompact(DocsVectorCompactArgs {
            provider,
            dimension,
            limit,
        }) => crate::command::docs_vector::handle_vector_compact(
            open_db()?,
            provider,
            dimension,
            limit,
        ),
        DocsAction::ModelStatus => {
            crate::command::docs_embedding::handle_model_status(open_db()?).await
        }
        DocsAction::VerifyQuote(DocsVerifyQuoteArgs {
            quote,
            doc,
            limit,
            json,
        }) => handle_verify_quote(&quote, doc.as_deref(), limit, json).await,
        DocsAction::VerifyIntegrity(DocsVerifyIntegrityArgs { doc, json }) => {
            handle_verify_integrity(doc.as_deref(), json).await
        }
    }
}

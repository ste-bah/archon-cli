//! `archon docs answer` — grounded Q&A over the document corpus.
//!
//! # One capability, one command
//!
//! `PRD-ARCHON-DOCS-001` specifies this as REQ-DOCS-013 (answer from retrieved
//! context), REQ-DOCS-014 (citations) and REQ-DOCS-015 (say so when the
//! evidence is insufficient). `REQ-KB-003` specifies the same capability again
//! and adds LLM synthesis and answer filing. Rather than ship a second answer
//! verb, this handler folds those two additions into the existing command.
//!
//! # Which path runs
//!
//! With a provider configured, `archon_pipeline::kb::query::QueryEngine`
//! synthesizes the answer and can file it back as a searchable document. With
//! no provider — or with `--no-synthesis` — the existing extractive path in
//! `archon_docs::answer` runs unchanged. The fallback is kept rather than
//! removed because an operator with no model configured still deserves cited
//! evidence, and REQ-DOCS-015 is satisfied by both paths.

use anyhow::Result;
use archon_core::config::ArchonConfig;
use archon_core::env_vars::ArchonEnvVars;
use archon_docs::retrieval::SearchMode;
use archon_pipeline::kb::query::{QaQueryOptions, QueryEngine};
use archon_pipeline::llm_adapter::KbProviderClient;

/// Build the provider-backed knowledge-base client, or `None` when no provider
/// can be resolved.
pub(crate) async fn build_kb_client(
    config: &ArchonConfig,
    env_vars: &ArchonEnvVars,
    model: Option<String>,
) -> Option<KbProviderClient> {
    match crate::runtime::llm::build_configured_llm_provider(config, env_vars, "kb").await {
        Ok(provider) => {
            let model = model.unwrap_or_else(|| config.api.default_model.clone());
            Some(KbProviderClient::new(provider, model))
        }
        Err(error) => {
            tracing::warn!("LLM provider unavailable for knowledge-base work: {error}");
            None
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn handle_answer(
    config: &ArchonConfig,
    env_vars: &ArchonEnvVars,
    query: &str,
    no_synthesis: bool,
    file: bool,
    kb: Option<String>,
    limit: usize,
    mode: &str,
    model: Option<String>,
) -> Result<()> {
    let synthesizer = if no_synthesis {
        None
    } else {
        build_kb_client(config, env_vars, model).await
    };

    let Some(synthesizer) = synthesizer else {
        if file {
            // Silently not filing would look identical to filing and failing.
            eprintln!("Warning: --file needs an LLM provider; the extractive answer is not filed.");
        }
        if !no_synthesis {
            eprintln!(
                "Warning: no LLM provider configured; returning the extractive answer. \
                 Run `archon auth` to enable synthesis."
            );
        }
        return crate::command::docs::handle_answer(query).await;
    };

    let db = crate::command::docs::open_db()?;
    let engine = QueryEngine::new(db).with_synthesizer(Box::new(synthesizer));
    let options = QaQueryOptions {
        top_k: limit,
        file_answer: file,
        include_derived_context: true,
        mode: SearchMode::parse(mode)?,
        kb,
    };

    let result = engine.query(query, &options).await?;

    for warning in &result.warnings {
        eprintln!("Warning: {warning}");
    }
    println!("{}\n", result.answer);
    if result.sources.is_empty() {
        // REQ-DOCS-015: an answer with no evidence must not read like one with.
        println!("No supporting evidence was found for this question.");
    } else {
        println!("Citations ({}):", result.sources.len());
        for (index, source) in result.sources.iter().enumerate() {
            println!(
                "  [{}] {}  score={:.3}  {}",
                index + 1,
                source.chunk_id,
                source.relevance_score,
                source.source_path
            );
        }
    }
    if let Some(document_id) = &result.filed_document_id {
        println!("\nFiled as document {document_id} (searchable; `docs provenance` traces it).");
    } else if file {
        eprintln!("\nWarning: the answer could not be filed; see the log for details.");
    }
    println!(
        "\nRetrieval {}ms, synthesis {}ms",
        result.search_duration_ms, result.synthesis_duration_ms
    );
    // NFR-KB-003: Q&A budget is 5 seconds end to end.
    let total_ms = result.search_duration_ms + result.synthesis_duration_ms;
    if total_ms > 5_000 {
        eprintln!("Warning: answer took {total_ms}ms, over the 5s budget.");
    }
    Ok(())
}

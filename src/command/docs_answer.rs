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
//!
//! # Why the synthesized answer streams
//!
//! Measured on a live run: 9.6s end to end, 9ms of it retrieval. The model
//! round trip is the entire cost and no amount of local work shortens it, so
//! the synthesized path prints tokens as they arrive instead of holding a
//! finished answer back. The extractive path is not streamed and does not need
//! to be — it is local and already fast.

use std::io::{self, Write};
use std::time::{Duration, Instant};

use anyhow::Result;
use archon_core::config::ArchonConfig;
use archon_core::env_vars::ArchonEnvVars;
use archon_docs::retrieval::SearchMode;
use archon_pipeline::kb::query::{AnswerStreamSink, QaQueryOptions, QaQueryResult, QueryEngine};
use archon_pipeline::llm_adapter::KbProviderClient;

/// NFR-KB-003's Q&A budget.
const QA_BUDGET_MS: u64 = 5_000;

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

/// Writes streamed answer text straight to the terminal.
///
/// The flush on every fragment is the point of the whole exercise. A model
/// emits partial lines, and Rust's stdout is line-buffered when attached to a
/// terminal, so an unflushed fragment would sit in the buffer until a newline
/// turned up — recreating exactly the pause streaming exists to remove.
pub(crate) struct TerminalAnswerSink<W: Write + Send, E: Write + Send> {
    out: W,
    err: E,
    started: Instant,
    first_token_at: Option<Duration>,
    wrote_body: bool,
}

impl<W: Write + Send, E: Write + Send> TerminalAnswerSink<W, E> {
    pub(crate) fn new(out: W, err: E, started: Instant) -> Self {
        Self {
            out,
            err,
            started,
            first_token_at: None,
            wrote_body: false,
        }
    }

    /// Milliseconds from the start of the command to the first visible
    /// character, or `None` when nothing was ever emitted.
    pub(crate) fn time_to_first_token_ms(&self) -> Option<u64> {
        self.first_token_at.map(|at| at.as_millis() as u64)
    }

    pub(crate) fn wrote_body(&self) -> bool {
        self.wrote_body
    }
}

impl<W: Write + Send, E: Write + Send> AnswerStreamSink for TerminalAnswerSink<W, E> {
    fn on_retrieved(&mut self, warnings: &[String]) -> Result<()> {
        for warning in warnings {
            writeln!(self.err, "Warning: {warning}")?;
        }
        Ok(())
    }

    fn on_token(&mut self, text: &str) -> Result<()> {
        if self.first_token_at.is_none() {
            self.first_token_at = Some(self.started.elapsed());
        }
        self.wrote_body = true;
        self.out.write_all(text.as_bytes())?;
        self.out.flush()?;
        Ok(())
    }
}

/// Everything printed after the answer body: citations, the filing note and the
/// timings.
///
/// Separated from [`handle_answer`] so it can be asserted on without a database
/// or a model behind it.
pub(crate) fn render_tail(
    out: &mut impl Write,
    err: &mut impl Write,
    result: &QaQueryResult,
    file_requested: bool,
    time_to_first_token_ms: Option<u64>,
    total_ms: u64,
) -> Result<()> {
    writeln!(out, "\n")?;
    if result.sources.is_empty() {
        // REQ-DOCS-015: an answer with no evidence must not read like one with.
        writeln!(out, "No supporting evidence was found for this question.")?;
    } else {
        writeln!(out, "Citations ({}):", result.sources.len())?;
        for (index, source) in result.sources.iter().enumerate() {
            writeln!(
                out,
                "  [{}] {}  score={:.3}  {}",
                index + 1,
                source.chunk_id,
                source.relevance_score,
                source.source_path
            )?;
        }
    }
    if let Some(document_id) = &result.filed_document_id {
        writeln!(
            out,
            "\nFiled as document {document_id} (searchable; `docs provenance` traces it)."
        )?;
    } else if file_requested {
        writeln!(
            err,
            "\nWarning: the answer could not be filed; see the log for details."
        )?;
    }
    match time_to_first_token_ms {
        Some(first_ms) => writeln!(
            out,
            "\nRetrieval {}ms, first token {first_ms}ms, total {total_ms}ms",
            result.search_duration_ms
        )?,
        None => writeln!(
            out,
            "\nRetrieval {}ms, synthesis {}ms",
            result.search_duration_ms, result.synthesis_duration_ms
        )?,
    }

    // NFR-KB-003 bounds how long the operator waits, and with a streamed answer
    // that wait ends at the first character, not at the last. Warning on the
    // total would fire on every healthy run — synthesis of a real answer takes
    // most of ten seconds no matter what — and a warning that is always on is
    // one nobody reads. So the budget is applied to time-to-first-token, which
    // is the number that actually reflects staring at a blank terminal. The
    // total is still printed above, unjudged, for anyone measuring throughput.
    let waited_ms = time_to_first_token_ms.unwrap_or(total_ms);
    if waited_ms > QA_BUDGET_MS {
        let what = if time_to_first_token_ms.is_some() {
            "the first token took"
        } else {
            "the answer took"
        };
        writeln!(
            err,
            "Warning: {what} {waited_ms}ms, over the {QA_BUDGET_MS}ms budget."
        )?;
    }
    Ok(())
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
    // Started before the provider is resolved, because the operator's wait
    // starts when they press enter — provider setup is part of it.
    let started = Instant::now();
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

    let mut sink = TerminalAnswerSink::new(io::stdout(), io::stderr(), started);
    let result = engine.query_streaming(query, &options, &mut sink).await?;
    let total_ms = started.elapsed().as_millis() as u64;
    let first_token_ms = sink.time_to_first_token_ms();
    let streamed_body = sink.wrote_body();

    let mut out = io::stdout();
    let mut err = io::stderr();
    if !streamed_body {
        // Defensive: a synthesizer that returns text without emitting it would
        // otherwise print citations for an invisible answer.
        write!(out, "{}", result.answer)?;
    }
    render_tail(&mut out, &mut err, &result, file, first_token_ms, total_ms)?;
    out.flush()?;
    Ok(())
}

#[cfg(test)]
#[path = "docs_answer_tests.rs"]
mod tests;

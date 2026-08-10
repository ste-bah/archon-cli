//! KB Q&A query engine — retrieve, gather context, synthesize, file answers.
//!
//! Implements REQ-KB-003, which `PRD-ARCHON-DOCS-001` specifies again as
//! REQ-DOCS-013/014/015. There is one capability, so there is one command:
//! `archon docs answer` drives this engine when a provider is configured and
//! falls back to `archon_docs::answer`'s extractive path when none is.
//!
//! NFR: search < 500ms, Q&A < 5s.
//!
//! # Where the evidence comes from and where answers go
//!
//! Retrieval runs over the chunks the shipping ingest path writes
//! (`doc_chunks`, via [`archon_docs::retrieval`]) — the same corpus
//! `kb search` and `docs search` rank, so the three verbs cannot disagree about
//! what is in the knowledge base. A filed answer is stored as an ordinary
//! document with `DerivedFrom` edges to the chunks it cited, which makes it
//! searchable in its own right.
//!
//! What this engine adds over `docs answer` — which is extractive and
//! discards its text — is LLM synthesis through [`QaSynthesizer`] and the
//! filing step.
//!
//! # Streaming
//!
//! [`QueryEngine::query_streaming`] is [`QueryEngine::query`] with an
//! [`AnswerStreamSink`] attached. It exists because synthesis is ~99% of the
//! command's wall clock, so a caller that can show text early should. Filing,
//! citations and the returned result are identical either way — see
//! [`AnswerStreamSink`].

use std::sync::Arc;
use std::time::Instant;

use anyhow::Result;
use archon_docs::models::{ProvenanceEdge, ProvenanceEdgeType};
use archon_docs::retrieval::SearchResult;
use cozo::DbInstance;
use tracing::warn;

use super::compile::{CONCEPT_SOURCE_PREFIX, SUMMARY_SOURCE_PREFIX};

#[path = "query_types.rs"]
mod query_types;
pub use query_types::*;

/// `source_path` prefix for an answer filed back into the knowledge base.
pub const ANSWER_SOURCE_PREFIX: &str = "archon-kb://answer/";

/// Score multiplier applied to hits inside previously filed answers.
///
/// EC-PIPE-018: a filed answer is real evidence but it is second-hand, so at
/// equal relevance it ranks below the source it was synthesised from. A
/// multiplier cannot do more than that — an answer that restates the question
/// and lists its citations genuinely does match more query terms.
const ANSWER_RANK_PENALTY: f64 = 0.9;

/// Characters of a chunk quoted in a citation.
const CITATION_SNIPPET_CHARS: usize = 200;

/// What REQ-DOCS-015 requires the engine to say when retrieval found nothing.
const INSUFFICIENT_CONTEXT: &str =
    "Insufficient context in the knowledge base to answer this question.";

/// Receives an answer in the order the model produces it.
///
/// Measured on a live `archon docs answer`: 9.6s end to end, 9ms of it
/// retrieval. The model round trip is the whole cost and streaming does not
/// shorten it — what it buys is the operator seeing text within a second
/// instead of watching a blank terminal for ten. Implementations write to a
/// terminal, so they own their own flushing; nothing downstream does it.
pub trait AnswerStreamSink: Send {
    /// Called once, after retrieval and before any answer text, carrying the
    /// non-fatal retrieval notes.
    ///
    /// They are handed over here rather than read off the returned
    /// [`QaQueryResult`] because by the time that value exists the answer has
    /// already been printed, and a warning about the evidence belongs above the
    /// answer it qualifies, not below it.
    fn on_retrieved(&mut self, warnings: &[String]) -> Result<()> {
        let _ = warnings;
        Ok(())
    }

    /// Called for each fragment of answer text, in order.
    fn on_token(&mut self, text: &str) -> Result<()>;
}

/// Trait for LLM-based answer synthesis.
#[async_trait::async_trait]
pub trait QaSynthesizer: Send + Sync {
    async fn synthesize(&self, question: &str, context: &str) -> Result<String>;

    /// Synthesize while handing each fragment to `sink` as it arrives, and
    /// return the complete text.
    ///
    /// The default bridges to [`Self::synthesize`] and emits the finished
    /// answer as one fragment, so an implementation with no incremental API
    /// stays correct without changes — it simply does not stream. An override
    /// MUST return everything it emitted: the return value, not the sink, is
    /// what gets filed.
    async fn synthesize_streaming(
        &self,
        question: &str,
        context: &str,
        sink: &mut dyn AnswerStreamSink,
    ) -> Result<String> {
        let answer = self.synthesize(question, context).await?;
        sink.on_token(&answer)?;
        Ok(answer)
    }
}

pub struct QueryEngine {
    /// Shared handle, never a clone of the inner `DbInstance` — see the note on
    /// [`super::compile::Compiler`]; filing an answer is a guarded write and
    /// fails outright against an unregistered instance.
    db: Arc<DbInstance>,
    synthesizer: Option<Box<dyn QaSynthesizer>>,
}

impl QueryEngine {
    pub fn new(db: Arc<DbInstance>) -> Self {
        Self {
            db,
            synthesizer: None,
        }
    }

    pub fn with_synthesizer(mut self, synth: Box<dyn QaSynthesizer>) -> Self {
        self.synthesizer = Some(synth);
        self
    }

    /// Full Q&A flow: retrieve, gather context, synthesize, optionally file.
    pub async fn query(&self, question: &str, opts: &QaQueryOptions) -> Result<QaQueryResult> {
        self.run_query(question, opts, None).await
    }

    /// As [`Self::query`], but hands the answer to `sink` as it is produced.
    ///
    /// Nothing else differs: the same [`QaQueryResult`] comes back and, with
    /// `file_answer`, the same complete answer is filed with the same
    /// provenance. Streaming changes when the operator sees the text, not what
    /// the engine does with it.
    pub async fn query_streaming(
        &self,
        question: &str,
        opts: &QaQueryOptions,
        sink: &mut dyn AnswerStreamSink,
    ) -> Result<QaQueryResult> {
        self.run_query(question, opts, Some(sink)).await
    }

    async fn run_query(
        &self,
        question: &str,
        opts: &QaQueryOptions,
        mut sink: Option<&mut dyn AnswerStreamSink>,
    ) -> Result<QaQueryResult> {
        let search_start = Instant::now();
        let (chunks, warnings) = self.retrieve(question, opts)?;
        let search_duration_ms = search_start.elapsed().as_millis() as u64;
        if let Some(sink) = sink.as_deref_mut() {
            sink.on_retrieved(&warnings)?;
        }

        if chunks.is_empty() {
            // Pushed through the sink as well: a streaming caller prints what
            // it is given, so skipping it here would leave a blank body where
            // the "no evidence" line belongs.
            if let Some(sink) = sink.as_deref_mut() {
                sink.on_token(INSUFFICIENT_CONTEXT)?;
            }
            return Ok(QaQueryResult {
                answer: INSUFFICIENT_CONTEXT.into(),
                sources: vec![],
                filed_document_id: None,
                search_duration_ms,
                synthesis_duration_ms: 0,
                warnings,
            });
        }

        let context = self.gather_context(chunks, opts.include_derived_context)?;

        let synth_start = Instant::now();
        let synthesized = self.synthesize_into(question, &context, sink).await?;
        let synthesis_duration_ms = synth_start.elapsed().as_millis() as u64;

        let filed_document_id = if opts.file_answer {
            match self.file_answer(question, &synthesized, opts.kb.as_deref()) {
                Ok(document_id) => Some(document_id),
                Err(e) => {
                    // The answer is already synthesised; failing to store it
                    // must not withhold it from the operator.
                    warn!("filing the answer failed: {}", e);
                    None
                }
            }
        } else {
            None
        };

        let sources = context
            .primary
            .iter()
            .map(|chunk| QaSource {
                chunk_id: chunk.chunk_id.clone(),
                document_id: chunk.document_id.clone(),
                source_path: chunk.source_path.clone(),
                relevance_score: chunk.score,
                quote: truncate_chars(&chunk.content, CITATION_SNIPPET_CHARS),
            })
            .collect();

        Ok(QaQueryResult {
            answer: synthesized.answer_text,
            sources,
            filed_document_id,
            search_duration_ms,
            synthesis_duration_ms,
            warnings,
        })
    }

    /// Retrieve candidate chunks, applying the knowledge-base filter and the
    /// filed-answer rank penalty.
    fn retrieve(
        &self,
        question: &str,
        opts: &QaQueryOptions,
    ) -> Result<(Vec<ScoredChunk>, Vec<String>)> {
        if opts.top_k == 0 {
            return Ok((Vec::new(), Vec::new()));
        }
        let allowed = match &opts.kb {
            Some(kb_id) => Some(archon_docs::store::list_kb_document_ids(&self.db, kb_id)?),
            None => None,
        };
        // Over-fetch when filtering: the retriever has no document filter, so
        // narrowing after the fact would otherwise return fewer than top_k.
        let fetch = if allowed.is_some() {
            opts.top_k.saturating_mul(4).max(opts.top_k)
        } else {
            opts.top_k
        };

        let results = archon_docs::retrieval::search_with_mode(
            &self.db,
            question,
            fetch,
            opts.mode,
            Default::default(),
        )?;
        let warnings = results.warnings.clone();

        let mut scored = Vec::new();
        for result in results.results {
            if let Some(allowed) = &allowed
                && !allowed.contains(&result.document_id)
            {
                continue;
            }
            scored.push(self.score_chunk(result)?);
        }
        scored.sort_by(|a, b| {
            b.score
                .total_cmp(&a.score)
                .then_with(|| a.chunk_id.cmp(&b.chunk_id))
        });
        scored.truncate(opts.top_k);
        Ok((scored, warnings))
    }

    fn score_chunk(&self, result: SearchResult) -> Result<ScoredChunk> {
        let source_path = self.source_path(&result.document_id)?;
        let penalty = if source_path.starts_with(ANSWER_SOURCE_PREFIX) {
            ANSWER_RANK_PENALTY
        } else {
            1.0
        };
        Ok(ScoredChunk {
            chunk_id: result.chunk_id,
            document_id: result.document_id,
            source_path,
            content: result.content,
            score: result.score * penalty,
        })
    }

    fn source_path(&self, document_id: &str) -> Result<String> {
        Ok(archon_docs::store::get_doc_source(&self.db, document_id)?
            .map(|doc| doc.source_path)
            .unwrap_or_default())
    }

    /// Attach the compiled summaries and concept articles that point at the
    /// documents the primary chunks came from.
    ///
    /// This is what makes `kb compile` pay off at question time: an answer can
    /// draw on a document-level summary the retriever would not have surfaced
    /// from a chunk match alone.
    pub fn gather_context(
        &self,
        primary: Vec<ScoredChunk>,
        include_derived: bool,
    ) -> Result<AnswerContext> {
        let mut context = AnswerContext {
            primary,
            ..Default::default()
        };
        if !include_derived {
            return Ok(context);
        }

        let mut seen = Vec::new();
        for chunk in &context.primary {
            for edge in archon_docs::store::list_provenance_to(&self.db, &chunk.document_id)? {
                let derived_id = edge.from_artifact_id;
                if seen.contains(&derived_id) {
                    continue;
                }
                seen.push(derived_id.clone());
                let path = self.source_path(&derived_id)?;
                let is_summary = path.starts_with(SUMMARY_SOURCE_PREFIX);
                let is_concept = path.starts_with(CONCEPT_SOURCE_PREFIX);
                if !is_summary && !is_concept {
                    continue;
                }
                let text = self.document_text(&derived_id)?;
                if text.trim().is_empty() {
                    continue;
                }
                if is_summary {
                    context.summaries.push(text);
                } else {
                    context.concepts.push(text);
                }
            }
        }
        Ok(context)
    }

    fn document_text(&self, document_id: &str) -> Result<String> {
        let mut chunks = archon_docs::store::list_chunks_for_doc(&self.db, document_id)?;
        chunks.sort_by_key(|chunk| chunk.chunk_index);
        Ok(chunks
            .iter()
            .map(|chunk| chunk.content.as_str())
            .collect::<Vec<_>>()
            .join("\n\n"))
    }

    /// Synthesize an answer using the LLM, or fall back to concatenated context.
    pub async fn synthesize_answer(
        &self,
        question: &str,
        context: &AnswerContext,
    ) -> Result<SynthesizedAnswer> {
        self.synthesize_into(question, context, None).await
    }

    async fn synthesize_into(
        &self,
        question: &str,
        context: &AnswerContext,
        sink: Option<&mut dyn AnswerStreamSink>,
    ) -> Result<SynthesizedAnswer> {
        let formatted = format_context(context);
        let citations = context
            .primary
            .iter()
            .map(|chunk| SourceCitation {
                chunk_id: chunk.chunk_id.clone(),
                document_id: chunk.document_id.clone(),
                quote: truncate_chars(&chunk.content, CITATION_SNIPPET_CHARS),
                relevance: chunk.score,
            })
            .collect();

        let answer_text = match (&self.synthesizer, sink) {
            (Some(synth), Some(sink)) => {
                synth
                    .synthesize_streaming(question, &synthesis_prompt(question, &formatted), sink)
                    .await?
            }
            (Some(synth), None) => {
                synth
                    .synthesize(question, &synthesis_prompt(question, &formatted))
                    .await?
            }
            // No model: the extractive fallback is assembled locally, so there
            // is nothing to stream — it is emitted in one piece so a streaming
            // caller still gets a body.
            (None, sink) => {
                let text = format!(
                    "Based on {} knowledge base source(s):\n\n{}",
                    context.primary.len(),
                    formatted
                );
                if let Some(sink) = sink {
                    sink.on_token(&text)?;
                }
                text
            }
        };

        Ok(SynthesizedAnswer {
            answer_text,
            source_citations: citations,
        })
    }

    /// File an answer back into the knowledge base as a searchable document.
    ///
    /// Returns the new document ID. Each citation becomes a `DerivedFrom` edge
    /// from the answer document to the chunk it quoted, so `docs provenance`
    /// can walk from the answer to its evidence.
    ///
    /// When the question was scoped to a knowledge base the answer joins it.
    /// Otherwise an answer filed under `--kb x` would be invisible to the next
    /// `--kb x` search — filed, but not where the operator was looking.
    pub fn file_answer(
        &self,
        question: &str,
        answer: &SynthesizedAnswer,
        kb: Option<&str>,
    ) -> Result<String> {
        let title = truncate_chars(question, 100);
        let body = format!(
            "# {title}\n\n{}\n\n## Sources\n{}\n",
            answer.answer_text,
            answer
                .source_citations
                .iter()
                .map(|c| format!("- {} ({})", c.chunk_id, c.document_id))
                .collect::<Vec<_>>()
                .join("\n")
        );
        let source_path = format!("{ANSWER_SOURCE_PREFIX}{}", uuid::Uuid::new_v4());
        let stored = archon_docs::ingest_text::ingest_text_source(
            &self.db,
            &source_path,
            "text/markdown",
            &body,
        )?;
        if let Some(kb_id) = kb {
            archon_docs::store::assign_document_to_kb(&self.db, kb_id, &stored.document_id)?;
        }

        for citation in &answer.source_citations {
            let edge = ProvenanceEdge {
                edge_id: format!(
                    "edge-kb-answer-{}-{}",
                    stored.document_id, citation.chunk_id
                ),
                from_artifact_id: stored.document_id.clone(),
                to_artifact_id: citation.chunk_id.clone(),
                edge_type: ProvenanceEdgeType::DerivedFrom,
                created_at: chrono::Utc::now().to_rfc3339(),
            };
            // Provenance is best effort: the answer is already committed and
            // must stay visible even if one edge cannot be written.
            if let Err(error) = archon_docs::store::insert_provenance_edge(&self.db, &edge) {
                warn!(
                    document_id = %stored.document_id,
                    chunk_id = %citation.chunk_id,
                    %error,
                    "failed to write filed-answer provenance edge"
                );
            }
        }
        Ok(stored.document_id)
    }
}

fn synthesis_prompt(question: &str, formatted_context: &str) -> String {
    format!(
        "Answer the following question using ONLY the provided context. \
         Cite your sources by chunk ID. If the context is insufficient, say so.\n\n\
         Question: {question}\n\nContext:\n{formatted_context}"
    )
}

fn format_context(context: &AnswerContext) -> String {
    let mut parts = Vec::new();
    for chunk in &context.primary {
        parts.push(format!(
            "### [{}] {} (relevance: {:.2})\n{}",
            chunk.chunk_id, chunk.source_path, chunk.score, chunk.content
        ));
    }
    if !context.summaries.is_empty() {
        parts.push("\n### Compiled summaries:".to_string());
        parts.extend(context.summaries.iter().cloned());
    }
    if !context.concepts.is_empty() {
        parts.push("\n### Related concepts:".to_string());
        parts.extend(context.concepts.iter().cloned());
    }
    parts.join("\n\n")
}

fn truncate_chars(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let prefix: String = text.chars().take(max).collect();
    format!("{prefix}...")
}

#[cfg(test)]
#[path = "query_tests.rs"]
mod query_tests;

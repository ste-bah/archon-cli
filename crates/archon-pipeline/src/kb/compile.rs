//! KB LLM compilation — summaries, concepts, cross-references, index.
//!
//! Implements REQ-KB-002. NFR-PIPE-012: 20 docs in < 5 minutes.
//!
//! # Where the content comes from and where it goes
//!
//! The pass reads the documents the shipping ingest path writes (`doc_sources`
//! / `doc_chunks`, via [`archon_docs`]) and writes everything it produces back
//! into that same store. A summary, a concept article and the index are
//! therefore ordinary documents the moment they are written: `kb search`,
//! `kb recall`, `docs search` and `kb process` all see them without knowing
//! this pass exists.
//!
//! It used to read and write a private `kb_nodes` graph instead. Nothing in the
//! shipping CLI ever populated that relation, so the pass had no reachable
//! input and its output had no reachable reader.
//!
//! The `Compiler` accepts an abstract [`KbLlmClient`] so tests can drive it
//! deterministically without a live model; the provider-backed implementation
//! lives beside the other provider adapters in [`crate::llm_adapter`].

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::Result;
use archon_docs::models::{ProvenanceEdge, ProvenanceEdgeType, SourceDocument};
use cozo::{DataValue, DbInstance, ScriptMutability};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

// ---------------------------------------------------------------------------
// Derived-document naming
// ---------------------------------------------------------------------------

/// URI scheme marking a document this pass produced.
///
/// Output is stored as ordinary documents, so the pass must be able to tell its
/// own output from operator content. Without that distinction the next run
/// summarises the previous run's summaries and the corpus grows without bound.
pub const DERIVED_SOURCE_SCHEME: &str = "archon-kb://";

/// `source_path` prefix for a per-document summary.
pub const SUMMARY_SOURCE_PREFIX: &str = "archon-kb://summary/";

/// `source_path` prefix for a concept article.
pub const CONCEPT_SOURCE_PREFIX: &str = "archon-kb://concept/";

/// `source_path` of the single knowledge-base index document.
pub const INDEX_SOURCE_PATH: &str = "archon-kb://index";

/// True when `source_path` names a document this pass wrote.
pub fn is_derived_source(source_path: &str) -> bool {
    source_path.starts_with(DERIVED_SOURCE_SCHEME)
}

// ---------------------------------------------------------------------------
// Trait
// ---------------------------------------------------------------------------

/// Abstract LLM completion interface for KB compilation.
///
/// Implementors call the actual LLM (see [`crate::llm_adapter`], or a stub).
#[async_trait::async_trait]
pub trait KbLlmClient: Send + Sync {
    /// Send a prompt and return the completion as text.
    async fn complete(&self, prompt: &str) -> Result<String>;
}

// ---------------------------------------------------------------------------
// Result types
// ---------------------------------------------------------------------------

/// Metrics returned after a compile pass.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CompileMetrics {
    /// Documents the watermark and filters selected for this run.
    ///
    /// Distinguishes "nothing new to compile" from "every document failed",
    /// which otherwise both report zero summaries.
    pub documents_selected: usize,
    pub summaries_generated: usize,
    pub concepts_extracted: usize,
    pub edges_created: usize,
    pub index_updated: bool,
    pub duration_secs: f64,
}

/// Result of compiling a single document.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentCompilation {
    /// The document that was summarised.
    pub source_document_id: String,
    /// The document the summary was stored as.
    pub summary_document_id: String,
    pub title: String,
    pub summary: String,
}

/// A concept extracted from one or more documents.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConceptArticle {
    pub name: String,
    pub explanation: String,
    /// Documents the concept was drawn from. Defaulted rather than required so
    /// one omission by the model does not discard the whole batch.
    #[serde(default)]
    pub source_documents: Vec<String>,
    /// The document ID the article was stored as. Not returned by the LLM —
    /// populated by [`Compiler::extract_concepts`].
    #[serde(default)]
    pub document_id: String,
}

/// A cross-reference relationship between two concepts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrossReference {
    pub source: String,
    pub target: String,
    pub relationship: String,
}

// ---------------------------------------------------------------------------
// Progress
// ---------------------------------------------------------------------------

/// Stage a [`CompileProgress`] event reports.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CompilePhase {
    DocumentsSelected,
    DocumentSummarized,
    DocumentFailed,
    ConceptsExtracted,
    ConceptsFailed,
    CrossReferencesBuilt,
    CrossReferencesFailed,
    IndexUpdated,
    IndexFailed,
}

/// One progress event.
///
/// NFR-PIPE-012 budgets five minutes for twenty documents and every document
/// costs a model round trip, so a caller that prints nothing leaves an operator
/// staring at a silent terminal for minutes. Emitting per document keeps the
/// wait legible without the pass itself deciding how to render it.
#[derive(Clone, Debug)]
pub struct CompileProgress {
    pub phase: CompilePhase,
    /// 1-based position of the document just handled, or 0 for batch phases.
    pub document_index: usize,
    pub document_total: usize,
    pub title: Option<String>,
    /// Why a `*Failed` phase failed. A bare "FAILED" tells an operator nothing
    /// they can act on, and these failures are usually provider or credential
    /// problems that only the error text identifies.
    pub detail: Option<String>,
    pub elapsed: Duration,
}

/// Sink for [`CompileProgress`] events.
///
/// `Arc<dyn Fn>` rather than `&mut dyn FnMut` because the pass is async: a
/// unique borrow held across an await point would make the whole future
/// non-`Send`.
pub type CompileProgressSink = Arc<dyn Fn(CompileProgress) + Send + Sync>;

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

fn now_f64() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

/// Parse an RFC3339 `discovered_at` into epoch seconds.
///
/// Documents whose timestamp cannot be parsed are treated as brand new
/// (`f64::MAX` would skip them forever; 0.0 would recompile them every run), so
/// they compile once and then land behind the watermark like everything else.
fn discovered_at_secs(value: &str) -> f64 {
    chrono::DateTime::parse_from_rfc3339(value)
        .map(|ts| ts.timestamp_millis() as f64 / 1000.0)
        .unwrap_or(f64::MAX)
}

/// A human-usable title for a document, derived from its source path.
fn title_for(document: &SourceDocument) -> String {
    document
        .source_path
        .rsplit(['/', '\\'])
        .find(|segment| !segment.is_empty())
        .unwrap_or(&document.source_path)
        .to_string()
}

/// Lowercase, dash-separated slug for a concept name.
fn slugify(name: &str) -> String {
    let slug: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let slug = slug.trim_matches('-').replace("--", "-");
    if slug.is_empty() {
        "concept".to_string()
    } else {
        slug
    }
}

/// Pull the JSON payload out of a model response.
///
/// Providers routinely wrap JSON in a ```json fence even when asked for bare
/// JSON. The first run of this pass against a live provider did exactly that:
/// every parse failed, the fence markup was stored verbatim as the "summary",
/// and concept extraction returned nothing. Stripping the fence costs four
/// lines and is the difference between the pass working and not.
fn json_payload(response: &str) -> &str {
    let trimmed = response.trim();

    // A fenced block, with or without a preamble ("Here you go:" happens).
    if let Some(start) = trimmed.find("```") {
        let rest = &trimmed[start + 3..];
        // Drop the optional language tag that follows the opening fence.
        let body = rest.split_once('\n').map_or(rest, |(_, body)| body);
        return body.split_once("```").map_or(body, |(body, _)| body).trim();
    }

    // Unfenced but prefixed with prose: take the outermost JSON value.
    if !trimmed.starts_with(['{', '[']) {
        let open = trimmed.find(['{', '[']);
        let close = trimmed.rfind(['}', ']']);
        if let (Some(open), Some(close)) = (open, close)
            && close > open
        {
            return trimmed[open..=close].trim();
        }
    }

    trimmed
}

fn truncate_chars(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    text.chars().take(max).collect()
}

// ---------------------------------------------------------------------------
// Compiler
// ---------------------------------------------------------------------------

/// LLM-powered knowledge base compiler.
///
/// Reads ingested documents, generates a summary per document and concept
/// articles across the batch, links both back to their sources with provenance
/// edges, and refreshes a single index document.
pub struct Compiler {
    /// Shared handle, never a clone of the inner `DbInstance`.
    ///
    /// `archon-cozo` registers a database's guard config against the address of
    /// the `DbInstance` inside the `Arc`. Cloning the instance out produces a
    /// new address with no registration, and every guarded write then fails
    /// with "database has no bound Cozo guard config".
    db: Arc<DbInstance>,
    llm: Box<dyn KbLlmClient>,
    /// Restrict compilation to one named knowledge base (`--kb`).
    kb_filter: Option<String>,
    progress: Option<CompileProgressSink>,
}

impl Compiler {
    /// Create a new `Compiler`.
    ///
    /// Ensures the `compile_state` relation exists (idempotent).
    pub fn new(db: Arc<DbInstance>, llm: Box<dyn KbLlmClient>) -> Result<Self> {
        Self::ensure_compile_schema(&db)?;
        Ok(Self {
            db,
            llm,
            kb_filter: None,
            progress: None,
        })
    }

    /// Compile only documents attached to `kb_id`.
    pub fn with_kb(mut self, kb_id: Option<String>) -> Self {
        self.kb_filter = kb_id.filter(|id| !id.trim().is_empty());
        self
    }

    /// Report progress to `sink` as the pass runs.
    pub fn with_progress(mut self, sink: CompileProgressSink) -> Self {
        self.progress = Some(sink);
        self
    }

    /// Create the `compile_state` relation used to track the incremental
    /// watermark. This is the one piece of state the pass owns itself.
    fn ensure_compile_schema(db: &DbInstance) -> Result<()> {
        let script = ":create compile_state { key: String => value: Float }";
        match db.run_script(script, Default::default(), ScriptMutability::Mutable) {
            Ok(_) => {}
            Err(e) => {
                let msg = e.to_string();
                if msg.contains("already exists") || msg.contains("conflicts") {
                    // Idempotent — relation already present
                } else {
                    return Err(anyhow::anyhow!(
                        "compile_state schema creation failed: {}",
                        msg
                    ));
                }
            }
        }
        Ok(())
    }

    fn emit(
        &self,
        phase: CompilePhase,
        index: usize,
        total: usize,
        title: Option<&str>,
        at: Instant,
    ) {
        self.emit_detail(phase, index, total, title, None, at);
    }

    fn emit_detail(
        &self,
        phase: CompilePhase,
        index: usize,
        total: usize,
        title: Option<&str>,
        detail: Option<String>,
        at: Instant,
    ) {
        if let Some(sink) = &self.progress {
            sink(CompileProgress {
                phase,
                document_index: index,
                document_total: total,
                title: title.map(ToString::to_string),
                detail,
                elapsed: at.elapsed(),
            });
        }
    }

    // -----------------------------------------------------------------------
    // Watermark
    // -----------------------------------------------------------------------

    fn get_last_compiled_at(&self) -> Result<f64> {
        let result = self
            .db
            .run_script(
                "?[value] := *compile_state{key, value}, key = 'last_compiled_at'",
                Default::default(),
                ScriptMutability::Immutable,
            )
            .map_err(|e| anyhow::anyhow!("read compile_state failed: {}", e))?;

        Ok(result
            .rows
            .first()
            .and_then(|r| r[0].get_float())
            .unwrap_or(0.0))
    }

    fn set_last_compiled_at(&self, ts: f64) -> Result<()> {
        let mut params = BTreeMap::new();
        params.insert("ts".to_string(), DataValue::from(ts));
        self.db
            .run_script(
                "?[key, value] <- [['last_compiled_at', $ts]] \
             :put compile_state { key => value }",
                params,
                ScriptMutability::Mutable,
            )
            .map_err(|e| anyhow::anyhow!("write compile_state failed: {}", e))?;
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Document selection
    // -----------------------------------------------------------------------

    /// Documents eligible for this run: operator content only, newer than the
    /// watermark, optionally restricted to one knowledge base.
    fn select_documents(&self, since: f64) -> Result<Vec<SourceDocument>> {
        // Compiling a database that has never seen an ingest is a legitimate
        // no-op, not a missing-relation error.
        archon_docs::schema::ensure_doc_schema(&self.db)?;

        let allowed = match &self.kb_filter {
            Some(kb_id) => Some(archon_docs::store::list_kb_document_ids(&self.db, kb_id)?),
            None => None,
        };

        let mut documents: Vec<SourceDocument> = archon_docs::store::list_doc_sources(&self.db)?
            .into_iter()
            .filter(|doc| !is_derived_source(&doc.source_path))
            .filter(|doc| discovered_at_secs(&doc.discovered_at) > since)
            .filter(|doc| {
                allowed
                    .as_ref()
                    .is_none_or(|ids| ids.contains(&doc.document_id))
            })
            .collect();
        documents.sort_by(|a, b| a.discovered_at.cmp(&b.discovered_at));
        Ok(documents)
    }

    /// Full text of a document, chunks in order.
    fn document_text(&self, document_id: &str) -> Result<String> {
        let mut chunks = archon_docs::store::list_chunks_for_doc(&self.db, document_id)?;
        chunks.sort_by_key(|chunk| chunk.chunk_index);
        Ok(chunks
            .iter()
            .map(|chunk| chunk.content.as_str())
            .collect::<Vec<_>>()
            .join("\n\n"))
    }

    // -----------------------------------------------------------------------
    // Storage helpers
    // -----------------------------------------------------------------------

    /// Store derived content as a document and mirror the source's knowledge-base
    /// memberships onto it.
    ///
    /// Mirroring matters: without it `kb search --kb x` would rank the corpus of
    /// `x` but never the summaries drawn from it.
    fn store_derived(
        &self,
        source_path: &str,
        content: &str,
        inherit_kb_of: &[String],
    ) -> Result<String> {
        let stored = archon_docs::ingest_text::ingest_text_source(
            &self.db,
            source_path,
            "text/markdown",
            content,
        )?;
        for source_document_id in inherit_kb_of {
            for kb_id in self.kb_memberships(source_document_id)? {
                archon_docs::store::assign_document_to_kb(&self.db, &kb_id, &stored.document_id)?;
            }
        }
        Ok(stored.document_id)
    }

    fn kb_memberships(&self, document_id: &str) -> Result<Vec<String>> {
        let mut params = BTreeMap::new();
        params.insert("did".to_string(), DataValue::from(document_id));
        let result = self
            .db
            .run_script(
                "?[kb_id] := *doc_kb_memberships{kb_id, document_id}, document_id = $did",
                params,
                ScriptMutability::Immutable,
            )
            .map_err(|e| anyhow::anyhow!("read kb memberships failed: {}", e))?;
        Ok(result
            .rows
            .iter()
            .filter_map(|row| row[0].get_str().map(ToString::to_string))
            .collect())
    }

    /// Insert a provenance edge with a deterministic ID so re-running the pass
    /// overwrites the same row instead of accumulating duplicates.
    fn link(&self, from: &str, to: &str, edge_type: ProvenanceEdgeType) -> Result<()> {
        let kind = match edge_type {
            ProvenanceEdgeType::Cites => "cites",
            _ => "derived",
        };
        archon_docs::store::insert_provenance_edge(
            &self.db,
            &ProvenanceEdge {
                edge_id: format!("edge-kb-{kind}-{from}-{to}"),
                from_artifact_id: from.to_string(),
                to_artifact_id: to.to_string(),
                edge_type,
                created_at: chrono::Utc::now().to_rfc3339(),
            },
        )?;
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Public compile API
    // -----------------------------------------------------------------------

    /// Main compile entry point.
    ///
    /// 1. Reads `last_compiled_at` from `compile_state`.
    /// 2. Selects ingested documents discovered after that watermark, skipping
    ///    this pass's own output.
    /// 3. For each: generates a summary, stores it as a document, and links it
    ///    to its source with a `DerivedFrom` edge.
    /// 4. Extracts concept articles across the batch, each stored as a document
    ///    linked to the documents it came from.
    /// 5. Builds `Cites` cross-references between concept articles.
    /// 6. Rewrites the single index document.
    /// 7. Records the current timestamp as `last_compiled_at`.
    pub async fn compile(&self) -> Result<CompileMetrics> {
        let start = Instant::now();
        let last_compiled_at = self.get_last_compiled_at()?;

        let documents = self.select_documents(last_compiled_at)?;
        let total = documents.len();
        self.emit(CompilePhase::DocumentsSelected, 0, total, None, start);
        if documents.is_empty() {
            return Ok(CompileMetrics {
                duration_secs: start.elapsed().as_secs_f64(),
                ..Default::default()
            });
        }

        info!("Compiling {} document(s)", total);
        let compile_ts = now_f64();

        let mut edges_created = 0usize;
        let mut failures = 0usize;
        let mut compiled: Vec<DocumentCompilation> = Vec::new();

        for (index, document) in documents.iter().enumerate() {
            let title = title_for(document);
            match self.compile_document(document).await {
                Ok(doc) => {
                    edges_created += 1; // one DerivedFrom edge per summary
                    self.emit(
                        CompilePhase::DocumentSummarized,
                        index + 1,
                        total,
                        Some(&title),
                        start,
                    );
                    compiled.push(doc);
                }
                Err(e) => {
                    failures += 1;
                    // One unreadable document must not abandon the other 19.
                    warn!(
                        "compile_document failed for document {}: {}",
                        document.document_id, e
                    );
                    self.emit_detail(
                        CompilePhase::DocumentFailed,
                        index + 1,
                        total,
                        Some(&title),
                        Some(e.to_string()),
                        start,
                    );
                }
            }
        }

        // Concepts and cross-references are best effort — a model that will not
        // produce parseable JSON must not cost the operator the summaries that
        // already succeeded. But the reason is reported, not swallowed: a silent
        // "0 concepts" is indistinguishable from a corpus with no concepts in it.
        let concepts = match self.extract_concepts(&compiled).await {
            Ok(concepts) => concepts,
            Err(e) => {
                warn!("extract_concepts failed: {}", e);
                self.emit_detail(
                    CompilePhase::ConceptsFailed,
                    0,
                    0,
                    None,
                    Some(e.to_string()),
                    start,
                );
                Vec::new()
            }
        };
        let concepts_extracted = concepts.len();
        edges_created += concepts
            .iter()
            .map(|concept| concept.source_documents.len())
            .sum::<usize>();
        self.emit(
            CompilePhase::ConceptsExtracted,
            0,
            concepts_extracted,
            None,
            start,
        );

        let named: Vec<(String, String)> = concepts
            .iter()
            .map(|c| (c.document_id.clone(), c.name.clone()))
            .collect();
        let cross_references = match self.build_cross_references(&named).await {
            Ok(count) => count,
            Err(e) => {
                warn!("build_cross_references failed: {}", e);
                self.emit_detail(
                    CompilePhase::CrossReferencesFailed,
                    0,
                    0,
                    None,
                    Some(e.to_string()),
                    start,
                );
                0
            }
        };
        edges_created += cross_references;
        self.emit(
            CompilePhase::CrossReferencesBuilt,
            0,
            cross_references,
            None,
            start,
        );

        let index_updated = match self.update_index_document().await {
            Ok(()) => {
                self.emit(CompilePhase::IndexUpdated, 0, total, None, start);
                true
            }
            Err(e) => {
                warn!("update_index_document failed: {}", e);
                self.emit_detail(
                    CompilePhase::IndexFailed,
                    0,
                    total,
                    None,
                    Some(e.to_string()),
                    start,
                );
                false
            }
        };

        // Advance the watermark only on a clean run. A provider outage that
        // failed every document would otherwise mark those documents compiled
        // forever, and the operator would have no way to tell — the next run
        // reports "nothing new" and the summaries never appear.
        if failures == 0 {
            self.set_last_compiled_at(compile_ts)?;
        } else {
            warn!(
                "{} document(s) failed; leaving the compile watermark unchanged so the next run retries them",
                failures
            );
        }

        let duration_secs = start.elapsed().as_secs_f64();
        let summaries_generated = compiled.len();
        info!(
            "Compile complete: {} summaries, {} concepts, {} edges in {:.2}s",
            summaries_generated, concepts_extracted, edges_created, duration_secs
        );

        Ok(CompileMetrics {
            documents_selected: total,
            summaries_generated,
            concepts_extracted,
            edges_created,
            index_updated,
            duration_secs,
        })
    }

    /// Compile a single document: generate a summary via the LLM and store it.
    ///
    /// If the LLM response is not valid JSON the raw text becomes the summary
    /// (logged at WARN, not fatal) — a model that ignores the response format
    /// still produced prose worth keeping.
    pub async fn compile_document(&self, document: &SourceDocument) -> Result<DocumentCompilation> {
        let title = title_for(document);
        let content = self.document_text(&document.document_id)?;
        if content.trim().is_empty() {
            anyhow::bail!("document {} has no chunk content", document.document_id);
        }

        let prompt = format!(
            "Summarize the following document in 100-200 words. Return JSON: {{\"summary\": \"...\"}}\n\nDocument title: {}\n\nContent:\n{}",
            title, content
        );
        let response = self.llm.complete(&prompt).await?;
        let payload = json_payload(&response);
        let summary = match serde_json::from_str::<serde_json::Value>(payload) {
            Ok(val) => val["summary"].as_str().unwrap_or(payload).to_string(),
            Err(_) => {
                warn!(
                    "Failed to parse LLM summary as JSON for document '{}', using raw response",
                    document.document_id
                );
                payload.to_string()
            }
        };

        let body = format!(
            "# Summary: {title}\n\n{summary}\n\nSource: {}\n",
            document.source_path
        );
        let summary_document_id = self.store_derived(
            &format!("{SUMMARY_SOURCE_PREFIX}{}", document.document_id),
            &body,
            std::slice::from_ref(&document.document_id),
        )?;
        self.link(
            &summary_document_id,
            &document.document_id,
            ProvenanceEdgeType::DerivedFrom,
        )?;

        Ok(DocumentCompilation {
            source_document_id: document.document_id.clone(),
            summary_document_id,
            title,
            summary,
        })
    }

    /// Extract key concepts across a batch of compiled documents.
    ///
    /// Each concept becomes a document with `DerivedFrom` edges to the
    /// documents it was drawn from.
    pub async fn extract_concepts(
        &self,
        compiled: &[DocumentCompilation],
    ) -> Result<Vec<ConceptArticle>> {
        if compiled.is_empty() {
            return Ok(vec![]);
        }

        let docs: String = compiled
            .iter()
            .map(|c| {
                format!(
                    "- {} (ID: {}): {}",
                    c.title,
                    c.source_document_id,
                    truncate_chars(&c.summary, 200)
                )
            })
            .collect::<Vec<_>>()
            .join("\n");

        let prompt = format!(
            "Extract key concepts from these documents. For each concept, provide a name and 2-3 sentence explanation.\nReturn JSON array: [{{\"name\": \"...\", \"explanation\": \"...\", \"source_documents\": [\"document_id\", ...]}}]\n\nDocuments:\n{}",
            docs
        );
        let response = self.llm.complete(&prompt).await?;

        let payload = json_payload(&response);
        let mut concepts = serde_json::from_str::<Vec<ConceptArticle>>(payload).map_err(|e| {
            anyhow::anyhow!(
                "concept extraction response was not a JSON array ({e}); response began: {}",
                truncate_chars(payload, 160)
            )
        })?;

        let known: Vec<&str> = compiled
            .iter()
            .map(|c| c.source_document_id.as_str())
            .collect();
        for concept in &mut concepts {
            // A model may cite a document that was not in the batch; keeping
            // such an edge would point provenance at nothing.
            concept
                .source_documents
                .retain(|id| known.contains(&id.as_str()));

            let body = format!("# {}\n\n{}\n", concept.name, concept.explanation);
            concept.document_id = self.store_derived(
                &format!("{CONCEPT_SOURCE_PREFIX}{}", slugify(&concept.name)),
                &body,
                &concept.source_documents,
            )?;
            for source_document_id in &concept.source_documents {
                if let Err(e) = self.link(
                    &concept.document_id,
                    source_document_id,
                    ProvenanceEdgeType::DerivedFrom,
                ) {
                    warn!(
                        "Failed to link concept '{}' to document '{}': {}",
                        concept.name, source_document_id, e
                    );
                }
            }
        }
        Ok(concepts)
    }

    /// Build cross-reference edges between concept articles.
    ///
    /// `concepts` is a slice of `(document_id, concept_name)` pairs. Returns the
    /// number of edges created.
    pub async fn build_cross_references(&self, concepts: &[(String, String)]) -> Result<usize> {
        if concepts.len() < 2 {
            return Ok(0);
        }

        let names: String = concepts
            .iter()
            .map(|(_, name)| name.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        let prompt = format!(
            "Given these concepts: [{}]. Identify relationships between them.\nReturn JSON array: [{{\"source\": \"name1\", \"target\": \"name2\", \"relationship\": \"description\"}}]",
            names
        );
        let response = self.llm.complete(&prompt).await?;

        let payload = json_payload(&response);
        let references = serde_json::from_str::<Vec<CrossReference>>(payload).map_err(|e| {
            anyhow::anyhow!(
                "cross-reference response was not a JSON array ({e}); response began: {}",
                truncate_chars(payload, 160)
            )
        })?;

        let by_name: std::collections::HashMap<&str, &str> = concepts
            .iter()
            .map(|(id, name)| (name.as_str(), id.as_str()))
            .collect();
        let mut count = 0;
        for reference in &references {
            let (Some(from), Some(to)) = (
                by_name.get(reference.source.as_str()),
                by_name.get(reference.target.as_str()),
            ) else {
                continue;
            };
            if from == to {
                continue;
            }
            match self.link(from, to, ProvenanceEdgeType::Cites) {
                Ok(()) => count += 1,
                Err(e) => warn!(
                    "Failed to link cross-reference '{}' -> '{}': {}",
                    reference.source, reference.target, e
                ),
            }
        }
        Ok(count)
    }

    /// Rewrite the single index document summarising the knowledge base.
    ///
    /// Deleted and re-ingested rather than updated in place: `doc_sources` rows
    /// are keyed by a generated document ID and deduplicated by content hash, so
    /// writing a changed index without removing the old one would leave two
    /// index documents in search results.
    pub async fn update_index_document(&self) -> Result<()> {
        let documents = archon_docs::store::list_doc_sources(&self.db)?;

        let mut sources = 0usize;
        let mut summaries = 0usize;
        let mut concepts = 0usize;
        let mut stale_index_ids = Vec::new();
        for document in &documents {
            if document.source_path == INDEX_SOURCE_PATH {
                stale_index_ids.push(document.document_id.clone());
            } else if document.source_path.starts_with(SUMMARY_SOURCE_PREFIX) {
                summaries += 1;
            } else if document.source_path.starts_with(CONCEPT_SOURCE_PREFIX) {
                concepts += 1;
            } else if !is_derived_source(&document.source_path) {
                sources += 1;
            }
        }

        let content = format!(
            "# Knowledge Base Index\n\n\
             Source documents: {sources}\n\
             Compiled summaries: {summaries}\n\
             Concept articles: {concepts}\n\
             Last compiled: {}\n",
            chrono::Utc::now().to_rfc3339()
        );

        for document_id in stale_index_ids {
            archon_docs::delete::delete_document(&self.db, &document_id)?;
        }
        self.store_derived(INDEX_SOURCE_PATH, &content, &[])?;
        Ok(())
    }
}

#[cfg(test)]
mod json_payload_tests {
    use super::json_payload;

    #[test]
    fn bare_json_passes_through() {
        assert_eq!(json_payload(" {\"a\":1} "), "{\"a\":1}");
    }

    #[test]
    fn a_fenced_block_is_unwrapped() {
        assert_eq!(json_payload("```json\n{\"a\":1}\n```"), "{\"a\":1}");
        assert_eq!(json_payload("```\n[1]\n```"), "[1]");
    }

    /// The case that broke the first live run: a preamble before the fence.
    #[test]
    fn a_preamble_before_the_fence_is_discarded() {
        assert_eq!(
            json_payload("Here you go:\n\n```json\n[{\"name\":\"x\"}]\n```\n"),
            "[{\"name\":\"x\"}]"
        );
    }

    #[test]
    fn an_unfenced_preamble_still_yields_the_outermost_value() {
        assert_eq!(
            json_payload("Sure! [{\"name\":\"x\"}] hope that helps"),
            "[{\"name\":\"x\"}]"
        );
    }

    /// Prose with no JSON at all is returned intact so it can be kept as a summary.
    #[test]
    fn prose_without_json_is_left_alone() {
        assert_eq!(json_payload("  just prose  "), "just prose");
    }
}

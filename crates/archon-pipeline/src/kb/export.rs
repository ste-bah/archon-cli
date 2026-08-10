//! Markdown export of the knowledge base.
//!
//! # What changed against the old `kb_nodes` export
//!
//! The previous version grouped `kb_nodes` by node type into `raw/`,
//! `compiled/`, `concepts/`, `answers/` and `index/`. Those five groups survive
//! unchanged — they map exactly onto the document store, because [`super::compile`]
//! and [`super::query`] write their output under the `archon-kb://` scheme:
//!
//! | old node type | new source                                |
//! |---------------|-------------------------------------------|
//! | `raw`         | ingested documents (any other source path)|
//! | `compiled`    | `archon-kb://summary/…`                   |
//! | `concept`     | `archon-kb://concept/…`                   |
//! | `answer`      | `archon-kb://answer/…`                    |
//! | `index`       | `archon-kb://index`                       |
//!
//! Two frontmatter fields could not carry over and were replaced rather than
//! dropped:
//!
//! - `domain_tag` had no equivalent — a document's grouping in the shipping
//!   store is its knowledge-base membership, so the field is now `kb`, listing
//!   every `doc_kb_memberships` entry.
//! - `chunk_index` was per-node and a `kb_node` was one chunk. A document owns
//!   many chunks, so the export joins them in `chunk_index` order and reports
//!   `chunks` (the count) instead.
//!
//! `derived_from` is new: it lists the provenance edges a summary, concept or
//! filed answer carries back to its sources, which the old export had no way to
//! show because edges lived in a separate relation it never read.

use std::path::Path;

use anyhow::Result;
use cozo::{DataValue, DbInstance, ScriptMutability};

use super::compile::{
    CONCEPT_SOURCE_PREFIX, INDEX_SOURCE_PATH, SUMMARY_SOURCE_PREFIX, is_derived_source,
};
use super::query::ANSWER_SOURCE_PREFIX;

/// Export groups, in output order.
pub const GROUPS: [&str; 5] = ["raw", "compiled", "concepts", "answers", "index"];

/// What to export.
#[derive(Clone, Debug, Default)]
pub struct ExportOptions {
    /// Restrict the export to one named knowledge base.
    pub kb: Option<String>,
}

/// One document, resolved and ready to render.
#[derive(Clone, Debug)]
pub struct ExportedDocument {
    pub group: &'static str,
    pub document_id: String,
    pub source_path: String,
    pub media_type: String,
    pub discovered_at: String,
    pub knowledge_bases: Vec<String>,
    pub derived_from: Vec<String>,
    pub chunks: usize,
    pub content: String,
}

/// Counts written, by group.
#[derive(Clone, Debug, Default)]
pub struct ExportSummary {
    pub raw: usize,
    pub compiled: usize,
    pub concepts: usize,
    pub answers: usize,
    pub index: usize,
}

impl ExportSummary {
    pub fn total(&self) -> usize {
        self.raw + self.compiled + self.concepts + self.answers + self.index
    }

    fn record(&mut self, group: &str) {
        match group {
            "compiled" => self.compiled += 1,
            "concepts" => self.concepts += 1,
            "answers" => self.answers += 1,
            "index" => self.index += 1,
            _ => self.raw += 1,
        }
    }
}

fn group_for(source_path: &str) -> &'static str {
    if source_path == INDEX_SOURCE_PATH {
        "index"
    } else if source_path.starts_with(SUMMARY_SOURCE_PREFIX) {
        "compiled"
    } else if source_path.starts_with(CONCEPT_SOURCE_PREFIX) {
        "concepts"
    } else if source_path.starts_with(ANSWER_SOURCE_PREFIX) {
        "answers"
    } else if is_derived_source(source_path) {
        // A future `archon-kb://` kind must land somewhere visible rather than
        // being silently filed with operator content.
        "compiled"
    } else {
        "raw"
    }
}

/// Collect every document the export covers, in group then path order.
pub fn collect(db: &DbInstance, options: &ExportOptions) -> Result<Vec<ExportedDocument>> {
    // A database that has never seen an ingest has no `doc_sources` relation,
    // and "nothing to export" should read as an empty export rather than a
    // missing-relation error.
    archon_docs::schema::ensure_doc_schema(db)?;

    let allowed = match &options.kb {
        Some(kb_id) => Some(archon_docs::store::list_kb_document_ids(db, kb_id)?),
        None => None,
    };

    let mut exported = Vec::new();
    for document in archon_docs::store::list_doc_sources(db)? {
        if let Some(allowed) = &allowed
            && !allowed.contains(&document.document_id)
        {
            continue;
        }
        let mut chunks = archon_docs::store::list_chunks_for_doc(db, &document.document_id)?;
        chunks.sort_by_key(|chunk| chunk.chunk_index);
        exported.push(ExportedDocument {
            group: group_for(&document.source_path),
            knowledge_bases: memberships(db, &document.document_id)?,
            derived_from: archon_docs::store::list_provenance_from(db, &document.document_id)?
                .into_iter()
                .map(|edge| edge.to_artifact_id)
                .collect(),
            chunks: chunks.len(),
            content: chunks
                .iter()
                .map(|chunk| chunk.content.as_str())
                .collect::<Vec<_>>()
                .join("\n\n"),
            document_id: document.document_id,
            source_path: document.source_path,
            media_type: document.media_type,
            discovered_at: document.discovered_at,
        });
    }

    exported.sort_by(|a, b| {
        group_order(a.group)
            .cmp(&group_order(b.group))
            .then_with(|| a.source_path.cmp(&b.source_path))
            .then_with(|| a.document_id.cmp(&b.document_id))
    });
    Ok(exported)
}

fn group_order(group: &str) -> usize {
    GROUPS
        .iter()
        .position(|g| *g == group)
        .unwrap_or(GROUPS.len())
}

fn memberships(db: &DbInstance, document_id: &str) -> Result<Vec<String>> {
    let mut params = std::collections::BTreeMap::new();
    params.insert("did".to_string(), DataValue::from(document_id));
    let result = db
        .run_script(
            "?[kb_id] := *doc_kb_memberships{kb_id, document_id}, document_id = $did :order kb_id",
            params,
            ScriptMutability::Immutable,
        )
        .map_err(|e| anyhow::anyhow!("read kb memberships failed: {e}"))?;
    Ok(result
        .rows
        .iter()
        .filter_map(|row| row[0].get_str().map(ToString::to_string))
        .collect())
}

/// Render one document as a markdown file body with YAML frontmatter.
pub fn render(document: &ExportedDocument) -> String {
    format!(
        "---\n\
         document_id: {}\n\
         group: {}\n\
         source: {}\n\
         media_type: {}\n\
         kb: [{}]\n\
         derived_from: [{}]\n\
         chunks: {}\n\
         discovered_at: {}\n\
         ---\n\n\
         # {}\n\n{}\n",
        document.document_id,
        document.group,
        document.source_path,
        document.media_type,
        document.knowledge_bases.join(", "),
        document.derived_from.join(", "),
        document.chunks,
        document.discovered_at,
        title_of(document),
        document.content,
    )
}

/// Heading for a document.
///
/// Derived documents are keyed by a generated ID, so the last path segment —
/// which is the right title for an ingested file — reads as `# doc-3f9a…` for
/// everything this pass produced. Each derived kind gets a heading a human can
/// scan instead.
fn title_of(document: &ExportedDocument) -> String {
    let last = document
        .source_path
        .rsplit(['/', '\\'])
        .find(|segment| !segment.is_empty())
        .unwrap_or(&document.source_path);

    if document.source_path == INDEX_SOURCE_PATH {
        "Knowledge Base Index".to_string()
    } else if document.source_path.starts_with(SUMMARY_SOURCE_PREFIX) {
        format!("Summary of {last}")
    } else if document.source_path.starts_with(CONCEPT_SOURCE_PREFIX) {
        last.replace('-', " ")
    } else if document.source_path.starts_with(ANSWER_SOURCE_PREFIX) {
        format!("Filed answer {last}")
    } else {
        last.to_string()
    }
}

/// Export to a single markdown stream (for stdout or a single file).
pub fn export_markdown(db: &DbInstance, options: &ExportOptions) -> Result<String> {
    let documents = collect(db, options)?;
    let mut out = String::from("# Knowledge base export\n");
    if let Some(kb) = &options.kb {
        out.push_str(&format!("\nKnowledge base: `{kb}`\n"));
    }
    for group in GROUPS {
        let in_group: Vec<&ExportedDocument> =
            documents.iter().filter(|d| d.group == group).collect();
        if in_group.is_empty() {
            continue;
        }
        out.push_str(&format!("\n## {group} ({})\n\n", in_group.len()));
        for document in in_group {
            out.push_str(&render(document));
            out.push('\n');
        }
    }
    if documents.is_empty() {
        out.push_str("\nNo documents. Ingest with `archon kb ingest <path>` first.\n");
    }
    Ok(out)
}

/// Export to a directory of markdown files, one per document, grouped into
/// `raw/`, `compiled/`, `concepts/`, `answers/` and `index/` subdirectories.
pub fn export_to_directory(
    db: &DbInstance,
    path: &Path,
    options: &ExportOptions,
) -> Result<ExportSummary> {
    let documents = collect(db, options)?;
    let mut summary = ExportSummary::default();
    for document in &documents {
        let dir = path.join(document.group);
        std::fs::create_dir_all(&dir)?;
        std::fs::write(
            dir.join(format!("{}.md", sanitize_filename(&document.document_id))),
            render(document),
        )?;
        summary.record(document.group);
    }
    Ok(summary)
}

/// Sanitize a string for use as a filename (replace non-alphanumeric with `_`).
fn sanitize_filename(value: &str) -> String {
    value
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

#[cfg(test)]
#[path = "export_tests.rs"]
mod export_tests;

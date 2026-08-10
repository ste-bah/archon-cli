//! `archon kb recall` — the R7 unified recall facade, wired to real stores.
//!
//! This is the live call site for `archon_knowledge::recall`. It assembles the
//! four adapters over the handles the CLI already owns, runs one query, and
//! prints the merged answer together with the two things the slice exists to
//! make visible: every source's outcome, and the fact that the scores are not
//! calibrated.
//!
//! # Why this is a new subcommand rather than a flag on `kb search`
//!
//! `kb search` returns `KnowledgeSearchResult` with `exact`/`semantic`/combined
//! columns; recall returns merged hits with provenance, duplicates and
//! conflicts. Folding one into the other would have meant either changing the
//! existing output — a breaking change for anything parsing it — or printing two
//! different row shapes under one flag. A separate verb costs nothing and leaves
//! `kb search` exactly as it was.
//!
//! # Sources that cannot be opened are reported, never dropped
//!
//! Memory lives in a different database and the code index may not have been
//! built. Both are optional at runtime; neither is allowed to vanish. A source
//! the operator selected but whose store would not open stays in the policy with
//! no adapter, so the facade accounts for it as `NoAdapter` and the summary says
//! so. The R7 gate rolls back on "any source silently omitted".

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use archon_knowledge::hybrid_retriever::SearchOptions;
use archon_knowledge::recall::adapters::{
    CodeIndexAdapter, DocsAdapter, KnowledgeStoreAdapter, MemoryAdapter,
};
use archon_knowledge::recall::{
    RecallHit, RecallQuery, RecallResponse, RecallSource, SourcePolicy, SourceStatus, UnifiedRecall,
};
use cozo::DbInstance;

use crate::command::kb_recall_sources::{CodeIndexStore, DocsStore, MemoryStore};

/// Everything `kb recall` needs that is not the query itself.
pub(crate) struct RecallArgs {
    pub(crate) limit: usize,
    /// Which stores may answer, in the operator's own order.
    pub(crate) sources: Vec<RecallSource>,
    pub(crate) source_timeout: Duration,
    /// Path to an already-built LEANN index. Without it the code source has no
    /// adapter, which is reported rather than hidden.
    pub(crate) code_index: Option<PathBuf>,
    /// Mode and any query embedding for the knowledge graph's own retriever.
    pub(crate) knowledge_options: SearchOptions,
}

/// Parse a comma-separated `--sources` list.
///
/// An unknown name is an error rather than a silent skip: a typo that quietly
/// narrowed the query would look identical to a store having nothing to say.
pub(crate) fn parse_sources(raw: &str) -> Result<Vec<RecallSource>> {
    let mut sources = Vec::new();
    for name in raw.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        let source = RecallSource::parse(name).ok_or_else(|| {
            anyhow::anyhow!(
                "unknown recall source `{name}`; expected one of memory, docs, knowledge, code"
            )
        })?;
        if !sources.contains(&source) {
            sources.push(source);
        }
    }
    if sources.is_empty() {
        anyhow::bail!("--sources selected no store");
    }
    Ok(sources)
}

pub(crate) async fn handle_recall(
    db: Arc<DbInstance>,
    query_text: &str,
    args: RecallArgs,
) -> Result<()> {
    let policy = SourcePolicy::even_share(&args.sources, args.limit, args.source_timeout);
    let query = RecallQuery {
        text: query_text.to_string(),
        limit: args.limit,
        source_policy: policy,
    };

    let mut recall = UnifiedRecall::new();
    for source in &args.sources {
        match source {
            RecallSource::Knowledge => {
                recall = recall.with_source(Arc::new(KnowledgeStoreAdapter::new(
                    Arc::clone(&db),
                    args.knowledge_options.clone(),
                )));
            }
            RecallSource::Docs => {
                recall = recall.with_source(Arc::new(DocsAdapter::new(Arc::new(DocsStore::new(
                    Arc::clone(&db),
                )))));
            }
            RecallSource::Memory => match MemoryStore::open_default() {
                Ok(store) => {
                    recall = recall.with_source(Arc::new(MemoryAdapter::new(Arc::new(store))));
                }
                Err(error) => eprintln!("Warning: memory source unavailable: {error}"),
            },
            RecallSource::Code => match open_code_index(args.code_index.as_deref()) {
                Ok(store) => {
                    recall = recall.with_source(Arc::new(CodeIndexAdapter::new(Arc::new(store))));
                }
                Err(error) => eprintln!("Warning: code source unavailable: {error}"),
            },
        }
    }

    print_response(&recall.recall(&query));
    Ok(())
}

fn open_code_index(path: Option<&std::path::Path>) -> Result<CodeIndexStore> {
    let path = path.ok_or_else(|| {
        anyhow::anyhow!("--code-index not given, so the code index was not consulted")
    })?;
    CodeIndexStore::open(path)
}

fn print_response(response: &RecallResponse) {
    for hit in &response.hits {
        println!("{}", hit_line(hit));
        for duplicate in &hit.duplicates {
            println!("    also in {}/{}", duplicate.source, duplicate.source_id);
        }
        for index in &hit.conflicts {
            println!("    disputed: conflict #{index}");
        }
    }
    println!("{} hit(s)", response.hits.len());

    for (index, conflict) in response.conflicts.iter().enumerate() {
        println!(
            "CONFLICT #{index} [{}] {} — {}",
            conflict.kind.as_str(),
            conflict.identity,
            conflict.explanation
        );
        for member in &conflict.members {
            println!(
                "    {}/{}  {}",
                member.source,
                member.source_id,
                preview(&member.content)
            );
        }
    }

    println!("Sources:");
    for outcome in &response.sources {
        println!(
            "  {:<9} {:<28} kept={} of {} in {}ms",
            outcome.source,
            status_label(&outcome.status),
            outcome.kept,
            outcome.returned,
            outcome.elapsed_ms
        );
    }
    if response.is_partial() {
        println!("Partial result: not every source answered cleanly.");
    }
    // Printed unconditionally and last, because an operator reading a score
    // column with no warning will read it as measured relevance.
    println!("Scores are UNCALIBRATED — {}", response.calibration_note);
}

fn hit_line(hit: &RecallHit) -> String {
    format!(
        "{:<9} {}  score={:.3}~  [{}]  {}",
        hit.source.as_str(),
        hit.source_id,
        hit.normalized_score,
        hit.provenance_refs.join(" "),
        preview(&hit.content)
    )
}

/// A one-line status an operator can scan, with the store's own message kept.
fn status_label(status: &SourceStatus) -> String {
    match status {
        SourceStatus::Ok => "ok".to_string(),
        SourceStatus::Failed { error } => format!("FAILED: {error}"),
        SourceStatus::LatencyBudgetExceeded { budget_ms } => {
            format!("TIMED OUT after {budget_ms}ms")
        }
        SourceStatus::Panicked { payload } => format!("PANICKED: {payload}"),
        SourceStatus::NoAdapter => "NOT CONSULTED (no adapter)".to_string(),
        SourceStatus::ExcludedByPolicy => "excluded by --sources".to_string(),
    }
}

fn preview(content: &str) -> String {
    const MAX: usize = 96;
    let flattened = content.split_whitespace().collect::<Vec<_>>().join(" ");
    if flattened.chars().count() <= MAX {
        return flattened;
    }
    let prefix: String = flattened.chars().take(MAX).collect();
    format!("{prefix}...")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sources_parse_in_the_operators_order_without_duplicates() {
        let parsed = parse_sources("code, memory ,code,docs").unwrap();
        assert_eq!(
            parsed,
            vec![RecallSource::Code, RecallSource::Memory, RecallSource::Docs]
        );
    }

    #[test]
    fn an_unknown_source_is_refused_rather_than_narrowing_the_query() {
        let error = parse_sources("docs,memries").unwrap_err().to_string();
        assert!(error.contains("unknown recall source `memries`"), "{error}");
    }

    #[test]
    fn an_empty_source_list_is_refused() {
        assert!(parse_sources("  ,  ").is_err());
    }

    /// The score column must never appear without a marker; `~` plus the
    /// trailing note is that marker.
    #[test]
    fn a_hit_line_marks_its_score_as_approximate() {
        let hit = RecallHit::at_rank(RecallSource::Docs, "c1", "Retention is thirty days.", 0)
            .with_provenance(["chunk:c1".to_string()]);
        let line = hit_line(&hit);
        assert!(line.contains("score=1.000~"), "{line}");
        assert!(line.contains("[chunk:c1]"), "{line}");
    }

    #[test]
    fn a_failed_source_label_keeps_the_stores_own_message() {
        let label = status_label(&SourceStatus::Failed {
            error: "no code index at /nowhere".into(),
        });
        assert!(label.contains("no code index at /nowhere"), "{label}");
    }

    #[test]
    fn a_source_with_no_adapter_says_it_was_not_consulted() {
        assert!(status_label(&SourceStatus::NoAdapter).contains("NOT CONSULTED"));
    }

    #[test]
    fn preview_flattens_and_truncates() {
        let long = "word ".repeat(50);
        let shown = preview(&long);
        assert!(shown.ends_with("..."));
        assert!(!shown.contains('\n'));
    }
}

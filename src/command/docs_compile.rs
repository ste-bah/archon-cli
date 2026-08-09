//! `archon docs compile` and `archon docs export` — the live call sites for
//! `archon_pipeline::kb::compile` and `archon_pipeline::kb::export`.
//!
//! # Why these live under `docs` and not `kb`
//!
//! Both read `doc_chunks` and write documents. `PRD-ARCHON-DOCS-001` puts
//! document intelligence in its own `docs` namespace over a shared engine, and
//! putting compile under `kb` would have implied a second corpus — which is
//! exactly the split this work exists to close.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use archon_core::config::ArchonConfig;
use archon_core::env_vars::ArchonEnvVars;
use archon_pipeline::kb::compile::{CompilePhase, CompileProgress, Compiler};
use archon_pipeline::kb::export::{ExportOptions, export_markdown, export_to_directory};

/// `archon docs compile`
pub(crate) async fn handle_compile(
    config: &ArchonConfig,
    env_vars: &ArchonEnvVars,
    kb: Option<String>,
    model: Option<String>,
) -> Result<()> {
    let db = crate::command::docs::open_db()?;
    let llm = crate::command::docs_answer::build_kb_client(config, env_vars, model)
        .await
        .ok_or_else(|| {
            // Compilation is entirely model work — unlike answering there is no
            // meaningful extractive fallback, so this is an error rather than a
            // degraded mode.
            anyhow::anyhow!(
                "no LLM provider is configured, so there is nothing to compile with. \
                 Run `archon auth` or set a provider in config.toml."
            )
        })?;

    if let Some(kb_id) = &kb {
        println!("KB: {kb_id}");
    }
    let compiler = Compiler::new(Arc::clone(&db), Box::new(llm))?
        .with_kb(kb)
        .with_progress(Arc::new(report_progress));

    let metrics = compiler.compile().await?;

    println!("Compile complete");
    println!("Documents selected:  {}", metrics.documents_selected);
    println!("Summaries generated: {}", metrics.summaries_generated);
    println!("Concepts extracted:  {}", metrics.concepts_extracted);
    println!("Edges created:       {}", metrics.edges_created);
    println!("Index updated:       {}", metrics.index_updated);
    println!("Duration:            {:.1}s", metrics.duration_secs);

    // Zero can mean two very different things, and conflating them sends an
    // operator to re-ingest documents that are already there.
    if metrics.documents_selected == 0 {
        println!(
            "Nothing new to compile. Ingest documents with `archon kb ingest <path>` first, \
             or check `archon docs list`."
        );
    } else if metrics.summaries_generated == 0 {
        anyhow::bail!(
            "all {} selected document(s) failed to compile; see the errors above",
            metrics.documents_selected
        );
    }
    Ok(())
}

/// Print one progress line per event.
///
/// NFR-PIPE-012 allows five minutes for twenty documents, so a silent run can
/// look indistinguishable from a hang. Every line carries elapsed time and the
/// document count so an operator can extrapolate.
fn report_progress(event: CompileProgress) {
    let elapsed = format_elapsed(event.elapsed);
    match event.phase {
        CompilePhase::DocumentsSelected => {
            if event.document_total == 0 {
                println!("[{elapsed}] No documents to compile.");
            } else {
                println!(
                    "[{elapsed}] Compiling {} document(s)...",
                    event.document_total
                );
            }
        }
        CompilePhase::DocumentSummarized => println!(
            "[{elapsed}] {}/{} summarized: {}",
            event.document_index,
            event.document_total,
            event.title.as_deref().unwrap_or("(untitled)")
        ),
        CompilePhase::DocumentFailed => eprintln!(
            "[{elapsed}] {}/{} FAILED: {} — {}",
            event.document_index,
            event.document_total,
            event.title.as_deref().unwrap_or("(untitled)"),
            event.detail.as_deref().unwrap_or("no detail")
        ),
        CompilePhase::ConceptsExtracted => println!(
            "[{elapsed}] {} concept article(s) extracted",
            event.document_total
        ),
        CompilePhase::ConceptsFailed => eprintln!(
            "[{elapsed}] concept extraction FAILED — {}",
            event.detail.as_deref().unwrap_or("no detail")
        ),
        CompilePhase::CrossReferencesBuilt => println!(
            "[{elapsed}] {} cross-reference(s) built",
            event.document_total
        ),
        CompilePhase::CrossReferencesFailed => eprintln!(
            "[{elapsed}] cross-referencing FAILED — {}",
            event.detail.as_deref().unwrap_or("no detail")
        ),
        CompilePhase::IndexUpdated => println!("[{elapsed}] index refreshed"),
        CompilePhase::IndexFailed => eprintln!(
            "[{elapsed}] index refresh FAILED — {}",
            event.detail.as_deref().unwrap_or("no detail")
        ),
    }
}

fn format_elapsed(elapsed: Duration) -> String {
    let secs = elapsed.as_secs();
    format!("{:02}:{:02}", secs / 60, secs % 60)
}

/// `archon docs export`
pub(crate) fn handle_export(out: Option<&Path>, kb: Option<String>) -> Result<()> {
    let db = crate::command::docs::open_db()?;
    let options = ExportOptions { kb };

    match out {
        Some(path) => {
            let summary = export_to_directory(&db, path, &options)?;
            println!(
                "Exported {} document(s) to {}",
                summary.total(),
                path.display()
            );
            println!("  raw:       {}", summary.raw);
            println!("  compiled:  {}", summary.compiled);
            println!("  concepts:  {}", summary.concepts);
            println!("  answers:   {}", summary.answers);
            println!("  index:     {}", summary.index);
        }
        None => print!("{}", export_markdown(&db, &options)?),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn elapsed_is_rendered_as_minutes_and_seconds() {
        assert_eq!(format_elapsed(Duration::from_secs(9)), "00:09");
        assert_eq!(format_elapsed(Duration::from_secs(75)), "01:15");
        assert_eq!(format_elapsed(Duration::from_secs(600)), "10:00");
    }
}

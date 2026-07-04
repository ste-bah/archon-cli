pub mod errors;
pub mod hash;
pub mod models;
pub mod schema;
pub mod store;

pub mod block_chunking;
pub mod chunking;
mod cozo_retry;
pub mod indexing;
mod indexing_adaptive;
mod indexing_cache;
mod indexing_options;
mod indexing_parallel;
mod indexing_progress;
mod indexing_result;
mod indexing_store;
pub mod ingest;
mod ingest_artifacts;
pub mod ingest_bytes;
mod ingest_directory;
mod ingest_multimodal;
mod ingest_pdf;
pub mod ingest_text;
pub mod inspect;
pub mod marker_source;
pub mod ocr;
pub mod pdf;
mod pdf_figure_vlm;
mod pdf_image_enrichment;
// The `--jobs auto` VRAM→worker derivation lives beside the enrichment engine's own worker
// knob (`image_workers`) so the two 1..=16 clamps can never drift apart; the CLI needs it to
// resolve the flag before ingest, hence this single-purpose re-export from an otherwise
// private module.
pub use pdf_image_enrichment::{VLM_HEADROOM_MB, VLM_SLOT_MB, auto_image_workers};
mod pdf_image_progress;
mod pdf_image_vlm;
pub mod pdf_scan;
pub mod provenance;
pub mod provenance_chunks;
pub mod quote_verify;
pub mod reprocess;
pub mod status;
mod tool_path;
pub mod vector_migration;
pub mod vector_store;

pub mod answer;
pub mod answer_timecode;
pub mod embed;
mod embed_config;
mod embed_fastembed;
mod embed_openai;
pub mod index_jobs;
pub mod index_queue;
#[cfg(test)]
mod index_queue_tests;
pub mod rerank;
pub mod retrieval;
mod retrieval_exact;
pub mod retrieval_image;
mod retrieval_query;
mod retrieval_semantic;
#[cfg(test)]
mod retrieval_tests;
pub mod vlm;

pub fn configure_cozo_write_lock_for_db(path: impl AsRef<std::path::Path>) {
    cozo_retry::configure_write_lock_for_db(path);
}

pub fn run_cozo_script_guarded(
    db: &cozo::DbInstance,
    script: &str,
    params: std::collections::BTreeMap<String, cozo::DataValue>,
    mutability: cozo::ScriptMutability,
    context: &str,
) -> anyhow::Result<cozo::NamedRows> {
    cozo_retry::run_script_guarded(db, script, params, mutability, context)
}

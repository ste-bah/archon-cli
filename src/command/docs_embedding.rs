use anyhow::Result;
use cozo::DbInstance;

use archon_docs::embed::{
    self, EmbeddingProviderConfig, EmbeddingProviderSelection, LocalEmbeddingProvider,
};
use archon_docs::vlm::factory::{self as vlm_factory, VlmProviderInitStatus};

pub(crate) fn init_embedding(_db: &DbInstance) -> Result<()> {
    if embed::get_provider().is_none() {
        let config = resolved_provider_config();
        match embed::init_provider(config) {
            Ok(()) => log_provider(),
            Err(e) => tracing::warn!("embedding provider not available: {e}"),
        }
    }
    Ok(())
}

pub(crate) async fn handle_model_status(db: DbInstance) -> Result<()> {
    let init_start = std::time::Instant::now();
    if embed::get_provider().is_none() {
        let _ = embed::init_provider(resolved_provider_config());
    }
    let init_elapsed = init_start.elapsed();

    match embed::get_provider() {
        Some(provider) => print_configured_status(&db, provider.as_ref(), init_elapsed),
        None => print_missing_status(init_elapsed),
    }
    print_vlm_status();
    Ok(())
}

fn resolved_provider_config() -> EmbeddingProviderConfig {
    let mut config = EmbeddingProviderConfig::from_env();
    if std::env::var_os("ARCHON_DOCS_EMBEDDING_PROVIDER").is_none()
        && let Ok(archon_config) = archon_core::config::load_config()
    {
        config.selection = match archon_config.memory.embedding_provider {
            archon_memory::embedding::EmbeddingProviderKind::Auto => {
                EmbeddingProviderSelection::Auto
            }
            archon_memory::embedding::EmbeddingProviderKind::Local => {
                EmbeddingProviderSelection::Local
            }
            archon_memory::embedding::EmbeddingProviderKind::OpenAI => {
                EmbeddingProviderSelection::OpenAiCompatible
            }
        };
        if config.openai_api_key.is_none() {
            config.openai_api_key = archon_config.llm.openai.api_key;
        }
        if config.openai_base_url.is_none() {
            config.openai_base_url = archon_config.llm.openai.base_url;
        }
    }
    config
}

fn log_provider() {
    if let Some(provider) = embed::get_provider() {
        tracing::info!(
            "embedding provider initialised: {}",
            provider.backend_name()
        );
    }
}

fn print_configured_status(
    db: &DbInstance,
    provider: &dyn LocalEmbeddingProvider,
    init_elapsed: std::time::Duration,
) {
    println!("Backend:       {}", provider.backend_name());
    println!("Dimension:     {}", provider.dimension());
    println!("Init result:   ok (took {}ms)", init_elapsed.as_millis());

    let smoke_start = std::time::Instant::now();
    match provider.embed_query("hello") {
        Ok(v) => println!(
            "Smoke embed:   ok (dim={}, took {}ms)",
            v.len(),
            smoke_start.elapsed().as_millis()
        ),
        Err(e) => {
            println!("Smoke embed:   failed: {}", e);
            println!("Hint: Cache may be corrupt or provider unavailable.");
        }
    }

    match archon_docs::store::count_embeddings(db) {
        Ok(count) => println!("Cozo vectors:  {} legacy indexed", count),
        Err(e) => println!("Vectors:       unable to query — {}", e),
    }
    print_vector_store_status(provider);
    match archon_docs::store::count_pending_chunks(db) {
        Ok(count) => println!("Pending:       {} chunks", count),
        Err(e) => println!("Pending:       unable to query — {}", e),
    }
    match archon_docs::index_queue::stats(db) {
        Ok(queue) => println!(
            "Index queue:   {} pending, {} leased, {} failed",
            queue.pending, queue.leased, queue.failed
        ),
        Err(e) => println!("Index queue:   unable to query — {}", e),
    }
    match archon_docs::index_jobs::summary(db) {
        Ok(jobs) => println!(
            "Index jobs:    {} running, {} paused, {} completed, {} failed, {} cancelled",
            jobs.running, jobs.paused, jobs.completed, jobs.failed, jobs.cancelled
        ),
        Err(e) => println!("Index jobs:    unable to query — {}", e),
    }
    match check_hnsw_index(db) {
        Ok(true) => println!("HNSW index:    present"),
        Ok(false) => println!("HNSW index:    not yet created"),
        Err(e) => println!("HNSW index:    unable to check — {}", e),
    }
    println!("pdfimages:     {}", pdfimages_status());
}

fn print_vector_store_status(provider: &dyn LocalEmbeddingProvider) {
    match archon_docs::vector_store::DocVectorStore::open_default() {
        Ok(store) => {
            match store.stats(Some(provider.backend_name())) {
                Ok(stats) => println!(
                    "RocksDB vecs:  {} raw, {} cache",
                    stats.raw_vectors, stats.cache_entries
                ),
                Err(e) => println!("RocksDB vecs:  unable to query — {e}"),
            }
            match store.latest_hnsw_manifest(provider.backend_name()) {
                Ok(Some(manifest)) => println!(
                    "Rust HNSW:     {} vectors, dim {}",
                    manifest.vector_count, manifest.dimension
                ),
                Ok(None) => println!("Rust HNSW:     not built"),
                Err(e) => println!("Rust HNSW:     unable to query — {e}"),
            }
        }
        Err(e) => println!("RocksDB vecs:  unavailable — {e}"),
    }
}

fn print_missing_status(init_elapsed: std::time::Duration) {
    println!("Backend:       not-configured");
    println!("Dimension:     n/a");
    println!(
        "Init result:   failed (took {}ms)",
        init_elapsed.as_millis()
    );
    if let Some(error) = embed::last_init_error() {
        println!("Last error:    {error}");
    }
    println!();
    println!("Set ARCHON_DOCS_EMBEDDING_PROVIDER=local|openai|auto|disabled.");
    println!("For OpenAI-compatible embeddings set ARCHON_DOCS_OPENAIKEY.");
    println!("For local embeddings, fastembed uses:");
    println!("  {}", fastembed_dir().display());
    println!("pdfimages:     {}", pdfimages_status());
}

fn print_vlm_status() {
    match std::env::current_dir()
        .map_err(anyhow::Error::from)
        .and_then(|cwd| archon_policy::load_policy_for_workspace(&cwd).map_err(anyhow::Error::from))
    {
        Ok(load) => {
            let decision = load.policy.docs_vlm_decision();
            let (provider, model) = vlm_factory::default_provider_summary(&load.policy);
            let report = vlm_factory::diagnostic_report(&load.policy);
            println!();
            println!(
                "VLM policy:    {} ({})",
                if decision.allowed {
                    "allowed"
                } else {
                    "denied"
                },
                decision.reason
            );
            println!(
                "VLM config:    provider={} model={}",
                provider,
                if model.is_empty() { "n/a" } else { &model }
            );
            println!("VLM provider:  {}", render_vlm_report(report));
        }
        Err(e) => println!("VLM policy:    unable to load policy — {e}"),
    }
}

fn render_vlm_report(report: vlm_factory::VlmProviderInitReport) -> String {
    match report.status {
        VlmProviderInitStatus::Registered => {
            format!("ok — {}/{} reachable", report.provider, report.model)
        }
        VlmProviderInitStatus::Disabled => format!("disabled — {}", report.message),
        VlmProviderInitStatus::Skipped => {
            format!(
                "skipped — {}/{}: {}",
                report.provider, report.model, report.message
            )
        }
    }
}

fn check_hnsw_index(db: &DbInstance) -> Result<bool> {
    match db.run_script(
        "?[count(chunk_id)] := *vec_text_chunks{chunk_id}",
        Default::default(),
        cozo::ScriptMutability::Immutable,
    ) {
        Ok(_) => Ok(true),
        Err(e)
            if e.to_string()
                .contains(archon_docs::errors::COZO_RELATION_NOT_FOUND) =>
        {
            Ok(false)
        }
        Err(e) => Err(anyhow::anyhow!("failed to query vec_text_chunks: {e}")),
    }
}

fn pdfimages_status() -> String {
    let bin = std::env::var_os("ARCHON_PDFIMAGES_BIN").unwrap_or_else(|| "pdfimages".into());
    let display = std::path::PathBuf::from(&bin).display().to_string();
    match std::process::Command::new(&bin).arg("-v").output() {
        Ok(output) if output.status.success() || !output.stderr.is_empty() => {
            format!("available ({display})")
        }
        Ok(output) => format!(
            "missing or unhealthy ({display}) status={:?}",
            output.status.code()
        ),
        Err(e) => format!("missing ({display}) — {e}"),
    }
}

fn fastembed_dir() -> std::path::PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("archon")
        .join("fastembed")
}

//! Opening the repository code index, and deciding whether to build it.
//!
//! Every session — TUI and `archon web` alike — opens the LEANN code index so
//! `/code`, the pipeline facades and the code-search tool have somewhere to
//! read from. Whether the session also *builds* that index is a separate
//! question, and it used to be answered unconditionally with "yes".
//!
//! The build is not a background trickle. It walks the working directory,
//! chunks every recognised source file, and pushes every chunk through a
//! CPU-resident BGE-base ONNX model; fastembed constructs that session with
//! `intra_threads = available_parallelism()`, so on a 32-core box the embedder
//! claims 32 intra-op threads and the process settles at 12–20 cores. Measured
//! here: 3,199 files, ~2.5 CPU-seconds per chunk, ~160 files indexed per four
//! minutes — roughly seventeen CPU-hours to finish once, on an `archon web`
//! that had never served a request and had no client connected. The sawtooth
//! that looks like a job restarting is the embed phase (parallel, ~20 cores)
//! alternating with the Cozo/HNSW persist phase (serial, under one core).
//!
//! It is resumable — `file_hash_matches` skips what is stored and each 32-file
//! group is committed before the next begins — but resumable is not cheap, and
//! two processes sharing one `.archon/leann.db` hand the write lock back and
//! forth in 20–40 second turns, which is what starves a second `archon` at
//! startup. So the build is opt-in: `[code_index] index_on_startup = true`.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use archon_tui::observability;

/// The code index handle plus the flag that cancels an in-flight build.
///
/// The cancel flag is returned even when no build was started: session
/// teardown stores `true` into it unconditionally, and a flag nobody reads is
/// cheaper than a branch every caller has to remember.
pub(super) struct LeannStartup {
    pub(super) integration: Option<Arc<archon_pipeline::runner::LeannIntegration>>,
    pub(super) cancel: Arc<AtomicBool>,
}

/// Open `<working_dir>/.archon/leann.db` and, if configured to, start the
/// repository build behind it.
pub(super) fn begin(
    config: &archon_core::config::ArchonConfig,
    working_dir: &Path,
) -> LeannStartup {
    let cancel = Arc::new(AtomicBool::new(false));
    let db_path = working_dir.join(".archon").join("leann.db");
    if let Some(parent) = db_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    let integration = match archon_leann::CodeIndex::new(&db_path, Default::default()) {
        Ok(idx) => Some(Arc::new(archon_pipeline::runner::LeannIntegration::new(
            Arc::new(idx),
        ))),
        Err(e) => {
            tracing::warn!(error = %e, "LEANN unavailable; continuing without code context");
            None
        }
    };

    if let Some(integration) = integration.as_ref() {
        if index_on_startup(config) {
            spawn_repository_build(
                Arc::clone(integration),
                working_dir.to_path_buf(),
                Arc::clone(&cancel),
            );
        } else {
            tracing::info!(
                "LEANN repository index opened but not built: \
                 set [code_index] index_on_startup = true to index this repository"
            );
        }
    }

    LeannStartup {
        integration,
        cancel,
    }
}

/// Whether this session should build the repository index.
pub(super) fn index_on_startup(config: &archon_core::config::ArchonConfig) -> bool {
    config.code_index.index_on_startup
}

/// Run the repository build off the Tokio worker threads.
///
/// `spawn_blocking` because tree-sitter, ONNX embedding and Cozo writes are all
/// synchronous and would otherwise occupy a worker for the whole run. The
/// cancellation flag is checked between files and between batches inside LEANN,
/// which matters at shutdown: dropping a `#[tokio::main]` runtime *waits* for
/// blocking tasks rather than aborting them, so without it a Ctrl-C leaves the
/// process embedding for a consumer that has already exited.
fn spawn_repository_build(
    integration: Arc<archon_pipeline::runner::LeannIntegration>,
    working_dir: PathBuf,
    cancel: Arc<AtomicBool>,
) {
    observability::spawn_named("leann-background-init", async move {
        let cancel_for_blocking = Arc::clone(&cancel);
        let result = observability::spawn_blocking_named("leann-background-index", move || {
            integration
                .init_repository_blocking_with_cancel(&working_dir, cancel_for_blocking.as_ref())
        })
        .await;
        match result {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                tracing::warn!(error = %e, "LEANN background init failed; continuing without code context");
            }
            Err(e) if e.is_cancelled() => {
                tracing::info!("LEANN background init cancelled");
            }
            Err(e) => {
                tracing::warn!(error = %e, "LEANN background init join failed; continuing without code context");
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_default_session_does_not_build_the_repository_index() {
        assert!(!index_on_startup(
            &archon_core::config::ArchonConfig::default()
        ));
    }

    /// The knob is only worth having if the name in a config file reaches it —
    /// a serde rename or a missing `#[serde(default)]` on the section would
    /// leave `index_on_startup = true` silently inert.
    #[test]
    fn the_config_file_spelling_reaches_the_flag() {
        let config: archon_core::config::ArchonConfig =
            toml::from_str("[code_index]\nindex_on_startup = true\n").expect("parse config");
        assert!(index_on_startup(&config));
    }

    /// The expensive half must stay behind the gate.
    ///
    /// `begin` returns the index handle whether or not it builds, so a future
    /// edit that hoists `spawn_repository_build` out of the `if` would compile,
    /// pass every other test, and quietly restore ~17 CPU-hours of unrequested
    /// work at every session start. This pins the shape instead.
    #[test]
    fn the_repository_build_is_reachable_only_through_the_gate() {
        let source = include_str!("leann_startup.rs");
        let body = source
            .split_once("pub(super) fn begin(")
            .expect("begin definition")
            .1
            .split_once("pub(super) fn index_on_startup(")
            .expect("end of begin")
            .0;

        assert_eq!(
            body.matches("spawn_repository_build(").count(),
            1,
            "the repository build must be started from exactly one place"
        );
        let gate = body
            .find("if index_on_startup(config)")
            .expect("startup gate");
        let call = body
            .find("spawn_repository_build(")
            .expect("repository build call");
        assert!(
            gate < call,
            "the repository build must be started inside the config gate"
        );
    }
}

//! `[memory]` → the shape `archon-memory` needs in order to open the store.
//!
//! Lives here rather than in `archon-memory` because the dependency edge points
//! core → memory: `archon_memory::MemoryOpenSpec` cannot name `ArchonConfig`.
//! It is a separate file from `sections.rs` only because that file is near the
//! 500-line ceiling.

use archon_memory::MemoryOpenSpec;
use archon_memory::embedding::EmbeddingConfig;

use super::MemoryConfig;

impl MemoryConfig {
    /// The one place `[memory]` is translated for an open.
    ///
    /// Four entry points open memory (the TUI, `--print`/`--headless`, a
    /// standalone `archon workflow`, and `archon memory ...`). Before #146 each
    /// built its own [`EmbeddingConfig`] literal, and `intra_threads` had
    /// already been added to three of the four before the fourth caught up. A
    /// field added to `[memory]` now reaches every opener by being written once,
    /// here, or it reaches none of them and the compiler says so.
    ///
    /// # Why `enabled` is not consulted
    ///
    /// It looks like the obvious guard and it is the wrong one. Everywhere
    /// `memory.enabled` is read — `interactive_setup` (registering
    /// `MemoryStoreTool`/`MemoryRecallTool`), `interactive_agent`
    /// (`set_memory`, the auto-extractor), `session.rs` and `web_runtime`
    /// (auto-capture), `interactive_finish` (garden consolidation) — it gates a
    /// *model-facing* surface, and the startup log says exactly that: "memory
    /// tools + graph injection DISABLED". Nothing it gates is about whether the
    /// process may hold the database.
    ///
    /// Three things depend on that reading:
    ///
    /// - The TUI loads its behavioural rules through `RulesEngine::new(memory)`
    ///   with no `enabled` check at all, so a disabled memory that also refused
    ///   to open would silently drop the consciousness defaults.
    /// - The task board is not a memory tool. It is how a subagent reports a
    ///   finding and how the workflow drain gate establishes that a run left no
    ///   gaps. Gating the open on `enabled` would mean `enabled = false` blocks
    ///   every decomposed-PRD run at the drain gate — a run whose completion
    ///   cannot be checked is refused (`workflow_board_drain.rs`) — which is a
    ///   far larger consequence than "the model has no memory tools" and not
    ///   one anybody asked for.
    /// - The other openers in this binary (`learning/gnn.rs`,
    ///   `cognitive_daemon_learning.rs`) already ignore it, so honouring it in
    ///   only the four session paths would make `enabled = false` mean different
    ///   things in the same process.
    ///
    /// So opening is unconditional and deliberate: `enabled` withholds memory
    /// from the *model*, not the store from the *process*. Changing that is a
    /// config-semantics change and belongs in a release note, not in a
    /// refactor.
    pub fn open_spec(&self) -> MemoryOpenSpec {
        MemoryOpenSpec {
            db_path: self.db_path.clone(),
            embedding: EmbeddingConfig {
                provider: self.embedding_provider,
                hybrid_alpha: self.hybrid_alpha,
                base_url: self.embedding_base_url.clone(),
                model: self.embedding_model.clone(),
                intra_threads: self.embedding_intra_threads,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use archon_memory::embedding::EmbeddingProviderKind;

    fn configured() -> MemoryConfig {
        MemoryConfig {
            db_path: Some("D:/somewhere/else/board.db".into()),
            embedding_provider: EmbeddingProviderKind::OpenAI,
            embedding_base_url: Some("http://127.0.0.1:1234/v1".into()),
            embedding_model: Some("text-embedding-3-large".into()),
            hybrid_alpha: 0.75,
            embedding_intra_threads: Some(3),
            ..MemoryConfig::default()
        }
    }

    /// Every field, not a sample. A field that stops being carried is a session
    /// embedding differently from its neighbour against the same database, and
    /// nothing at runtime reports it — which is exactly how `intra_threads`
    /// reached three of the four openers and not the fourth.
    #[test]
    fn open_spec_carries_every_configured_embedding_field() {
        let spec = configured().open_spec();
        assert_eq!(spec.embedding.provider, EmbeddingProviderKind::OpenAI);
        assert_eq!(spec.embedding.hybrid_alpha, 0.75);
        assert_eq!(
            spec.embedding.base_url.as_deref(),
            Some("http://127.0.0.1:1234/v1")
        );
        assert_eq!(
            spec.embedding.model.as_deref(),
            Some("text-embedding-3-large")
        );
        assert_eq!(spec.embedding.intra_threads, Some(3));
    }

    /// The trap the board fix was written to avoid: an opener that resolves the
    /// default data dir instead of the configured file gets a private store that
    /// accepts writes nobody reads. Pinned here because every entry point now
    /// resolves through this one spec.
    #[test]
    fn open_spec_resolves_the_configured_database_not_the_default() {
        let (data_dir, db_path) = configured().open_spec().resolve_paths();
        assert_eq!(
            db_path,
            std::path::PathBuf::from("D:/somewhere/else/board.db")
        );
        assert_eq!(data_dir, std::path::PathBuf::from("D:/somewhere/else"));
    }
}

use std::collections::BTreeMap;

use cozo::DataValue;

use super::MemoryGraph;
use super::helpers::{empty_rows, run_mutable};
use crate::types::MemoryError;

impl MemoryGraph {
    // -- schema ------------------------------------------------

    pub(super) fn init_schema(&self) -> Result<(), MemoryError> {
        run_mutable(
            &self.db,
            ":create memories {
                    id: String
                    =>
                    content: String,
                    title: String,
                    memory_type: String,
                    importance: Float,
                    tags: String,
                    source_type: String,
                    project_path: String,
                    created_at: String,
                    updated_at: String,
                    access_count: Int,
                    last_accessed: String
                }",
            Default::default(),
            "memory schema: create memories relation",
        )
        .or_else(ignore_already_exists)?;

        run_mutable(
            &self.db,
            ":create relationships {
                    from_id: String,
                    to_id: String,
                    rel_type: String
                    =>
                    context: String,
                    strength: Float,
                    created_at: String
                }",
            Default::default(),
            "memory schema: create relationships relation",
        )
        .or_else(ignore_already_exists)?;

        run_mutable(
            &self.db,
            ":create score_applications {
                    memory_id: String,
                    provenance_id: String
                    =>
                    applied_at: String
                }",
            Default::default(),
            "memory schema: create score_applications relation",
        )
        .or_else(ignore_already_exists)?;

        self.init_board_schema()?;
        self.init_memory_fts()?;

        Ok(())
    }

    /// Create the agent task board relation and its run index.
    ///
    /// Real columns, not tags. `memories` could not carry this: its schema is
    /// fixed at twelve columns, so status, claim, and round would have to be
    /// tag-encoded against the sixteen non-trend tags that
    /// `crud_importance.rs` asserts in Datalog, and `update_memory` replaces
    /// the whole tag vector last-writer-wins with no compare-and-set. Two
    /// agents claiming one item would both believe they won.
    ///
    /// `claimed_by` is the only nullable column, and it is nullable rather than
    /// an empty-string sentinel because the claim CAS turns on `is_null` --
    /// "unclaimed" has to be a state the database can test, not a convention
    /// each query re-implements.
    ///
    /// Idempotent like every relation above it, so an existing store picks the
    /// board up on next open with no migration step.
    fn init_board_schema(&self) -> Result<(), MemoryError> {
        run_mutable(
            &self.db,
            ":create board_items {
                    id: String
                    =>
                    run_id: String,
                    kind: String,
                    status: String,
                    title: String,
                    evidence: String,
                    acceptance: String,
                    raised_by: String,
                    claimed_by: String?,
                    round: Int,
                    created_at: String,
                    updated_at: String
                }",
            Default::default(),
            "memory schema: create board_items relation",
        )
        .or_else(ignore_already_exists)?;

        // A board is polled: the drain gate reads it at every barrier and every
        // agent looking for work reads it again. Without this index a run-scoped
        // read is a full scan of every run's items, and the cost grows with the
        // whole board rather than with the caller's own run.
        run_mutable(
            &self.db,
            "::index create board_items:by_run {run_id}",
            Default::default(),
            "memory schema: create board_items:by_run index",
        )
        .or_else(ignore_already_exists)?;

        Ok(())
    }

    /// Create the memory FTS indexes, rebuilding any left on the old tokenizer.
    ///
    /// TOKENIZER: `NGram(2, 2)`, not `NGram(1, 2)`.
    ///
    /// The unigrams were the whole problem. Cozo applies the index tokenizer to
    /// the QUERY (`fts/indexing.rs`, `parse_fts_query(q)?.tokenize(tokenizer)`),
    /// so each search term expands into character n-grams -- and a one-character
    /// n-gram like `"e"` has a posting list containing essentially every row.
    /// `fts_search_literal` range-scans that list in full and `rmp_serde`-decodes
    /// every entry; `FtsExpr::And` evaluates every sub-literal without
    /// short-circuiting and `Or` every branch. `k` cannot save it either: it
    /// truncates the finished, sorted result set rather than pruning the search.
    ///
    /// So the cost was per-term posting-list length, which no caller-side bound
    /// could reach. Measured on a 1.7 GB store: 12.4 seconds to return zero
    /// corrections, and a full core for over a minute under a workflow prompt.
    ///
    /// Bigrams keep substring matching -- `"auth"` still finds
    /// `"authentication"` through `au`/`ut`/`th` -- and keep working for
    /// languages without whitespace word breaks, which `Simple` would not.
    /// `archon-docs` reached the same conclusion independently: its exact-match
    /// index is `NGram(2, 2)`, never starting at 1.
    fn init_memory_fts(&self) -> Result<(), MemoryError> {
        for (index, extractor, extract_filter, tokenizer) in [
            (
                "content_fts",
                "content",
                r#"content != """#,
                CONTENT_FTS_TOKENIZER,
            ),
            ("title_fts", "title", r#"title != """#, SHORT_FTS_TOKENIZER),
            ("tags_fts", "tags", r#"tags != "[]""#, SHORT_FTS_TOKENIZER),
        ] {
            let create = format!(
                r#"::fts create memories:{index} {{
                extractor: {extractor},
                extract_filter: {extract_filter},
                tokenizer: {tokenizer},
                filters: [Lowercase],
            }}"#
            );
            let context = format!("memory schema: create memories:{index} index");

            match run_mutable(&self.db, &create, Default::default(), &context) {
                Ok(_) => {}
                // Already present -- but possibly on the OLD tokenizer, and a
                // `::fts create` is a no-op that would silently leave it there.
                // The stored postings ARE the tokenization, so changing the
                // declaration cannot migrate them: the index has to be dropped
                // and rebuilt from the rows.
                Err(error) if already_exists(&error) => {
                    self.rebuild_memory_fts_index(index, &create, tokenizer)?;
                }
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }

    /// Drop and recreate one FTS index so it is rebuilt under the current
    /// tokenizer.
    ///
    /// Rebuilding costs a full re-index of the relation, so it is only done
    /// when the existing index does not already match. Cozo does not expose the
    /// tokenizer of an existing index, so "does not match" is inferred from a
    /// marker relation written after a successful rebuild -- absent marker means
    /// the index predates this migration.
    fn rebuild_memory_fts_index(
        &self,
        index: &str,
        create: &str,
        tokenizer: &str,
    ) -> Result<(), MemoryError> {
        if self.memory_fts_is_current(index, tokenizer)? {
            return Ok(());
        }
        tracing::info!(
            index,
            "rebuilding memory FTS index onto the current tokenizer; \
             this re-indexes the relation once and can take a while on a large store"
        );
        run_mutable(
            &self.db,
            &format!("::fts drop memories:{index}"),
            Default::default(),
            &format!("memory schema: drop stale memories:{index} index"),
        )
        .or_else(ignore_already_exists)?;
        run_mutable(
            &self.db,
            create,
            Default::default(),
            &format!("memory schema: recreate memories:{index} index"),
        )?;
        self.mark_memory_fts_current(index, tokenizer)?;
        tracing::info!(index, "memory FTS index rebuilt");
        Ok(())
    }

    /// Has `index` already been rebuilt under `tokenizer`?
    ///
    /// A missing marker relation, a missing row, or a different tokenizer all
    /// mean "no". Fail-safe direction: an unreadable marker reports stale, so
    /// the worst case is one unnecessary rebuild rather than silently keeping a
    /// slow index forever.
    fn memory_fts_is_current(&self, index: &str, tokenizer: &str) -> Result<bool, MemoryError> {
        run_mutable(
            &self.db,
            ":create memory_fts_state { index: String => tokenizer: String }",
            Default::default(),
            "memory schema: create memory_fts_state relation",
        )
        .or_else(ignore_already_exists)?;

        let mut params = BTreeMap::new();
        params.insert("idx".to_string(), DataValue::from(index));
        let rows = run_mutable(
            &self.db,
            "?[tokenizer] := *memory_fts_state{index: $idx, tokenizer}",
            params,
            "memory schema: read memory_fts_state",
        )?;
        Ok(rows
            .rows
            .first()
            .and_then(|row| row.first())
            .and_then(DataValue::get_str)
            == Some(tokenizer))
    }

    /// Record the tokenizer `index` was last rebuilt under.
    fn mark_memory_fts_current(&self, index: &str, tokenizer: &str) -> Result<(), MemoryError> {
        let mut params = BTreeMap::new();
        params.insert("idx".to_string(), DataValue::from(index));
        params.insert("tok".to_string(), DataValue::from(tokenizer));
        run_mutable(
            &self.db,
            "?[index, tokenizer] <- [[$idx, $tok]] :put memory_fts_state {index => tokenizer}",
            params,
            "memory schema: record memory_fts_state",
        )?;
        Ok(())
    }
}

/// `:create` and `::fts create` are run unconditionally at open; treat an
/// existing relation or index as success and surface everything else.
fn ignore_already_exists(error: MemoryError) -> Result<cozo::NamedRows, MemoryError> {
    if already_exists(&error) {
        Ok(empty_rows())
    } else {
        Err(error)
    }
}

/// True when a schema script failed only because the object is already there.
fn already_exists(error: &MemoryError) -> bool {
    let msg = error.to_string();
    msg.contains("already exists") || msg.contains("conflicts")
}

/// Tokenizer for `content_fts`, the large prose index.
///
/// BIGRAMS ONLY -- no unigrams. A one-character n-gram's posting list contains
/// essentially every row, and Cozo range-scans it in full, `rmp_serde`-decoding
/// every entry, for each such term. That was the dominant cost on a 1.7 GB
/// store. Dropping unigrams means a single-character search can no longer match
/// here, which is an acceptable trade for prose: one character is not a
/// meaningful full-text query over long content, and it was never worth a full
/// relation scan.
const CONTENT_FTS_TOKENIZER: &str = "NGram(2, 2, false)";

/// Tokenizer for `title_fts` and `tags_fts`, the short-value indexes.
///
/// UNIGRAMS KEPT, deliberately. A tag genuinely can be one character -- `x`,
/// `c`, `go` -- and `search_by_tags_any`/`search_by_tags_all` pin that. These
/// fields are also orders of magnitude smaller than content (a title is tens of
/// characters where content is thousands), so their posting lists are short,
/// and they are queried from deliberate tag/title filters rather than from a
/// 21 KB prompt. The cost that made this urgent simply is not here.
const SHORT_FTS_TOKENIZER: &str = "NGram(1, 2, false)";
